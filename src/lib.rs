//! Bezel moves a pointer from one machine onto another across a screen edge.
//! The Windows process owns the physical devices. The other side only injects.

pub mod allow;
pub mod ballistics;
pub mod desk;
pub mod doctor;
pub mod fake;
pub mod frame;
pub mod hid;
pub mod host;
pub mod inbox;
pub mod inject;
pub mod install;
pub mod machine;
pub mod net;
pub mod report;
pub mod scale;
pub mod settings;
pub mod status;
pub mod trace;
pub mod update;
pub mod upgrade;
pub mod who;

pub const DEFAULT_PORT: u16 = 47940;

#[cfg(windows)]
pub mod win;

#[cfg(target_os = "macos")]
pub mod mac;

#[cfg(target_os = "linux")]
pub mod linux;
