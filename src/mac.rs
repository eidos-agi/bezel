//! Inject on the Mac. This process does not own a physical keyboard.
//! Accessibility has to be granted to the process that launches `bezel listen`.

use crate::allow::peer_allowed;
use crate::desk::Edge;
use crate::frame::{Frame, VERSION};
use crate::hid::{self, mac_keycode};
use crate::inject::{Cursor, Effect};
use crate::net::{self, Link};
use crate::update;
use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use std::collections::BTreeSet;
use std::io::{self, ErrorKind, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

pub fn listen(port: u16, allow_name: &str, upgrade: bool) -> io::Result<()> {
    if unsafe { AXIsProcessTrusted() == 0 } {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "Accessibility permission is missing for this process. Grant it in System Settings, then start bezel listen again.",
        ));
    }
    if upgrade {
        crate::upgrade::watch();
    }
    let ip = tailscale_ipv4()?;
    let listener = bind_listener(ip, port)?;
    eprintln!("bezel: listening on {ip}:{port} for {allow_name}");
    loop {
        let (stream, peer) = listener.accept()?;
        let allowed = net::resolve(allow_name, 0).unwrap_or_default();
        let allowed_ips: Vec<IpAddr> = allowed.into_iter().map(|addr| addr.ip()).collect();
        if !peer_allowed(peer.ip(), &allowed_ips) {
            eprintln!("bezel: refused {}", peer.ip());
            continue;
        }
        eprintln!("bezel: connected {}", peer.ip());
        if let Err(err) = dispatch(stream) {
            eprintln!("bezel: disconnected ({err})");
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

fn dispatch(mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    if is_update(&stream)? {
        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        return accept_update(&mut stream);
    }
    let _ = stream.set_read_timeout(None);
    session(stream)
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
    std::fs::create_dir_all(&root)?;
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

fn session(stream: TcpStream) -> io::Result<()> {
    let mut link = Link::new(stream)?;
    let (width, height, scale) = display();
    // Windows sends the edge. This value holds only until that frame arrives.
    let mut cursor = Cursor::new(width, height, Edge::Left);
    let mut held_keys = BTreeSet::new();
    let mut held_buttons = BTreeSet::new();
    let mut last_beat = Instant::now();
    let mut greeted = false;
    loop {
        match link.recv() {
            Ok(frames) => {
                if frames.is_empty() {
                    thread::sleep(Duration::from_millis(4));
                }
                for frame in frames {
                    if let Frame::Hello { version } = frame {
                        if version != VERSION {
                            return Err(io::Error::new(ErrorKind::InvalidData, "version"));
                        }
                        link.send(&Frame::HelloAck {
                            logical_w: width.round().clamp(1.0, u16::MAX as f64) as u16,
                            logical_h: height.round().clamp(1.0, u16::MAX as f64) as u16,
                            scale_milli: scale,
                            accepts_config: true,
                        })?;
                        greeted = true;
                        continue;
                    }
                    if !greeted {
                        continue;
                    }
                    if matches!(frame, Frame::Heartbeat) {
                        continue;
                    }
                    if let Frame::Config { edge } = frame {
                        cursor.set_edge(edge);
                        eprintln!("bezel: edge {}", edge.as_str());
                        continue;
                    }
                    for effect in cursor.apply(frame) {
                        perform(&mut held_keys, &mut held_buttons, &effect)?;
                        if let Effect::Send(reply) = effect {
                            link.send(&reply)?;
                            if matches!(reply, Frame::Leave) {
                                eprintln!("bezel: left");
                            }
                        }
                    }
                }
            }
            Err(err) => {
                let ups = cursor.release_all();
                for effect in &ups {
                    let _ = perform(&mut held_keys, &mut held_buttons, effect);
                }
                eprintln!("bezel: disconnected");
                return Err(err);
            }
        }
        if last_beat.elapsed() >= Duration::from_millis(200) {
            link.send(&Frame::Heartbeat)?;
            last_beat = Instant::now();
        }
        link.flush()?;
    }
}

fn perform(
    held_keys: &mut BTreeSet<u16>,
    held_buttons: &mut BTreeSet<u8>,
    effect: &Effect,
) -> io::Result<()> {
    match *effect {
        Effect::Place { x, y } => {
            post_move(x, y, held_buttons, flags(held_keys))?;
        }
        Effect::Button {
            button,
            down,
            x,
            y,
        } => {
            if down {
                held_buttons.insert(button);
            } else {
                held_buttons.remove(&button);
            }
            post_button(button, down, x, y, flags(held_keys))?;
        }
        Effect::Wheel { dx, dy } => {
            let event = CGEvent::new_scroll_event(
                event_source()?,
                1,
                2,
                i32::from(dy),
                i32::from(dx),
                0,
            )
                .map_err(|_| io::Error::new(ErrorKind::Other, "scroll"))?;
            event.set_flags(flags(held_keys));
            event.post(CGEventTapLocation::HID);
        }
        Effect::Key { hid, down } => {
            if down {
                held_keys.insert(hid);
            } else {
                held_keys.remove(&hid);
            }
            let Some(code) = mac_keycode(hid) else {
                return Ok(());
            };
            let event = CGEvent::new_keyboard_event(event_source()?, code, down)
                .map_err(|_| io::Error::new(ErrorKind::Other, "key"))?;
            if hid::is_modifier(hid) {
                event.set_type(CGEventType::FlagsChanged);
            }
            event.set_flags(flags(held_keys));
            event.post(CGEventTapLocation::HID);
        }
        Effect::Send(_) => {}
    }
    Ok(())
}

fn post_move(
    x: f64,
    y: f64,
    buttons: &BTreeSet<u8>,
    flags: CGEventFlags,
) -> io::Result<()> {
    let (kind, button) = drag_kind(buttons);
    let event = CGEvent::new_mouse_event(
        event_source()?,
        kind,
        CGPoint { x, y },
        button,
    )
    .map_err(|_| io::Error::new(ErrorKind::Other, "move"))?;
    event.set_flags(flags);
    event.post(CGEventTapLocation::HID);
    Ok(())
}

fn post_button(
    button: u8,
    down: bool,
    x: f64,
    y: f64,
    flags: CGEventFlags,
) -> io::Result<()> {
    let mouse_button = mouse_button(button);
    let kind = match (button, down) {
        (0, true) => CGEventType::LeftMouseDown,
        (0, false) => CGEventType::LeftMouseUp,
        (1, true) => CGEventType::RightMouseDown,
        (1, false) => CGEventType::RightMouseUp,
        (_, true) => CGEventType::OtherMouseDown,
        (_, false) => CGEventType::OtherMouseUp,
    };
    let event = CGEvent::new_mouse_event(
        event_source()?,
        kind,
        CGPoint { x, y },
        mouse_button,
    )
    .map_err(|_| io::Error::new(ErrorKind::Other, "button"))?;
    event.set_flags(flags);
    event.post(CGEventTapLocation::HID);
    Ok(())
}

fn drag_kind(buttons: &BTreeSet<u8>) -> (CGEventType, CGMouseButton) {
    if buttons.contains(&0) {
        (CGEventType::LeftMouseDragged, CGMouseButton::Left)
    } else if buttons.contains(&1) {
        (CGEventType::RightMouseDragged, CGMouseButton::Right)
    } else if !buttons.is_empty() {
        (CGEventType::OtherMouseDragged, CGMouseButton::Center)
    } else {
        (CGEventType::MouseMoved, CGMouseButton::Left)
    }
}

fn mouse_button(button: u8) -> CGMouseButton {
    match button {
        1 => CGMouseButton::Right,
        2 => CGMouseButton::Center,
        _ => CGMouseButton::Left,
    }
}

fn flags(held: &BTreeSet<u16>) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    if held.iter().copied().any(hid::is_ctrl) {
        flags |= CGEventFlags::CGEventFlagCommand;
    }
    if held.iter().copied().any(hid::is_shift) {
        flags |= CGEventFlags::CGEventFlagShift;
    }
    if held.iter().copied().any(hid::is_alt) {
        flags |= CGEventFlags::CGEventFlagAlternate;
    }
    flags
}

fn event_source() -> io::Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| io::Error::new(ErrorKind::Other, "event source"))
}

fn display() -> (f64, f64, u16) {
    let display = CGDisplay::main();
    let bounds = display.bounds();
    let width = bounds.size.width.max(1.0);
    let height = bounds.size.height.max(1.0);
    let scale = ((display.pixels_wide() as f64 / width) * 1000.0).round() as u16;
    (width, height, scale.max(1))
}

fn tailscale_ipv4() -> io::Result<Ipv4Addr> {
    let bins = [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
    ];
    for bin in bins {
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


