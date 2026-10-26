//! A stable name for this machine. The same machine id always yields the same
//! "First Last". The lists are frozen: reordering them would rename every machine.
//!
//! First names are the 10,000 most common given names in the Social Security
//! Administration files for 1880, 1920, 1950, 1980, and 2000. Last names are the
//! 10,000 most common surnames in the 2010 Census. Both are US government work.
//! Each file is alphabetical.

#[cfg(any(windows, target_os = "macos"))]
use std::process::Command;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const FIRST_RAW: &str = include_str!("../names/first.txt");
const LAST_RAW: &str = include_str!("../names/last.txt");

pub fn name() -> String {
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED.get_or_init(|| name_from(&machine_id())).clone()
}

pub fn name_from(id: &str) -> String {
    let id = normalize(id);
    let first = pick(first_names(), &id, 1);
    let last = pick(last_names(), &id, 2);
    format!("{} {}", cap(first), cap(last))
}

fn first_names() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| words(FIRST_RAW))
}

fn last_names() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| words(LAST_RAW))
}

fn words(raw: &str) -> Vec<&str> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn pick<'a>(list: &[&'a str], id: &str, lane: u64) -> &'a str {
    let index = (mix(fnv(id.as_bytes()), lane) as usize) % list.len();
    list[index]
}

fn cap(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => {
            let mut out = first.to_ascii_uppercase().to_string();
            out.push_str(&chars.as_str().to_ascii_lowercase());
            out
        }
        None => String::new(),
    }
}

fn normalize(id: &str) -> String {
    id.trim()
        .trim_matches(|c| c == '{' || c == '}' || c == '"')
        .to_ascii_lowercase()
}

/// FNV-1a, then splitmix64. Both are fixed so a name never drifts.
fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn mix(hash: u64, lane: u64) -> u64 {
    let mut z = hash.wrapping_add(lane.wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

pub fn machine_id() -> String {
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED.get_or_init(|| {
        platform_id()
            .map(|id| normalize(&id))
            .filter(|id| !id.is_empty())
            .unwrap_or_else(fallback_id)
    })
    .clone()
}

#[cfg(windows)]
fn platform_id() -> Option<String> {
    let output = Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .find(|token| token.len() >= 32 && token.contains('-'))
        .map(str::to_string)
}

#[cfg(target_os = "macos")]
fn platform_id() -> Option<String> {
    let output = Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("\"IOPlatformUUID\"") else {
            continue;
        };
        if let Some(uuid) = rest.split('"').find(|part| part.len() >= 32) {
            return Some(uuid.to_string());
        }
    }
    None
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_id() -> Option<String> {
    let text = std::fs::read_to_string("/etc/machine-id").ok()?;
    let id = text.trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

fn fallback_id() -> String {
    if cfg!(test) {
        return "test-machine".to_string();
    }
    let path = crate::settings::default_path()
        .with_file_name("who.id");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let id = normalize(&text);
        if !id.is_empty() {
            return id;
        }
    }
    let tick = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_nanos())
        .unwrap_or(0);
    let id = format!("{:032x}", mix(fnv(&tick.to_le_bytes()), 9));
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn ten_thousand_of_each() {
        assert_eq!(words(FIRST_RAW).len(), 10_000);
        assert_eq!(words(LAST_RAW).len(), 10_000);
    }

    #[test]
    fn the_same_id_keeps_the_same_name() {
        assert_eq!(name_from("machine-a"), name_from("machine-a"));
        assert_eq!(name_from(" Machine-A "), name_from("machine-a"));
        assert_ne!(name_from("machine-a"), name_from("machine-b"));
        let name = name_from("machine-a");
        let mut parts = name.split(' ');
        let first = parts.next().unwrap();
        let last = parts.next().unwrap();
        assert!(parts.next().is_none());
        assert!(first.chars().next().unwrap().is_ascii_uppercase());
        assert!(last.chars().next().unwrap().is_ascii_uppercase());
    }

    #[test]
    fn nearby_ids_land_on_different_names() {
        let names: BTreeSet<_> = (0..200).map(|n| name_from(&format!("machine-{n}"))).collect();
        assert!(names.len() > 190);
    }
}
