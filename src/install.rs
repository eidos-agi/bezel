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
    register_task(exe)?;
    if already_running() {
        println!("bezel: already running. It will come back at logon.");
        return Ok(());
    }
    spawn_capture(exe)?;
    println!("bezel: started");
    Ok(())
}

/// Start the capture when it is gone. Leave a connected capture alone.
pub fn keep() -> io::Result<()> {
    #[cfg(not(windows))]
    {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()?;
        let status_path = status::path_for(&settings::default_path());
        let status = fs::read_to_string(status_path).unwrap_or_default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_secs())
            .unwrap_or(0);
        match decide(&exe.display().to_string(), &bezel_processes()?, &status, now) {
            Keep::Leave => Ok(()),
            Keep::Start => spawn_capture(&exe),
            Keep::Restart(pid) => {
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .status();
                spawn_capture(&exe)
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Keep {
    Leave,
    Start,
    Restart(u32),
}

fn decide(self_exe: &str, procs: &[(u32, String)], status: &str, now: u64) -> Keep {
    if procs.is_empty() {
        return Keep::Start;
    }
    if procs.len() != 1 {
        return Keep::Leave;
    }
    let (pid, path) = &procs[0];
    if !self_exe.eq_ignore_ascii_case(path) {
        return Keep::Leave;
    }
    if field(status, "link") != "quiet" {
        return Keep::Leave;
    }
    if field(status, "pid").parse::<u32>().ok() != Some(*pid) {
        return Keep::Leave;
    }
    let Some(missed) = parse_utc(field(status, "missed")) else {
        return Keep::Leave;
    };
    if now.saturating_sub(missed) < 30 {
        return Keep::Leave;
    }
    Keep::Restart(*pid)
}

fn field<'a>(text: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key}: ");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or("")
        .trim()
}

fn parse_utc(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' || bytes[13] != b':' || bytes[16] != b':' || bytes[19] != b'Z' {
        return None;
    }
    let year: i64 = text[0..4].parse().ok()?;
    let month: u32 = text[5..7].parse().ok()?;
    let day: u32 = text[8..10].parse().ok()?;
    let hour: u64 = text[11..13].parse().ok()?;
    let minute: u64 = text[14..16].parse().ok()?;
    let second: u64 = text[17..19].parse().ok()?;
    if month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some((days as u64).saturating_mul(86_400) + hour * 3600 + minute * 60 + second)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

pub fn windows_task(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
    <TimeTrigger>
      <Repetition>
        <Interval>PT2M</Interval>
        <Duration>P3650D</Duration>
        <StopAtDurationEnd>false</StopAtDurationEnd>
      </Repetition>
      <StartBoundary>2026-01-01T00:00:00</StartBoundary>
      <Enabled>true</Enabled>
    </TimeTrigger>
  </Triggers>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <Hidden>true</Hidden>
    <ExecutionTimeLimit>PT1M</ExecutionTimeLimit>
  </Settings>
  <Actions>
    <Exec>
      <Command>{exe}</Command>
      <Arguments>keep</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        exe = xml_escape(&exe.display().to_string())
    )
}

#[cfg(windows)]
fn register_task(exe: &Path) -> io::Result<()> {
    let path = std::env::temp_dir().join("bezel-task.xml");
    let mut bytes = vec![0xFF, 0xFE];
    for unit in windows_task(exe).encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(&path, &bytes)?;
    let status = Command::new("schtasks")
        .args(["/Create", "/TN", "Bezel", "/XML"])
        .arg(&path)
        .arg("/F")
        .status()
        .map_err(|err| io::Error::new(ErrorKind::Other, format!("schtasks ({err})")))?;
    let _ = fs::remove_file(&path);
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::Other,
            "schtasks could not register Bezel",
        ))
    }
}

#[cfg(windows)]
fn spawn_capture(exe: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const DETACHED_PROCESS: u32 = 0x00000008;
    Command::new(exe)
        .arg("--to")
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|err| io::Error::new(ErrorKind::Other, format!("could not start bezel ({err})")))
}

#[cfg(windows)]
fn bezel_processes() -> io::Result<Vec<(u32, String)>> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::core::PWSTR;
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|err| io::Error::other(err.to_string()))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = Vec::new();
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let name = utf16_prefix(&entry.szExeFile);
                if name.len() >= 5 && name[..5].eq_ignore_ascii_case("bezel") {
                    if let Some(path) = image_path(entry.th32ProcessID) {
                        found.push((entry.th32ProcessID, path));
                    }
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        return Ok(found);

        fn image_path(pid: u32) -> Option<String> {
            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
                let mut buf = [0u16; 1024];
                let mut len = buf.len() as u32;
                let ok = QueryFullProcessImageNameW(
                    handle,
                    Default::default(),
                    PWSTR(buf.as_mut_ptr()),
                    &mut len,
                );
                let _ = CloseHandle(handle);
                ok.ok()?;
                let text = std::ffi::OsString::from_wide(&buf[..len as usize]);
                Some(text.to_string_lossy().into_owned())
            }
        }

        fn utf16_prefix(buf: &[u16]) -> String {
            let end = buf.iter().position(|unit| *unit == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..end])
        }
    }
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

    #[test]
    fn the_windows_task_runs_keep() {
        let text = windows_task(Path::new("C:\\Users\\ada\\bezel & co.exe"));
        assert!(text.contains("C:\\Users\\ada\\bezel &amp; co.exe"));
        assert!(text.contains("<Arguments>keep</Arguments>"));
        assert!(text.contains("<Interval>PT2M</Interval>"));
        assert!(text.contains("<LogonTrigger>"));
    }

    #[test]
    fn utc_stamps_round_trip() {
        let stamp = crate::status::utc(1_761_490_543);
        assert_eq!(parse_utc(&stamp), Some(1_761_490_543));
        assert_eq!(parse_utc("none"), None);
    }

    #[test]
    fn keep_starts_only_when_nothing_is_running() {
        let exe = r"C:\bezel.exe";
        let connected = "link: connected\npid: 4\nmissed: none\n";
        assert_eq!(decide(exe, &[], connected, 100), Keep::Start);
        assert_eq!(
            decide(exe, &[(4, exe.to_string())], connected, 100),
            Keep::Leave
        );
        assert_eq!(
            decide(exe, &[(9, r"C:\old.exe".to_string())], connected, 100),
            Keep::Leave
        );
    }

    #[test]
    fn keep_restarts_a_quiet_capture_after_thirty_seconds() {
        let exe = r"C:\bezel.exe";
        let missed = crate::status::utc(1_000);
        let status = format!("link: quiet\npid: 4\nmissed: {missed}\n");
        assert_eq!(
            decide(exe, &[(4, exe.to_string())], &status, 1_020),
            Keep::Leave
        );
        assert_eq!(
            decide(exe, &[(4, exe.to_string())], &status, 1_030),
            Keep::Restart(4)
        );
        let other = format!("link: quiet\npid: 9\nmissed: {missed}\n");
        assert_eq!(
            decide(exe, &[(4, exe.to_string())], &other, 1_030),
            Keep::Leave
        );
    }
}
