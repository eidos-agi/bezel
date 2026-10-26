//! This machine's MagicDNS name, for the other machine's settings file.

use std::io::{self, ErrorKind};
use std::process::Command;

pub fn print() -> io::Result<()> {
    let json = tailscale_json()?;
    let name = dns_name(&json).ok_or_else(|| {
        io::Error::new(ErrorKind::NotFound, "tailscale did not report a DNS name")
    })?;
    println!("{name}");
    Ok(())
}

pub fn dns_name(json: &str) -> Option<String> {
    let self_at = json.find("\"Self\"")?;
    let rest = &json[self_at..];
    let key = "\"DNSName\"";
    let at = rest.find(key)? + key.len();
    let mut tail = rest[at..].trim_start();
    tail = tail.strip_prefix(':')?.trim_start();
    tail = tail.strip_prefix('"')?;
    let end = tail.find('"')?;
    let mut name = tail[..end].trim().to_string();
    if name.ends_with('.') {
        name.pop();
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn tailscale_json() -> io::Result<String> {
    let mut last = None;
    for bin in bins() {
        let output = match Command::new(bin).args(["status", "--json"]).output() {
            Ok(output) => output,
            Err(err) => {
                last = Some(err);
                continue;
            }
        };
        if !output.status.success() {
            continue;
        }
        return String::from_utf8(output.stdout).map_err(|_| {
            io::Error::new(ErrorKind::InvalidData, "tailscale status was not text")
        });
    }
    Err(io::Error::new(
        ErrorKind::NotFound,
        match last {
            Some(err) => format!("tailscale is not installed ({err})"),
            None => "tailscale is not installed".to_string(),
        },
    ))
}

fn bins() -> Vec<&'static str> {
    let mut bins = vec!["tailscale"];
    if cfg!(windows) {
        bins.push("C:\\Program Files\\Tailscale\\tailscale.exe");
    }
    if cfg!(target_os = "macos") {
        bins.push("/Applications/Tailscale.app/Contents/MacOS/Tailscale");
        bins.push("/opt/homebrew/bin/tailscale");
        bins.push("/usr/local/bin/tailscale");
    }
    if cfg!(target_os = "linux") {
        bins.push("/usr/bin/tailscale");
    }
    bins
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_self_dns_name_drops_the_trailing_dot() {
        let json = r#"{"Self":{"DNSName":"darwin-thinkpad.tail0ffb4a.ts.net.","Online":true},"Peer":{"DNSName":"other.tail.net."}}"#;
        assert_eq!(
            dns_name(json).as_deref(),
            Some("darwin-thinkpad.tail0ffb4a.ts.net")
        );
        let pretty = "{\n  \"Self\": {\n    \"DNSName\": \"darwin-dell-xps.tail0ffb4a.ts.net.\"\n  }\n}";
        assert_eq!(
            dns_name(pretty).as_deref(),
            Some("darwin-dell-xps.tail0ffb4a.ts.net")
        );
    }
}
