//! A read-only checkup. It names who is connected and whether a move off the
//! chosen edge could hand the pointer over. It does not open the connection
//! and it does not move the pointer.

use crate::desk::{Desk, Edge, Mon};
use crate::inbox::{self, Note};
use crate::net;
use crate::settings::Settings;
use crate::who;
#[cfg_attr(target_os = "linux", allow(unused_imports))]
use std::io::{self, ErrorKind};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Up,
    Down,
    Unread,
}

pub struct Facts {
    pub release: String,
    pub here: String,
    pub peer: String,
    pub peer_host: String,
    pub link: Link,
    pub unresolved: bool,
    pub owns_pointer: bool,
    pub edge: Edge,
    pub desk: Option<Desk>,
    pub path: String,
    pub host: String,
    pub port: u16,
    pub scale: f64,
    pub allow: String,
}

pub fn checkup(settings: &Settings, path: &Path) -> String {
    let here = who::name();
    let peer_host = peer_host_of(settings).to_string();
    let peer = named_peer(path, &here).unwrap_or_else(|| peer_host.clone());
    let (link, unresolved) = observe(&peer_host, settings.port);
    render(&Facts {
        release: crate::upgrade::current_version().to_string(),
        here,
        peer,
        peer_host,
        link,
        unresolved,
        owns_pointer: cfg!(windows),
        edge: settings.edge,
        desk: live_desk(),
        path: path.display().to_string(),
        host: settings.host.clone(),
        port: settings.port,
        scale: settings.scale,
        allow: settings.allow.clone(),
    })
}

pub fn render(facts: &Facts) -> String {
    let mut lines = Vec::new();
    lines.push(format!("release {}", facts.release));
    lines.push(format!("{} is here.", facts.here));
    lines.push(presence(facts));
    if facts.unresolved {
        lines.push(format!("{} does not resolve.", facts.peer_host));
    }
    let (could, because) = verdict(facts);
    lines.push(format!("could move: {could}"));
    lines.extend(because);
    if facts.owns_pointer {
        if let Some(desk) = &facts.desk {
            lines.push("screens now".to_string());
            lines.extend(screen_lines(desk, facts.edge));
            lines.push(
                "A copy that is already running keeps the screens it had when it started."
                    .to_string(),
            );
        }
    }
    lines.push(format!("settings {}", facts.path));
    lines.push(format!("host {}", facts.host));
    lines.push(format!("port {}", facts.port));
    lines.push(format!("edge {}", facts.edge.as_str()));
    lines.push(format!("scale {}", scale_text(facts.scale)));
    lines.push(format!("allow {}", facts.allow));
    lines.join("\n")
}

/// The other machine's name, from notes that are still live.
/// The note text is ignored.
/// Live messages. Deleted rows stay in the file and stay out of this list.
pub fn message_lines(notes: &[Note]) -> String {
    let visible: Vec<&Note> = notes.iter().filter(|note| !note.deleted).collect();
    if visible.is_empty() {
        return "messages: none".to_string();
    }
    let mut lines = Vec::with_capacity(visible.len() + 1);
    lines.push("messages".to_string());
    for note in visible {
        if note.to.is_empty() {
            lines.push(format!("{} {} {}", note.id, note.from, note.text));
        } else {
            lines.push(format!(
                "{} {} -> {} {}",
                note.id, note.from, note.to, note.text
            ));
        }
    }
    lines.join("\n")
}

pub fn read_messages(settings: &Path) -> io::Result<String> {
    let store = inbox::Store::open(inbox::path_for(settings))?;
    Ok(message_lines(store.notes()))
}

pub fn add_message(settings: &Path, text: &str) -> io::Result<Note> {
    let mut store = inbox::Store::open(inbox::path_for(settings))?;
    address(&mut store);
    store.add(text)
}

pub fn update_message(settings: &Path, id: u32, text: &str) -> io::Result<()> {
    let mut store = inbox::Store::open(inbox::path_for(settings))?;
    address(&mut store);
    store.update(id, text)
}

pub fn delete_message(settings: &Path, id: u32) -> io::Result<()> {
    let mut store = inbox::Store::open(inbox::path_for(settings))?;
    address(&mut store);
    store.delete(id)
}

fn address(store: &mut inbox::Store) {
    let here = who::name();
    if let Some(peer) = other_name(store.notes(), &here) {
        store.set_peer(&peer);
    }
}

pub fn other_name(notes: &[Note], here: &str) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    for note in notes.iter().filter(|note| !note.deleted) {
        let Some(name) = mention(note, here) else {
            continue;
        };
        let replace = best
            .as_ref()
            .map(|(rev, _)| note.rev >= *rev)
            .unwrap_or(true);
        if replace {
            best = Some((note.rev, name));
        }
    }
    best.map(|(_, name)| name)
}

pub fn peer_up(sessions: &[Session], addrs: &[SocketAddr], port: u16) -> bool {
    let peers: Vec<Ipv4Addr> = addrs
        .iter()
        .filter_map(|addr| match addr.ip() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
        .collect();
    sessions.iter().any(|session| {
        peers.iter().any(|ip| {
            session.remote_ip == *ip && (session.remote_port == port || session.local_port == port)
        })
    })
}

pub fn parse_proc_tcp(text: &str) -> Vec<Session> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 && line.contains("local_address") {
            continue;
        }
        let mut cols = line.split_whitespace();
        let Some(_slot) = cols.next() else {
            continue;
        };
        let Some(local) = cols.next() else {
            continue;
        };
        let Some(remote) = cols.next() else {
            continue;
        };
        let Some(state) = cols.next() else {
            continue;
        };
        if state != "01" {
            continue;
        }
        let (Some((local_ip, local_port)), Some((remote_ip, remote_port))) = (
            parse_proc_endpoint(local),
            parse_proc_endpoint(remote),
        ) else {
            continue;
        };
        out.push(Session {
            local_ip,
            local_port,
            remote_ip,
            remote_port,
        });
    }
    out
}

pub fn parse_netstat(text: &str) -> Vec<Session> {
    let mut out = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 || cols.last().copied() != Some("ESTABLISHED") {
            continue;
        }
        let local = cols[cols.len() - 3];
        let foreign = cols[cols.len() - 2];
        let (Some((local_ip, local_port)), Some((remote_ip, remote_port))) =
            (split_v4(local), split_v4(foreign))
        else {
            continue;
        };
        out.push(Session {
            local_ip,
            local_port,
            remote_ip,
            remote_port,
        });
    }
    out
}

fn presence(facts: &Facts) -> String {
    match facts.link {
        Link::Up => format!("{} is connected.", facts.peer),
        Link::Down => format!("{} is not connected.", facts.peer),
        Link::Unread => format!("The connection to {} was not read.", facts.peer),
    }
}

fn verdict(facts: &Facts) -> (&'static str, Vec<String>) {
    let mut lines = Vec::new();
    let could = if !facts.owns_pointer {
        match facts.link {
            Link::Down => {
                lines.push(format!(
                    "{} is not connected, so the pointer stays on Windows.",
                    facts.peer
                ));
                "no"
            }
            Link::Unread => {
                lines.push("The connection table could not be read.".to_string());
                "unknown"
            }
            Link::Up => {
                lines.push(
                    "The link is up. A move is decided by the screens on Windows. This machine only injects."
                        .to_string(),
                );
                "unknown"
            }
        }
    } else if facts.desk.is_none() {
        lines.push(format!(
            "No screens were found, so the pointer stays on {}.",
            facts.here
        ));
        "no"
    } else if facts.link == Link::Unread {
        lines.push("The connection table could not be read.".to_string());
        "unknown"
    } else {
        let desk = facts.desk.as_ref().unwrap();
        let mut mons: Vec<Mon> = desk.monitors().to_vec();
        mons.sort_by_key(|mon| (mon.x, mon.y, mon.w, mon.h));
        let open: Vec<&Mon> = mons.iter().filter(|mon| desk.edge_open(mon, facts.edge)).collect();
        let seams: Vec<&Mon> = mons
            .iter()
            .filter(|mon| !desk.edge_open(mon, facts.edge))
            .collect();
        if open.is_empty() {
            lines.push(format!(
                "The {} edge is where the screens touch, so a move stays on {}.",
                facts.edge.as_str(),
                facts.here
            ));
            if facts.link != Link::Up {
                lines.push(format!(
                    "{} is not connected, so the pointer stays on {}.",
                    facts.peer, facts.here
                ));
            }
            "no"
        } else if facts.link != Link::Up {
            lines.push(format!(
                "{} is not connected, so the pointer stays on {}.",
                facts.peer, facts.here
            ));
            "no"
        } else {
            lines.push(format!(
                "A move off an open {} edge would hand the pointer to {}.",
                facts.edge.as_str(),
                facts.peer
            ));
            for mon in seams {
                lines.push(format!(
                    "The {} edge of {} touches another screen, so a move there stays on {}.",
                    facts.edge.as_str(),
                    mon_label(mon),
                    facts.here
                ));
            }
            "yes"
        }
    };
    lines.push("The pointer was not moved.".to_string());
    (could, lines)
}

fn screen_lines(desk: &Desk, edge: Edge) -> Vec<String> {
    let mut mons: Vec<Mon> = desk.monitors().to_vec();
    mons.sort_by_key(|mon| (mon.x, mon.y, mon.w, mon.h));
    mons.iter()
        .map(|mon| {
            let primary = if mon.primary { " primary" } else { "" };
            let state = if desk.edge_open(mon, edge) {
                "open"
            } else {
                "touches another screen"
            };
            format!(
                "{}x{} at {},{}{}  {} {}",
                mon.w,
                mon.h,
                mon.x,
                mon.y,
                primary,
                edge.as_str(),
                state
            )
        })
        .collect()
}

fn mon_label(mon: &Mon) -> String {
    let primary = if mon.primary { " primary" } else { "" };
    format!(
        "{}x{} at {},{}{}",
        mon.w, mon.h, mon.x, mon.y, primary
    )
}

fn mention(note: &Note, here: &str) -> Option<String> {
    if !note.from.is_empty() && note.from != here {
        return Some(note.from.clone());
    }
    if !note.to.is_empty() && note.to != here {
        return Some(note.to.clone());
    }
    None
}

fn peer_host_of(settings: &Settings) -> &str {
    #[cfg(target_os = "macos")]
    {
        settings.allow.as_str()
    }
    #[cfg(not(target_os = "macos"))]
    {
        settings.host.as_str()
    }
}

fn named_peer(path: &Path, here: &str) -> Option<String> {
    let store = inbox::Store::open(inbox::path_for(path)).ok()?;
    other_name(store.notes(), here)
}

fn observe(host: &str, port: u16) -> (Link, bool) {
    let addrs = match net::resolve(host, port) {
        Ok(addrs) => addrs,
        Err(_) => return (Link::Down, true),
    };
    match sessions() {
        Ok(rows) => {
            if peer_up(&rows, &addrs, port) {
                (Link::Up, false)
            } else {
                (Link::Down, false)
            }
        }
        Err(_) => (Link::Unread, false),
    }
}

fn live_desk() -> Option<Desk> {
    #[cfg(windows)]
    {
        crate::win::screens().ok()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn scale_text(scale: f64) -> String {
    let text = format!("{scale}");
    if text.contains('.') {
        text
    } else {
        format!("{scale:.1}")
    }
}

fn parse_proc_endpoint(text: &str) -> Option<(Ipv4Addr, u16)> {
    let (ip_hex, port_hex) = text.split_once(':')?;
    if ip_hex.len() != 8 {
        return None;
    }
    let ip = u32::from_str_radix(ip_hex, 16).ok()?;
    let port = u16::from_str_radix(port_hex, 16).ok()?;
    Some((Ipv4Addr::from(u32::from_be(ip)), port))
}

fn split_v4(text: &str) -> Option<(Ipv4Addr, u16)> {
    let (host, port) = text.rsplit_once('.')?;
    Some((host.parse().ok()?, port.parse().ok()?))
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn ipv4_from_dword(addr: u32) -> Ipv4Addr {
    Ipv4Addr::from(u32::from_be(addr))
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn win_port(value: u32) -> u16 {
    u16::from_be(value as u16)
}

#[cfg(windows)]
fn sessions() -> io::Result<Vec<Session>> {
    use std::mem::size_of;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    };
    const AF_INET: u32 = 2;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    let mut size = 0u32;
    let mut buf = Vec::new();
    let mut code = ERROR_INSUFFICIENT_BUFFER;
    for _ in 0..5 {
        code = unsafe {
            GetExtendedTcpTable(
                if buf.is_empty() {
                    None
                } else {
                    Some(buf.as_mut_ptr() as *mut _)
                },
                &mut size,
                false,
                AF_INET,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        if code == 0 {
            break;
        }
        if code != ERROR_INSUFFICIENT_BUFFER || size == 0 {
            return Err(io::Error::new(ErrorKind::Other, format!("tcp table {code}")));
        }
        buf.resize(size as usize, 0);
    }
    if code != 0 || buf.len() < size_of::<MIB_TCPTABLE_OWNER_PID>() {
        return Err(io::Error::new(ErrorKind::Other, format!("tcp table {code}")));
    }
    let table = unsafe { &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID) };
    let count = table.dwNumEntries as usize;
    let row_size = size_of::<MIB_TCPROW_OWNER_PID>();
    let max = buf.len().saturating_sub(size_of::<u32>()) / row_size.max(1);
    if count > max {
        return Err(io::Error::new(ErrorKind::InvalidData, "tcp table"));
    }
    let rows = unsafe { std::slice::from_raw_parts(table.table.as_ptr(), count) };
    let mut out = Vec::new();
    for row in rows {
        if row.dwState != 5 {
            continue;
        }
        out.push(Session {
            local_ip: ipv4_from_dword(row.dwLocalAddr),
            local_port: win_port(row.dwLocalPort),
            remote_ip: ipv4_from_dword(row.dwRemoteAddr),
            remote_port: win_port(row.dwRemotePort),
        });
    }
    Ok(out)
}

#[cfg(target_os = "linux")]
fn sessions() -> io::Result<Vec<Session>> {
    let text = std::fs::read_to_string("/proc/net/tcp")?;
    Ok(parse_proc_tcp(&text))
}

#[cfg(target_os = "macos")]
fn sessions() -> io::Result<Vec<Session>> {
    let output = std::process::Command::new("netstat")
        .args(["-n", "-p", "tcp"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::new(ErrorKind::Other, "netstat"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_netstat(&text))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn sessions() -> io::Result<Vec<Session>> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "no connection table",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddrV4;

    fn note(rev: u32, deleted: bool, from: &str, to: &str, text: &str) -> Note {
        Note {
            id: rev,
            rev,
            deleted,
            from: from.to_string(),
            to: to.to_string(),
            text: text.to_string(),
        }
    }

    fn facts(link: Link, desk: Option<Desk>, owns: bool) -> Facts {
        Facts {
            release: "0.1.9".to_string(),
            here: "Gray Castleberry".to_string(),
            peer: "Williams Schooley".to_string(),
            peer_host: "daniels-macbook-pro.tail0ffb4a.ts.net".to_string(),
            link,
            unresolved: false,
            owns_pointer: owns,
            edge: Edge::Left,
            desk,
            path: "settings.toml".to_string(),
            host: "daniels-macbook-pro.tail0ffb4a.ts.net".to_string(),
            port: 47940,
            scale: 1.0,
            allow: "darwin-dell-xps.tail0ffb4a.ts.net".to_string(),
        }
    }

    fn gap() -> Desk {
        Desk::new(vec![
            Mon { x: -3840, y: 0, w: 2560, h: 1440, primary: false },
            Mon { x: 0, y: 0, w: 1920, h: 1200, primary: true },
        ])
    }

    #[test]
    fn an_open_edge_and_a_live_link_could_move() {
        let text = render(&facts(Link::Up, Some(gap()), true));
        assert!(text.contains("Gray Castleberry is here."));
        assert!(text.contains("Williams Schooley is connected."));
        assert!(text.contains("could move: yes"));
        assert!(text.contains("A move off an open left edge would hand the pointer to Williams Schooley."));
        assert!(text.contains("The pointer was not moved."));
        assert!(text.contains("1920x1200 at 0,0 primary  left open"));
        assert!(text.contains("2560x1440 at -3840,0  left open"));
        assert!(!text.contains("100.72.135.59"));
        assert!(!text.contains("entered"));
    }

    #[test]
    fn a_down_peer_keeps_the_pointer_here() {
        let text = render(&facts(Link::Down, Some(gap()), true));
        assert!(text.contains("Williams Schooley is not connected."));
        assert!(text.contains("could move: no"));
        assert!(text.contains("Williams Schooley is not connected, so the pointer stays on Gray Castleberry."));
        assert!(text.contains("The pointer was not moved."));
    }

    #[test]
    fn a_seam_does_not_count_as_a_way_over() {
        let touching = Desk::new(vec![
            Mon { x: 0, y: 0, w: 100, h: 100, primary: true },
            Mon { x: 100, y: 0, w: 100, h: 100, primary: false },
        ]);
        let text = render(&facts(Link::Up, Some(touching), true));
        assert!(text.contains("could move: yes"));
        assert!(text.contains("The left edge of 100x100 at 100,0 touches another screen, so a move there stays on Gray Castleberry."));
        assert!(text.contains("100x100 at 0,0 primary  left open"));
    }

    #[test]
    fn no_open_edge_cannot_move() {
        let thin = Desk::new(vec![Mon { x: 0, y: 0, w: 1, h: 10, primary: true }]);
        let text = render(&facts(Link::Up, Some(thin), true));
        assert!(text.contains("could move: no"));
        assert!(text.contains("where the screens touch"));
    }

    #[test]
    fn the_listener_does_not_invent_a_windows_edge() {
        let text = render(&facts(Link::Up, None, false));
        assert!(text.contains("Williams Schooley is connected."));
        assert!(text.contains("could move: unknown"));
        assert!(text.contains("This machine only injects."));
        assert!(text.contains("The pointer was not moved."));
        assert!(!text.contains("screens now"));
    }

    #[test]
    fn a_missing_name_is_not_called_down() {
        let mut sample = facts(Link::Unread, Some(gap()), true);
        sample.unresolved = false;
        let text = render(&sample);
        assert!(text.contains("The connection to Williams Schooley was not read."));
        assert!(text.contains("could move: unknown"));
        assert!(!text.contains("is not connected"));
    }

    #[test]
    fn the_other_name_skips_this_machine_and_deleted_notes() {
        let notes = vec![
            note(1, false, "Gray Castleberry", "", "hello from windows"),
            note(2, true, "Old Name", "", "gone"),
            note(3, false, "Gray Castleberry", "Williams Schooley", "edge left"),
            note(4, false, "Williams Schooley", "Gray Castleberry", "connected, do not print this sentence"),
        ];
        assert_eq!(
            other_name(&notes, "Gray Castleberry").as_deref(),
            Some("Williams Schooley")
        );
        assert_eq!(other_name(&notes[..1], "Gray Castleberry"), None);
    }

    #[test]
    fn only_the_bezel_port_counts_as_connected() {
        let peer = Ipv4Addr::new(100, 72, 135, 59);
        let here = Ipv4Addr::new(100, 84, 4, 24);
        let sessions = [
            Session { local_ip: here, local_port: 50000, remote_ip: peer, remote_port: 47940 },
            Session { local_ip: here, local_port: 22, remote_ip: peer, remote_port: 22 },
        ];
        let addr = SocketAddr::V4(SocketAddrV4::new(peer, 47940));
        assert!(peer_up(&sessions, &[addr], 47940));
        assert!(!peer_up(&sessions[1..], &[addr], 47940));
        let inbound = [Session {
            local_ip: peer,
            local_port: 47940,
            remote_ip: here,
            remote_port: 50000,
        }];
        let windows = SocketAddr::V4(SocketAddrV4::new(here, 47940));
        assert!(peer_up(&inbound, &[windows], 47940));
    }

    #[test]
    fn proc_and_netstat_keep_established_rows() {
        let peer = Ipv4Addr::new(100, 72, 135, 59);
        let here = Ipv4Addr::new(100, 84, 4, 24);
        let local = proc_endpoint(here, 50000);
        let remote = proc_endpoint(peer, 47940);
        let text = format!(
            "  sl  local_address rem_address   st\n   0: {local} {remote} 01 00000000:00000000\n   1: {local} {remote} 0A 00000000:00000000\n"
        );
        let rows = parse_proc_tcp(&text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].remote_ip, peer);
        assert_eq!(rows[0].remote_port, 47940);
        assert_eq!(rows[0].local_ip, here);
        let netstat = "\
tcp4       0      0  100.84.4.24.50000      100.72.135.59.47940    ESTABLISHED
tcp4       0      0  *.47940                *.*                    LISTEN
tcp4       0      0  100.84.4.24.22         100.72.135.59.22       ESTABLISHED
";
        let rows = parse_netstat(netstat);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].remote_port, 47940);
        assert_eq!(rows[1].remote_port, 22);
    }

    #[test]
    fn a_windows_dword_is_the_address() {
        assert_eq!(ipv4_from_dword(0x0100_000A), Ipv4Addr::new(10, 0, 0, 1));
        // 47940 is network order 0x44BB in the low half of the dword.
        assert_eq!(win_port(0x0000_44BB), 47940);
    }

    #[test]
    fn deleted_messages_stay_out_of_the_list() {
        let notes = vec![
            note(1, true, "Williams Schooley", "Gray Castleberry", "secret tombstone"),
            note(2, false, "Williams Schooley", "Gray Castleberry", "still here"),
        ];
        let text = message_lines(&notes);
        assert!(text.contains("still here"));
        assert!(text.contains("Williams Schooley -> Gray Castleberry"));
        assert!(!text.contains("secret tombstone"));
    }

    #[test]
    fn doctor_addresses_a_message_to_the_other_machine() {
        let root = std::env::temp_dir().join(format!(
            "bezel-doctor-{}-{}",
            std::process::id(),
            ipv4_from_dword(0x0100_000A).octets()[0]
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let settings = root.join("settings.toml");
        let mut store = inbox::Store::open(inbox::path_for(&settings)).unwrap();
        store
            .observe(crate::frame::Frame::Note {
                id: 7,
                rev: 1,
                deleted: false,
                from: "Williams Schooley".into(),
                to: who::name(),
                text: "connected".into(),
            })
            .unwrap();
        let note = add_message(&settings, "the open edge is the far left").unwrap();
        assert_eq!(note.from, who::name());
        assert_eq!(note.to, "Williams Schooley");
        update_message(&settings, note.id, "still the far left").unwrap();
        let listed = read_messages(&settings).unwrap();
        assert!(listed.contains("still the far left"));
        assert!(!listed.contains("the open edge is the far left"));
        delete_message(&settings, note.id).unwrap();
        let listed = read_messages(&settings).unwrap();
        assert!(!listed.contains("still the far left"));
        assert!(listed.contains("connected"));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn proc_endpoint(ip: Ipv4Addr, port: u16) -> String {
        let octets = ip.octets();
        format!(
            "{:02X}{:02X}{:02X}{:02X}:{:04X}",
            octets[3], octets[2], octets[1], octets[0], port
        )
    }
}
