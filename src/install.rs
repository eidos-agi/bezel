//! Install the listener or the capture process, then print the one human step.
//! A copy that is already running is left running.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::process::Command;

use crate::settings;
use crate::status;

pub fn install() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    #[cfg(windows)]
    {
        install_windows(&exe)
    }
    #[cfg(target_os = "linux")]
    {
        install_linux(&exe)
    }
    #[cfg(target_os = "macos")]
    {
        install_mac(&exe)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = exe;
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "bezel install runs on Windows, Mac, or Linux",
        ))
    }
}

pub fn linux_unit(exe: &Path) -> String {
    format!(
        "\
[Unit]
Description=Bezel pointer listener
After=graphical-session.target

[Service]
ExecStart={exe} listen
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
",
        exe = exe.display()
    )
}

pub fn mac_plist(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.eidos-agi.bezel</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>listen</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
"#,
        exe = xml_escape(&exe.display().to_string())
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn already_running() -> bool {
    let path = status::path_for(&settings::default_path());
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Some(pid) = status::pid_of(&text) else {
        return false;
    };
    pid_alive(pid)
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
            let Ok(handle) = handle else {
                return false;
            };
            let _ = CloseHandle(handle);
            true
        }
    }
}

#[cfg(target_os = "linux")]
fn install_linux(exe: &Path) -> io::Result<()> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").ok_or_else(|| {
        io::Error::new(ErrorKind::NotFound, "HOME is unset")
    })?;
    let dir = PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user");
    fs::create_dir_all(&dir)?;
    let unit = dir.join("bezel.service");
    fs::write(&unit, linux_unit(exe))?;
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    let _ = Command::new("systemctl")
        .args(["--user", "enable", "bezel.service"])
        .status();
    if already_running() {
        println!("bezel: listener already running. Start it again to use this binary.");
    } else {
        let status = Command::new("systemctl")
            .args(["--user", "start", "bezel.service"])
            .status()
            .map_err(|err| io::Error::new(ErrorKind::Other, format!("systemctl ({err})")))?;
        if !status.success() {
            return Err(io::Error::new(
                ErrorKind::Other,
                "systemctl --user start bezel.service failed",
            ));
        }
    }
    println!("bezel: wrote {}", unit.display());
    println!("bezel: allow keyboard, pointer, and a monitor on the desktop dialog");
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_mac(exe: &Path) -> io::Result<()> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").ok_or_else(|| {
        io::Error::new(ErrorKind::NotFound, "HOME is unset")
    })?;
    let dir = PathBuf::from(home).join("Library").join("LaunchAgents");
    fs::create_dir_all(&dir)?;
    let plist = dir.join("com.eidos-agi.bezel.plist");
    fs::write(&plist, mac_plist(exe))?;
    if already_running() {
        println!("bezel: listener already running. Start it again to use this binary.");
    } else {
        let uid = Command::new("id").arg("-u").output();
        if let Ok(uid) = uid {
            let uid = String::from_utf8_lossy(&uid.stdout);
            let domain = format!("gui/{}", uid.trim());
            let _ = Command::new("launchctl")
                .args(["bootstrap", &domain])
                .arg(&plist)
                .status();
        }
    }
    println!("bezel: wrote {}", plist.display());
    println!(
        "bezel: grant Accessibility to the process that launches bezel listen, then start it again"
    );
    Ok(())
}

#[cfg(windows)]
fn install_windows(exe: &Path) -> io::Result<()> {
    if already_running() {
        println!("bezel: already running. Start it again to use this binary.");
        return Ok(());
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const DETACHED_PROCESS: u32 = 0x00000008;
    Command::new(exe)
        .arg("--to")
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map_err(|err| io::Error::new(ErrorKind::Other, format!("could not start bezel ({err})")))?;
    println!("bezel: started");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linux_unit_points_at_the_binary() {
        let text = linux_unit(Path::new("/home/ada/.local/bin/bezel"));
        assert!(text.contains("ExecStart=/home/ada/.local/bin/bezel listen\n"));
        assert!(text.contains("WantedBy=default.target"));
    }

    #[test]
    fn the_mac_plist_escapes_the_path() {
        let text = mac_plist(Path::new("/Users/ada/bezel & co"));
        assert!(text.contains("/Users/ada/bezel &amp; co"));
        assert!(text.contains("<string>listen</string>"));
    }
}
