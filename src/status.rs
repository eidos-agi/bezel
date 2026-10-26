//! What the running copy last saw. `bezel status` prints the file.
//! The capture process writes it from the session thread, not from the hook.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::upgrade;

struct Record {
    path: PathBuf,
    release: String,
    peer: String,
    link: String,
    edge: String,
    role: String,
    pid: u32,
    captured: String,
    screens: Vec<String>,
    pointer: String,
    cross: String,
    entered: String,
    left: String,
    released: String,
    missed: String,
    revived: String,
    disconnected: String,
    last_flush: Option<Instant>,
}

static OPEN: Mutex<Option<Record>> = Mutex::new(None);

pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub fn path_for(settings: &Path) -> PathBuf {
    match settings.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join("status.txt"),
        _ => PathBuf::from("status.txt"),
    }
}

pub fn open(settings: &Path, role: &str, edge: &str, screens: &[Rect]) {
    let record = Record {
        path: path_for(settings),
        release: upgrade::current_version().to_string(),
        peer: "unknown".to_string(),
        link: if role == "listen" {
            "listening".to_string()
        } else {
            "waiting".to_string()
        },
        edge: edge.to_string(),
        role: role.to_string(),
        pid: std::process::id(),
        captured: stamp(),
        screens: screens
            .iter()
            .map(|screen| format!("{},{} {}x{}", screen.x, screen.y, screen.w, screen.h))
            .collect(),
        pointer: "none".to_string(),
        cross: "none".to_string(),
        entered: "none".to_string(),
        left: "none".to_string(),
        released: "none".to_string(),
        missed: "none".to_string(),
        revived: "none".to_string(),
        disconnected: "none".to_string(),
        last_flush: None,
    };
    let mut slot = OPEN.lock().unwrap_or_else(|err| err.into_inner());
    *slot = Some(record);
    if let Some(record) = slot.as_mut() {
        let _ = write(record);
    }
}

pub fn link(state: &str) {
    update(|record| record.link = state.to_string());
}

pub fn edge(edge: &str) {
    update(|record| record.edge = edge.to_string());
}

pub fn peer(version: &str) {
    update(|record| record.peer = version.to_string());
}

pub fn pointer(x: i32, y: i32) {
    let mut slot = OPEN.lock().unwrap_or_else(|err| err.into_inner());
    let Some(record) = slot.as_mut() else {
        return;
    };
    record.pointer = format!("{x},{y}");
    let due = record
        .last_flush
        .map(|at| at.elapsed() >= std::time::Duration::from_millis(500))
        .unwrap_or(true);
    if due {
        let _ = write(record);
    }
}

/// A cross changed. `enter` is the attempt. The later words are the result.
pub fn observe(word: &str) {
    let when = stamp();
    update(|record| {
        record.cross = word.to_string();
        match word {
            "entered" => record.entered = when,
            "left" => record.left = when,
            "released" => record.released = when,
            "missed" => record.missed = when,
            "revived" => record.revived = when,
            "disconnected" => record.disconnected = when,
            _ => {}
        }
    });
}

pub fn show(settings: &Path) -> io::Result<()> {
    let path = path_for(settings);
    match fs::read_to_string(&path) {
        Ok(text) => {
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
            Ok(())
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            println!("release: {}", upgrade::current_version());
            println!("status: missing");
            Ok(())
        }
        Err(err) => Err(err),
    }
}

pub fn pid_of(text: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        line.strip_prefix("pid: ")
            .and_then(|rest| rest.trim().parse().ok())
    })
}

fn update(change: impl FnOnce(&mut Record)) {
    let mut slot = OPEN.lock().unwrap_or_else(|err| err.into_inner());
    let Some(record) = slot.as_mut() else {
        return;
    };
    change(record);
    let _ = write(record);
}

fn write(record: &mut Record) -> io::Result<()> {
    if let Some(parent) = record.path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let text = render(record);
    let tmp = record.path.with_extension("txt.tmp");
    fs::write(&tmp, &text)?;
    fs::rename(&tmp, &record.path)?;
    record.last_flush = Some(Instant::now());
    Ok(())
}

fn render(record: &Record) -> String {
    let mut out = format!(
        "\
release: {release}
peer: {peer}
link: {link}
edge: {edge}
role: {role}
pid: {pid}
captured: {captured}
pointer: {pointer}
cross: {cross}
entered: {entered}
left: {left}
released: {released}
missed: {missed}
revived: {revived}
disconnected: {disconnected}
",
        release = record.release,
        peer = record.peer,
        link = record.link,
        edge = record.edge,
        role = record.role,
        pid = record.pid,
        captured = record.captured,
        pointer = record.pointer,
        cross = record.cross,
        entered = record.entered,
        left = record.left,
        released = record.released,
        missed = record.missed,
        revived = record.revived,
        disconnected = record.disconnected,
    );
    if record.screens.is_empty() {
        out.push_str("screen: none\n");
    } else {
        for screen in &record.screens {
            out.push_str("screen: ");
            out.push_str(screen);
            out.push('\n');
        }
    }
    out
}

fn stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_secs())
        .unwrap_or(0);
    utc(secs)
}

pub fn utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_is_utc() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(86_400), "1970-01-02T00:00:00Z");
    }

    #[test]
    fn a_status_file_names_its_pid() {
        let text = "\
release: 0.1.12
pid: 42
link: waiting
";
        assert_eq!(pid_of(text), Some(42));
        assert_eq!(pid_of("release: 0.1.12\n"), None);
    }
}
