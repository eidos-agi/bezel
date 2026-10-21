//! Opt-in key trace. Lines go to the console only. They are not stored and
//! they are not filed on GitHub.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);
static LATER: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
    if on {
        eprintln!("bezel: debug keys");
    }
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn dir(down: bool) -> &'static str {
    if down {
        "down"
    } else {
        "up"
    }
}

pub fn key(line: &str) {
    if on() {
        eprintln!("bezel: key {line}");
    }
}

/// The keyboard hook must not print. The UI thread prints these.
pub fn later(line: String) {
    if !on() {
        return;
    }
    let mut lines = LATER.lock().unwrap_or_else(|err| err.into_inner());
    if lines.len() < 256 {
        lines.push(line);
    }
}

pub fn flush() {
    if !on() {
        return;
    }
    let lines = match LATER.lock() {
        Ok(mut lines) => std::mem::take(&mut *lines),
        Err(err) => std::mem::take(&mut *err.into_inner()),
    };
    for line in lines {
        eprintln!("bezel: key {line}");
    }
}
