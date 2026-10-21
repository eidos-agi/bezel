//! Both machines ask the GitHub release for a newer binary and say so.
//! `bezel upgrade` is what installs it.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

pub const REPO: &str = "eidos-agi/bezel";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

static OFFERED: Mutex<Option<Version>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches(['v', 'V']);
        let mut parts = text.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch_text = parts.next()?;
        if parts.next().is_some() || patch_text.contains('-') || patch_text.contains('+') {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch: patch_text.parse().ok()?,
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version {
        major: 0,
        minor: 0,
        patch: 0,
    })
}

pub fn expected_asset() -> Option<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Some("bezel-x86_64-pc-windows-msvc.exe")
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some("bezel-aarch64-apple-darwin")
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Some("bezel-x86_64-apple-darwin")
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    {
        None
    }
}

pub fn choose_asset(json: &str, expected: &str) -> Option<(Version, String)> {
    let version = Version::parse(&json_string(json, "tag_name")?)?;
    let url = asset_url(json, expected)?;
    Some((version, url))
}

pub fn looks_like_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    let mz = bytes.starts_with(b"MZ");
    let macho = matches!(
        bytes[0..4],
        [0xCF, 0xFA, 0xED, 0xFE]
            | [0xFE, 0xED, 0xFA, 0xCF]
            | [0xCA, 0xFE, 0xBA, 0xBE]
            | [0xBE, 0xBA, 0xFE, 0xCA]
            | [0xCE, 0xFA, 0xED, 0xFE]
            | [0xFE, 0xED, 0xFA, 0xCE]
    );
    mz || macho
}

/// Check soon, then every six hours. A newer release is printed once. Nothing is installed.
pub fn watch() {
    thread::spawn(|| loop {
        if let Err(err) = offer() {
            eprintln!("bezel: upgrade check failed ({err})");
        }
        thread::sleep(CHECK_EVERY);
    });
}

pub fn offer() -> io::Result<()> {
    let Some(expected) = expected_asset() else {
        return Ok(());
    };
    let Some((version, _)) = newer_release(expected)? else {
        return Ok(());
    };
    let mut offered = OFFERED.lock().unwrap_or_else(|err| err.into_inner());
    if *offered == Some(version) {
        return Ok(());
    }
    *offered = Some(version);
    eprintln!("bezel: {version} is available. Run bezel upgrade");
    Ok(())
}

/// Install the newer release binary over this one. The user starts bezel again.
pub fn install() -> io::Result<()> {
    let expected = expected_asset().ok_or_else(|| {
        io::Error::new(
            ErrorKind::Unsupported,
            "there is no release binary for this machine",
        )
    })?;
    let Some((version, url)) = newer_release(expected)? else {
        println!("bezel: {} is the latest release", current_version());
        return Ok(());
    };
    eprintln!("bezel: installing {version}");
    let path = download(&url)?;
    let bytes = fs::read(&path)?;
    if bytes.len() < 65_536 || !looks_like_binary(&bytes) {
        let _ = fs::remove_file(&path);
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "the release asset is not a bezel binary",
        ));
    }
    if let Err(err) = swap_in(&path) {
        let _ = fs::remove_file(&path);
        return Err(err);
    }
    println!("bezel: installed {version}. Start bezel again.");
    Ok(())
}

fn newer_release(expected: &str) -> io::Result<Option<(Version, String)>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = match curl_text(&url) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let Some((version, asset_url)) = choose_asset(&body, expected) else {
        return Ok(None);
    };
    if version <= current_version() {
        return Ok(None);
    }
    Ok(Some((version, asset_url)))
}

fn download(url: &str) -> io::Result<PathBuf> {
    let current = std::env::current_exe()?;
    let dest = current.with_file_name("bezel.download");
    let _ = fs::remove_file(&dest);
    curl_file(url, &dest)?;
    Ok(dest)
}

fn swap_in(downloaded: &Path) -> io::Result<PathBuf> {
    let current = std::env::current_exe()?;
    let backup = current.with_file_name("bezel.previous");
    fs::copy(&current, &backup)?;
    #[cfg(windows)]
    {
        let retired = current.with_file_name("bezel.retired");
        let _ = fs::remove_file(&retired);
        fs::rename(&current, &retired)?;
        if let Err(err) = fs::rename(downloaded, &current) {
            let _ = fs::rename(&retired, &current);
            return Err(err);
        }
    }
    #[cfg(not(windows))]
    {
        fs::rename(downloaded, &current)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&current, fs::Permissions::from_mode(0o755))?;
    }
    Ok(backup)
}

fn curl_text(url: &str) -> io::Result<String> {
    let output = curl_cmd(url, None)?;
    if !output.status.success() {
        return Err(curl_error(&output));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| io::Error::new(ErrorKind::InvalidData, "upgrade response was not text"))
}

fn curl_file(url: &str, dest: &Path) -> io::Result<()> {
    let output = curl_cmd(url, Some(dest))?;
    if !output.status.success() {
        let _ = fs::remove_file(dest);
        return Err(curl_error(&output));
    }
    Ok(())
}

fn curl_cmd(url: &str, dest: Option<&Path>) -> io::Result<std::process::Output> {
    let mut cmd = Command::new("curl");
    cmd.arg("-fsSL")
        .arg("--max-time")
        .arg(if dest.is_some() { "180" } else { "20" })
        .arg("-A")
        .arg("bezel")
        .arg("-H")
        .arg("Accept: application/vnd.github+json");
    if let Some(token) = github_token() {
        cmd.arg("-H")
            .arg(format!("Authorization: Bearer {token}"));
    }
    if let Some(dest) = dest {
        cmd.arg("-o").arg(dest);
    }
    cmd.arg(url);
    cmd.output().map_err(|err| {
        io::Error::new(
            ErrorKind::Other,
            format!("curl is required to check for an upgrade ({err})"),
        )
    })
}

fn curl_error(output: &std::process::Output) -> io::Error {
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.contains("404") {
        return io::Error::new(ErrorKind::NotFound, "no published release");
    }
    io::Error::new(
        ErrorKind::Other,
        if detail.is_empty() {
            "upgrade request failed".to_string()
        } else {
            detail.to_string()
        },
    )
}

fn github_token() -> Option<String> {
    std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .ok()
        .filter(|token| !token.is_empty())
}

fn json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let idx = json.find(&needle)?;
    let rest = json[idx + needle.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    unescape(rest)
}

fn asset_url(json: &str, expected: &str) -> Option<String> {
    let mut rest = json;
    while let Some(idx) = rest.find("\"name\"") {
        rest = &rest[idx + 6..];
        let Some(name) = name_here(rest) else {
            continue;
        };
        if name == expected {
            let horizon = &rest[..rest.len().min(8_000)];
            return json_string(horizon, "browser_download_url");
        }
    }
    None
}

fn name_here(rest: &str) -> Option<String> {
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    unescape(rest)
}

fn unescape(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            return Some(out);
        }
        if ch == '\\' {
            out.push(match chars.next()? {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                other => other,
            });
        } else {
            out.push(ch);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sort_as_numbers() {
        let older = Version::parse("v0.1.0").unwrap();
        let newer = Version::parse("0.1.1").unwrap();
        let major = Version::parse("V1.0.0").unwrap();
        assert!(older < newer);
        assert!(newer < major);
        assert_eq!(older, Version::parse("0.1.0").unwrap());
        assert!(Version::parse("0.1.0-beta").is_none());
        assert!(Version::parse("0.1").is_none());
    }

    #[test]
    fn the_matching_asset_is_the_one_for_this_machine() {
        let json = r#"
        {
          "tag_name": "v0.2.0",
          "assets": [
            {"name": "bezel-aarch64-apple-darwin", "browser_download_url": "https://example.test/mac"},
            {"name": "bezel-x86_64-pc-windows-msvc.exe", "browser_download_url": "https://example.test/win"}
          ]
        }
        "#;
        let (version, url) = choose_asset(json, "bezel-x86_64-pc-windows-msvc.exe").unwrap();
        assert_eq!(version, Version::parse("0.2.0").unwrap());
        assert_eq!(url, "https://example.test/win");
        assert!(choose_asset(json, "bezel-other").is_none());
    }

    #[test]
    fn a_binary_starts_with_its_magic() {
        assert!(looks_like_binary(b"MZ\x90\x00rest"));
        assert!(looks_like_binary(&[0xCF, 0xFA, 0xED, 0xFE]));
        assert!(!looks_like_binary(b"<htm"));
        assert!(!looks_like_binary(b"MZ"));
    }
}
