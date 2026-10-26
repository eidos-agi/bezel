//! The settings file. Command-line flags override it.

use crate::desk::Edge;
use crate::DEFAULT_PORT;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub scale: f64,
    pub edge: Edge,
    pub allow: String,
    /// GitHub repo that receives problem reports. Empty stays quiet.
    pub report: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: DEFAULT_PORT,
            scale: 1.0,
            edge: Edge::Left,
            allow: String::new(),
            report: String::new(),
        }
    }
}

pub fn default_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("APPDATA") {
        return PathBuf::from(dir).join("bezel").join("settings.toml");
    }
    if cfg!(target_os = "linux") {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
        });
        if let Some(base) = base {
            return base.join("bezel").join("settings.toml");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("bezel")
            .join("settings.toml");
    }
    PathBuf::from("settings.toml")
}

/// Load the file, or write the defaults when it is missing.
pub fn open(path: &Path) -> io::Result<Settings> {
    if path.exists() {
        load(path)
    } else {
        let settings = Settings::default();
        save(path, &settings)?;
        Ok(settings)
    }
}

pub fn load(path: &Path) -> io::Result<Settings> {
    let text = fs::read_to_string(path)?;
    parse(&text).map_err(|err| io::Error::new(ErrorKind::InvalidData, err))
}

pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, render(settings))
}

pub fn render(settings: &Settings) -> String {
    format!(
        "\
# Bezel
# edge = left puts the other machine on the left of the Windows desktop.
# The pointer leaves that outer edge and returns across that machine's facing edge.
# host is that machine. allow is the Windows machine a listener accepts.
# report is a GitHub repo (owner/name). Leave it empty and bezel stays quiet.

host = {host}
port = {port}
scale = {scale}
edge = {edge}
allow = {allow}
report = {report}
",
        host = settings.host,
        port = settings.port,
        scale = format_scale(settings.scale),
        edge = settings.edge.as_str(),
        allow = settings.allow,
        report = settings.report,
    )
}

fn format_scale(scale: f64) -> String {
    let text = format!("{scale}");
    if text.contains('.') {
        text
    } else {
        format!("{scale:.1}")
    }
}

pub fn parse(text: &str) -> Result<Settings, String> {
    let mut settings = Settings::default();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {}: expected key = value", number + 1));
        };
        let key = key.trim();
        let value = unquote(value.trim());
        if value.is_empty() && !matches!(key, "host" | "allow" | "report") {
            return Err(format!("line {}: {key} is empty", number + 1));
        }
        match key {
            "host" => settings.host = value,
            "allow" => settings.allow = value,
            "report" => settings.report = value,
            "port" => {
                settings.port = value
                    .parse()
                    .map_err(|_| format!("line {}: bad port", number + 1))?;
            }
            "scale" => {
                let scale: f64 = value
                    .parse()
                    .map_err(|_| format!("line {}: bad scale", number + 1))?;
                if !scale.is_finite() || scale <= 0.0 {
                    return Err(format!("line {}: bad scale", number + 1));
                }
                settings.scale = scale;
            }
            "edge" => {
                settings.edge = Edge::parse(&value.to_ascii_lowercase())
                    .map_err(|err| format!("line {}: {err}", number + 1))?;
            }
            other => return Err(format!("line {}: unknown setting {other}", number + 1)),
        }
    }
    Ok(settings)
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_text() {
        let text = render(&Settings::default());
        let parsed = parse(&text).unwrap();
        assert_eq!(parsed, Settings::default());
    }

    #[test]
    fn comments_and_a_right_edge() {
        let parsed = parse(
            "
            # note
            edge = right
            scale = 0.5
            host = \"other.tailnet\"
            ",
        )
        .unwrap();
        assert_eq!(parsed.edge, Edge::Right);
        assert_eq!(parsed.scale, 0.5);
        assert_eq!(parsed.host, "other.tailnet");
        assert_eq!(parsed.port, DEFAULT_PORT);
        assert!(parsed.allow.is_empty());
        assert!(parsed.report.is_empty());
    }

    #[test]
    fn an_empty_host_is_kept() {
        let parsed = parse("host =\nallow =\nreport = eidos-agi/bezel\n").unwrap();
        assert!(parsed.host.is_empty());
        assert!(parsed.allow.is_empty());
        assert_eq!(parsed.report, "eidos-agi/bezel");
    }

    #[test]
    fn a_typo_is_an_error() {
        assert!(parse("edje = left\n").is_err());
    }
}
