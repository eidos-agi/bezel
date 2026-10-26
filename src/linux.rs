//! Inject on Linux. This process does not own a physical keyboard.
//! The desktop portal has to allow pointer and keyboard control. Sharing a
//! monitor gives the stream id that absolute placement needs.

use crate::allow::peer_allowed;
use crate::desk::Edge;
use crate::frame::{Frame, VERSION};
use crate::hid::{self, linux_keycode};
use crate::inbox::Store;
use crate::inject::{Cursor, Effect};
use crate::net::{self, Link};
use crate::update;
use std::collections::HashMap;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use serde::Serialize;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{DynamicType, OwnedObjectPath, OwnedValue, Structure, Value};

const DEST: &str = "org.freedesktop.portal.Desktop";
const OBJECT: &str = "/org/freedesktop/portal/desktop";
const REMOTE: &str = "org.freedesktop.portal.RemoteDesktop";
const CAST: &str = "org.freedesktop.portal.ScreenCast";
const KEYBOARD: u32 = 1;
const POINTER: u32 = 2;

struct Monitor {
    node: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

struct Desk {
    conn: Connection,
    session: OwnedObjectPath,
    monitors: Vec<Monitor>,
    width: f64,
    height: f64,
}

pub fn listen(port: u16, allow_name: &str, upgrade: bool, settings: &Path) -> io::Result<()> {
    if upgrade {
        crate::upgrade::watch();
    }
    let desk = open_desk()?;
    let (width, height) = (desk.width, desk.height);
    eprintln!(
        "bezel: screen {}x{}",
        width.round() as u32,
        height.round() as u32
    );
    let screens: Vec<crate::status::Rect> = desk
        .monitors
        .iter()
        .map(|mon| crate::status::Rect {
            x: mon.x.round() as i32,
            y: mon.y.round() as i32,
            w: mon.w.round() as i32,
            h: mon.h.round() as i32,
        })
        .collect();
    crate::status::open(settings, "listen", "left", &screens);
    let ip = tailscale_ipv4()?;
    let listener = bind_listener(ip, port)?;
    eprintln!("bezel: listening on {ip}:{port} for {allow_name}");
    loop {
        let (stream, peer) = listener.accept()?;
        let allowed = net::resolve(allow_name, 0).unwrap_or_default();
        let allowed_ips: Vec<IpAddr> = allowed.into_iter().map(|addr| addr.ip()).collect();
        if !peer_allowed(peer.ip(), &allowed_ips) {
            eprintln!("bezel: refused {}", peer.ip());
            crate::report::submit(
                "peer-refused",
                "A connection was refused because it was not the allowed Windows machine.",
            );
            continue;
        }
        eprintln!("bezel: connected {}", peer.ip());
        crate::status::link("connected");
        if let Err(err) = dispatch(&desk, stream, settings, width, height) {
            eprintln!("bezel: disconnected ({err})");
            crate::status::link("listening");
            crate::status::observe("disconnected");
            crate::report::submit(
                "session-dropped",
                &format!("The connection closed ({err})."),
            );
        }
    }
}

fn bind_listener(ip: Ipv4Addr, port: u16) -> io::Result<TcpListener> {
    let tries = if std::env::var_os("BEZEL_RESTART").is_some() {
        50
    } else {
        1
    };
    let mut last = None;
    for _ in 0..tries {
        match TcpListener::bind((ip, port)) {
            Ok(listener) => return Ok(listener),
            Err(err) => {
                last = Some(err);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(ErrorKind::AddrInUse, "bind")))
}

fn dispatch(desk: &Desk, mut stream: TcpStream, settings: &Path, width: f64, height: f64) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    if is_update(&stream)? {
        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        return accept_update(&mut stream);
    }
    let _ = stream.set_read_timeout(None);
    session(desk, stream, settings, width, height)
}

fn is_update(stream: &TcpStream) -> io::Result<bool> {
    let mut buf = [0u8; 4];
    let n = match stream.peek(&mut buf) {
        Ok(n) => n,
        Err(err) if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut => {
            return Ok(false);
        }
        Err(err) if err.kind() == ErrorKind::Interrupted => return is_update(stream),
        Err(err) => return Err(err),
    };
    if n < 4 {
        thread::sleep(Duration::from_millis(20));
        match stream.peek(&mut buf) {
            Ok(n) if n >= 4 => {}
            _ => return Ok(false),
        }
    }
    Ok(&buf == update::MAGIC)
}

fn accept_update(stream: &mut TcpStream) -> io::Result<()> {
    let root = update::source_root();
    fs::create_dir_all(&root)?;
    match update::take(stream, &root) {
        Ok(()) => {
            let _ = stream.write_all(b"ok\n");
            let _ = stream.flush();
            eprintln!("bezel: updated");
            update::restart(&root)
        }
        Err(err) => {
            let msg = format!("err {err}\n");
            let _ = stream.write_all(msg.as_bytes());
            let _ = stream.flush();
            eprintln!("bezel: update failed ({err})");
            Err(err)
        }
    }
}

fn session(
    desk: &Desk,
    stream: TcpStream,
    settings: &Path,
    width: f64,
    height: f64,
) -> io::Result<()> {
    let mut link = Link::new(stream)?;
    let mut cursor = Cursor::new(width, height, Edge::Left);
    let mut last_beat = Instant::now();
    let mut inbox_at = Instant::now();
    let mut greeted = false;
    let mut told_version = false;
    let mut inbox = Store::attach(crate::inbox::path_for(settings));
    let scale = 1000u16;
    loop {
        match link.recv() {
            Ok(frames) => {
                if frames.is_empty() {
                    thread::sleep(Duration::from_millis(4));
                }
                for frame in frames {
                    match frame {
                        Frame::Hello { version } => {
                            if version != VERSION {
                                return Err(io::Error::new(ErrorKind::InvalidData, "version"));
                            }
                            link.send(&Frame::HelloAck {
                                logical_w: width.round().clamp(1.0, u16::MAX as f64) as u16,
                                logical_h: height.round().clamp(1.0, u16::MAX as f64) as u16,
                                scale_milli: scale,
                                accepts_config: true,
                                accepts_inbox: true,
                                accepts_version: true,
                            })?;
                            greeted = true;
                            crate::status::link("connected");
                        }
                        Frame::Heartbeat => {}
                        Frame::PeerRelease { version } if greeted => {
                            crate::status::peer(&version);
                            if !told_version {
                                told_version = true;
                                link.send(&Frame::PeerRelease {
                                    version: crate::upgrade::current_version().to_string(),
                                })?;
                            }
                        }
                        Frame::Config { edge } if greeted => {
                            cursor.set_edge(edge);
                            crate::status::edge(edge.as_str());
                            eprintln!("bezel: edge {}", edge.as_str());
                        }
                        frame @ (Frame::InboxHello { .. } | Frame::Note { .. }) if greeted => {
                            if !inbox.has_said() {
                                let hello = inbox.greet();
                                link.send(&hello)?;
                            }
                            for reply in inbox.observe(frame)? {
                                link.send(&reply)?;
                            }
                        }
                        frame if greeted => {
                            for effect in cursor.apply(frame) {
                                perform(desk, &effect)?;
                                if let Effect::Send(reply) = effect {
                                    link.send(&reply)?;
                                    if matches!(reply, Frame::Leave) {
                                        crate::status::observe("left");
                                        eprintln!("bezel: left");
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(err) => {
                for effect in cursor.release_all() {
                    let _ = perform(desk, &effect);
                }
                eprintln!("bezel: disconnected");
                return Err(err);
            }
        }
        if last_beat.elapsed() >= Duration::from_millis(200) {
            link.send(&Frame::Heartbeat)?;
            last_beat = Instant::now();
        }
        if greeted && inbox_at.elapsed() >= Duration::from_millis(500) {
            inbox_at = Instant::now();
            if let Ok(frames) = inbox.poll() {
                for frame in frames {
                    link.send(&frame)?;
                }
            }
        }
        link.flush()?;
    }
}

fn perform(desk: &Desk, effect: &Effect) -> io::Result<()> {
    match effect {
        Effect::Place { x, y } => desk.place(*x, *y),
        Effect::Button {
            button, down, x, y, ..
        } => {
            desk.place(*x, *y)?;
            desk.button(*button, *down)
        }
        Effect::Wheel { dx, dy } => desk.wheel(*dx, *dy),
        Effect::Key { hid, down } => {
            let Some(code) = linux_keycode(*hid) else {
                if crate::trace::on() {
                    crate::trace::key(&format!(
                        "{} {} not-posted unmapped",
                        hid::label(*hid),
                        crate::trace::dir(*down)
                    ));
                }
                return Ok(());
            };
            desk.key(code, *down)?;
            if crate::trace::on() {
                crate::trace::key(&format!(
                    "{} {} posted",
                    hid::label(*hid),
                    crate::trace::dir(*down)
                ));
            }
            Ok(())
        }
        Effect::Send(_) => Ok(()),
    }
}

fn open_desk() -> io::Result<Desk> {
    let conn = Connection::session().map_err(portal_err)?;
    let unique = bus_name(&conn)?;
    let session_token = fresh("session");
    let create_token = fresh("create");
    let mut options = HashMap::new();
    options.insert("handle_token", Value::from(create_token.clone()));
    options.insert("session_handle_token", Value::from(session_token.clone()));
    let created = portal(&conn, REMOTE, "CreateSession", &(options,), &create_token)?;
    let session = object_path(&created, "session_handle").or_else(|err| {
        eprintln!("bezel: {err}");
        OwnedObjectPath::try_from(bus_path(&unique, "session", &session_token))
            .map_err(|path_err| io::Error::new(ErrorKind::InvalidData, path_err.to_string()))
    })?;

    let select_token = fresh("select");
    let mut options = HashMap::new();
    options.insert("handle_token", Value::from(select_token.clone()));
    options.insert("types", Value::U32(KEYBOARD | POINTER));
    options.insert("persist_mode", Value::U32(2));
    if let Some(saved) = read_token() {
        options.insert("restore_token", Value::from(saved));
    }
    portal(
        &conn,
        REMOTE,
        "SelectDevices",
        &(session.clone(), options),
        &select_token,
    )?;

    let sources_token = fresh("sources");
    let mut options = HashMap::new();
    options.insert("handle_token", Value::from(sources_token.clone()));
    options.insert("types", Value::U32(1));
    options.insert("multiple", Value::Bool(true));
    // 1 is a hidden cursor. The stream is only used to place the pointer.
    options.insert("cursor_mode", Value::U32(1));
    portal(
        &conn,
        CAST,
        "SelectSources",
        &(session.clone(), options),
        &sources_token,
    )?;

    eprintln!("bezel: allow keyboard, pointer, and a monitor on the desktop dialog");
    let start_token = fresh("start");
    let mut options = HashMap::new();
    options.insert("handle_token", Value::from(start_token.clone()));
    let started = portal(
        &conn,
        REMOTE,
        "Start",
        &(session.clone(), "", options),
        &start_token,
    )?;
    if let Some(token) = text_of(started.get("restore_token")) {
        let _ = write_token(&token);
    }
    let devices = u32_of(started.get("devices")).unwrap_or(0);
    if devices & POINTER == 0 {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "the desktop did not allow the pointer",
        ));
    }
    let monitors = match started.get("streams") {
        Some(value) => monitors_of(value)?,
        None => Vec::new(),
    };
    if monitors.is_empty() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            "the desktop did not share a monitor",
        ));
    }
    let (width, height) = bounds(&monitors);
    Ok(Desk {
        conn,
        session,
        monitors,
        width,
        height,
    })
}

fn portal(
    conn: &Connection,
    interface: &str,
    method: &str,
    body: &(impl Serialize + DynamicType),
    token: &str,
) -> io::Result<HashMap<String, OwnedValue>> {
    let unique = bus_name(conn)?;
    let path = bus_path(&unique, "request", token);
    let watcher = Proxy::new(conn, DEST, path.as_str(), "org.freedesktop.portal.Request")
        .map_err(portal_err)?;
    let mut signals = watcher.receive_signal("Response").map_err(portal_err)?;
    let proxy = Proxy::new(conn, DEST, OBJECT, interface).map_err(portal_err)?;
    let returned: OwnedObjectPath = proxy.call(method, body).map_err(portal_err)?;
    if returned.as_str() != path {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("portal request path {returned} was not {path}"),
        ));
    }
    let message = signals.next().ok_or_else(|| {
        io::Error::new(ErrorKind::UnexpectedEof, "the desktop closed the request")
    })?;
    let (code, results): (u32, HashMap<String, OwnedValue>) =
        message.body().deserialize().map_err(portal_err)?;
    if code != 0 {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!("the desktop declined {method} ({code})"),
        ));
    }
    Ok(results)
}

fn bus_name(conn: &Connection) -> io::Result<String> {
    conn.unique_name()
        .map(|name| name.to_string())
        .ok_or_else(|| io::Error::new(ErrorKind::NotConnected, "no bus name"))
}

fn bus_path(unique: &str, kind: &str, token: &str) -> String {
    let sender = unique.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/{kind}/{sender}/{token}")
}

fn fresh(kind: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("bezel{kind}{n}{}", std::process::id())
}

impl Desk {
    fn place(&self, x: f64, y: f64) -> io::Result<()> {
        let (node, x, y) = local_on(&self.monitors, x, y);
        self.call(
            "NotifyPointerMotionAbsolute",
            &(self.session.clone(), empty(), node, x, y),
        )
    }

    fn button(&self, button: u8, down: bool) -> io::Result<()> {
        let code: i32 = match button {
            0 => 0x110,
            1 => 0x111,
            _ => 0x112,
        };
        let state: u32 = if down { 1 } else { 0 };
        self.call(
            "NotifyPointerButton",
            &(self.session.clone(), empty(), code, state),
        )
    }

    fn wheel(&self, dx: i16, dy: i16) -> io::Result<()> {
        let (horizontal, vertical) = scroll_steps(dx, dy);
        if vertical != 0 {
            self.call(
                "NotifyPointerAxisDiscrete",
                &(self.session.clone(), empty(), 0u32, vertical),
            )?;
        }
        if horizontal != 0 {
            self.call(
                "NotifyPointerAxisDiscrete",
                &(self.session.clone(), empty(), 1u32, horizontal),
            )?;
        }
        Ok(())
    }

    fn key(&self, code: u16, down: bool) -> io::Result<()> {
        let state: u32 = if down { 1 } else { 0 };
        self.call(
            "NotifyKeyboardKeycode",
            &(self.session.clone(), empty(), i32::from(code), state),
        )
    }

    fn call(&self, method: &str, body: &(impl Serialize + DynamicType)) -> io::Result<()> {
        let proxy = Proxy::new(&self.conn, DEST, OBJECT, REMOTE).map_err(portal_err)?;
        proxy.call::<_, _, ()>(method, body).map_err(portal_err)
    }
}

fn empty() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

/// Mutter treats a positive vertical step as down. Windows reports up as positive.
fn scroll_steps(dx: i16, dy: i16) -> (i32, i32) {
    (i32::from(dx), -i32::from(dy))
}

fn local_on(monitors: &[Monitor], x: f64, y: f64) -> (u32, f64, f64) {
    let monitor = monitors
        .iter()
        .find(|monitor| {
            x >= monitor.x && y >= monitor.y && x < monitor.x + monitor.w && y < monitor.y + monitor.h
        })
        .unwrap_or(&monitors[0]);
    let x = (x - monitor.x).clamp(0.0, (monitor.w - 1.0).max(0.0));
    let y = (y - monitor.y).clamp(0.0, (monitor.h - 1.0).max(0.0));
    (monitor.node, x, y)
}

fn bounds(monitors: &[Monitor]) -> (f64, f64) {
    let mut width: f64 = 1.0;
    let mut height: f64 = 1.0;
    for monitor in monitors {
        width = width.max(monitor.x + monitor.w);
        height = height.max(monitor.y + monitor.h);
    }
    (width, height)
}

fn monitors_of(value: &OwnedValue) -> io::Result<Vec<Monitor>> {
    let owned = value
        .try_clone()
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err.to_string()))?;
    let value: Value = owned.into();
    let rows: Vec<(u32, HashMap<String, OwnedValue>)> = value.try_into().map_err(|err| {
        io::Error::new(ErrorKind::InvalidData, format!("streams: {err}"))
    })?;
    let mut monitors = Vec::new();
    for (node, props) in rows {
        let Some((w, h)) = props.get("size").and_then(pair_of) else {
            continue;
        };
        if w < 1.0 || h < 1.0 {
            continue;
        }
        let (x, y) = props.get("position").and_then(pair_of).unwrap_or((0.0, 0.0));
        monitors.push(Monitor { node, x, y, w, h });
    }
    Ok(monitors)
}

fn pair_of(value: &OwnedValue) -> Option<(f64, f64)> {
    let owned = value.try_clone().ok()?;
    let value: Value = owned.into();
    let structure: Structure = value.try_into().ok()?;
    let fields = structure.fields();
    if fields.len() != 2 {
        return None;
    }
    Some((number(&fields[0])?, number(&fields[1])?))
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::I16(n) => Some(f64::from(*n)),
        Value::I32(n) => Some(f64::from(*n)),
        Value::I64(n) => Some(*n as f64),
        Value::U16(n) => Some(f64::from(*n)),
        Value::U32(n) => Some(f64::from(*n)),
        Value::U64(n) => Some(*n as f64),
        Value::F64(n) => Some(*n),
        _ => None,
    }
}

fn object_path(map: &HashMap<String, OwnedValue>, key: &str) -> io::Result<OwnedObjectPath> {
    let value = map.get(key).ok_or_else(|| {
        io::Error::new(ErrorKind::NotFound, format!("portal response has no {key}"))
    })?;
    let owned = value
        .try_clone()
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err.to_string()))?;
    if let Ok(path) = OwnedObjectPath::try_from(owned.try_clone().map_err(|err| {
        io::Error::new(ErrorKind::InvalidData, err.to_string())
    })?) {
        return Ok(path);
    }
    if let Ok(text) = String::try_from(owned) {
        return OwnedObjectPath::try_from(text).map_err(|err| {
            io::Error::new(ErrorKind::InvalidData, format!("{key}: {err}"))
        });
    }
    Err(io::Error::new(
        ErrorKind::InvalidData,
        format!("{key}: {value:?}"),
    ))
}

fn text_of(value: Option<&OwnedValue>) -> Option<String> {
    let owned = value?.try_clone().ok()?;
    String::try_from(owned).ok()
}

fn u32_of(value: Option<&OwnedValue>) -> Option<u32> {
    let owned = value?.try_clone().ok()?;
    u32::try_from(owned).ok()
}

fn token_path() -> std::path::PathBuf {
    crate::settings::default_path().with_file_name("portal.token")
}

fn read_token() -> Option<String> {
    let text = fs::read_to_string(token_path()).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn write_token(token: &str) -> io::Result<()> {
    let path = token_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    write!(file, "{token}")
}

fn portal_err(err: impl ToString) -> io::Error {
    io::Error::new(ErrorKind::Other, err.to_string())
}

fn tailscale_ipv4() -> io::Result<Ipv4Addr> {
    for bin in ["tailscale", "/usr/bin/tailscale"] {
        let Ok(output) = Command::new(bin).args(["ip", "-4"]).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(ip) = text.split_whitespace().find_map(|word| word.parse().ok()) {
            return Ok(ip);
        }
    }
    Err(io::Error::new(
        ErrorKind::NotFound,
        "tailscale ip -4 returned no address",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(node: u32, x: f64, y: f64, w: f64, h: f64) -> Monitor {
        Monitor { node, x, y, w, h }
    }

    #[test]
    fn a_point_on_the_second_screen_is_local_to_that_stream() {
        let monitors = vec![monitor(1, 0.0, 0.0, 1920.0, 1080.0), monitor(2, 1920.0, 0.0, 800.0, 600.0)];
        assert_eq!(bounds(&monitors), (2720.0, 1080.0));
        assert_eq!(local_on(&monitors, 2000.0, 10.0), (2, 80.0, 10.0));
        assert_eq!(local_on(&monitors, 10.0, 10.0), (1, 10.0, 10.0));
    }

    #[test]
    fn vertical_scroll_is_flipped_for_mutter() {
        assert_eq!(scroll_steps(0, 1), (0, -1));
        assert_eq!(scroll_steps(-2, -3), (-2, 3));
    }
}
