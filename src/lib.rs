//! Bezel moves a pointer from one machine onto another across a screen edge.
//! The Windows process owns the physical devices. The Mac process only injects.

pub mod allow;
pub mod ballistics;
pub mod desk;
pub mod fake;
pub mod frame;
pub mod hid;
pub mod inbox;
pub mod inject;
pub mod machine;
pub mod net;
pub mod report;
pub mod scale;
pub mod settings;
pub mod update;
pub mod upgrade;
pub mod who;

pub const DEFAULT_PORT: u16 = 47940;
pub const DEFAULT_MAC: &str = "daniels-macbook-pro.tail0ffb4a.ts.net";
pub const DEFAULT_WINDOWS: &str = "darwin-dell-xps.tail0ffb4a.ts.net";

#[cfg(windows)]
pub mod win;

#[cfg(target_os = "macos")]
pub mod mac;
