//! Both machines file a problem on the GitHub repo. An open issue with the
//! same title gets a comment. The token is never printed.

use crate::upgrade;
use crate::who;
use std::collections::BTreeSet;
use std::io::{self, ErrorKind, Write};
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static FILED: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
static REPO: Mutex<Option<String>> = Mutex::new(None);

/// Problems are filed only when this is an `owner/name` repo. Empty stays quiet.
pub fn configure(repo: &str) {
    let repo = repo.trim();
    let value = if repo.is_empty() || !repo_ok(repo) {
        None
    } else {
        Some(repo.to_string())
    };
    *REPO.lock().unwrap_or_else(|err| err.into_inner()) = value;
}

fn repo_ok(repo: &str) -> bool {
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) => {
            !owner.is_empty()
                && !name.is_empty()
                && owner
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
        }
        _ => false,
    }
}

fn configured_repo() -> Option<String> {
    REPO.lock().unwrap_or_else(|err| err.into_inner()).clone()
}

pub fn submit(kind: &'static str, summary: &str) {
    if configured_repo().is_none() {
        return;
    }
    let mut filed = FILED.lock().unwrap_or_else(|err| err.into_inner());
    if !filed.insert(kind) {
        return;
    }
    drop(filed);
    let summary = summary.to_string();
    thread::spawn(move || match file(kind, &summary) {
        Ok(url) => eprintln!("bezel: filed {url}"),
        Err(err) => eprintln!("bezel: problem not filed ({err})"),
    });
}

pub fn file_now(kind: &str, summary: &str) -> io::Result<()> {
    let url = file(kind, summary)?;
    println!("bezel: {url}");
    Ok(())
}

fn file(kind: &str, summary: &str) -> io::Result<String> {
    let repo = configured_repo().ok_or_else(|| {
        io::Error::new(ErrorKind::NotFound, "report repo is unset")
    })?;
    let token = github_token().ok_or_else(|| {
        io::Error::new(ErrorKind::PermissionDenied, "no GitHub token")
    })?;
    let title = format!("bezel: {kind}");
    let body = render(summary);
    let number = open_issue(&repo, &title, &token)?;
    let url = if let Some(number) = number {
        comment(&repo, number, &body, &token)?;
        format!("https://github.com/{repo}/issues/{number}")
    } else {
        create(&repo, &title, &body, &token)?
    };
    Ok(url)
}

fn render(summary: &str) -> String {
    format!(
        "{summary}\n\nmachine: {}\nrelease: {}\nos: {}\n",
        who::name(),
        upgrade::current_version(),
        os_name()
    )
}

fn os_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn open_issue(repo: &str, title: &str, token: &str) -> io::Result<Option<u64>> {
    let query = format!("repo:{repo} is:open in:title \"{title}\"");
    let url = format!(
        "https://api.github.com/search/issues?q={}",
        url_encode(&query)
    );
    let json = github(&url, "GET", None, token)?;
    if json_total(&json) == Some(0) {
        return Ok(None);
    }
    Ok(issue_number(&json))
}

fn create(repo: &str, title: &str, body: &str, token: &str) -> io::Result<String> {
    let payload = format!(
        "{{\"title\":\"{}\",\"body\":\"{}\"}}",
        json_escape(title),
        json_escape(body)
    );
    let url = format!("https://api.github.com/repos/{repo}/issues");
    let json = github(&url, "POST", Some(payload), token)?;
    issue_number(&json)
        .map(|number| format!("https://github.com/{repo}/issues/{number}"))
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "GitHub did not return an issue"))
}

fn comment(repo: &str, number: u64, body: &str, token: &str) -> io::Result<()> {
    let payload = format!("{{\"body\":\"{}\"}}", json_escape(body));
    let url = format!("https://api.github.com/repos/{repo}/issues/{number}/comments");
    github(&url, "POST", Some(payload), token)?;
    Ok(())
}

fn github(url: &str, method: &str, payload: Option<String>, token: &str) -> io::Result<String> {
    let mut cmd = Command::new("curl");
    cmd.arg("-fsSL")
        .arg("--max-time")
        .arg("20")
        .arg("-A")
        .arg("bezel")
        .arg("-X")
        .arg(method)
        .arg("-H")
        .arg("Accept: application/vnd.github+json")
        .arg("-H")
        .arg(format!("Authorization: Bearer {token}"));
    let path = if payload.is_some() {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|dur| dur.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("bezel-report-{}-{tick}", std::process::id()));
        if let Some(body) = &payload {
            let mut file = std::fs::File::create(&path)?;
            file.write_all(body.as_bytes())?;
        }
        cmd.arg("-H")
            .arg("Content-Type: application/json")
            .arg("--data-binary")
            .arg(format!("@{}", path.display()));
        Some(path)
    } else {
        None
    };
    cmd.arg(url);
    let output = cmd.output().map_err(|err| {
        io::Error::new(ErrorKind::Other, format!("curl is required to file a problem ({err})"))
    });
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
    let output = output?;
    if !output.status.success() {
        return Err(io::Error::new(ErrorKind::Other, "GitHub did not accept the problem"));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| io::Error::new(ErrorKind::InvalidData, "GitHub response was not text"))
}

fn github_token() -> Option<String> {
    let from_env = std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .ok()
        .filter(|token| !token.is_empty());
    if from_env.is_some() {
        return from_env;
    }
    let output = Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?;
    let token = token.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        None
    } else {
        Some(token.to_string())
    }
}

fn json_total(json: &str) -> Option<u64> {
    let key = "\"total_count\"";
    let idx = json.find(key)?;
    let rest = json[idx + key.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

pub fn issue_number(json: &str) -> Option<u64> {
    let marker = "/issues/";
    let idx = json.find(marker)?;
    let rest = &json[idx + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn json_escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn url_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_repo_does_not_start_a_report() {
        configure("");
        assert!(configured_repo().is_none());
        configure("not a repo");
        assert!(configured_repo().is_none());
        configure("eidos-agi/bezel");
        assert_eq!(configured_repo().as_deref(), Some("eidos-agi/bezel"));
        configure("");
    }

    #[test]
    fn an_issue_url_yields_its_number() {
        let json = r#"{"total_count":1,"items":[{"html_url":"https://github.com/eidos-agi/bezel/issues/12"}]}"#;
        assert_eq!(json_total(json), Some(1));
        assert_eq!(issue_number(json), Some(12));
        assert_eq!(json_total(r#"{"total_count":0}"#), Some(0));
    }

    #[test]
    fn the_body_is_json_and_hides_nothing_secret_in_the_escape() {
        let escaped = json_escape("line\nquote\"slash\\");
        assert_eq!(escaped, r#"line\nquote\"slash\\"#);
        assert!(!escaped.contains('\n'));
    }

    #[test]
    fn the_search_query_has_no_raw_space() {
        let query = "repo:eidos-agi/bezel is:open in:title \"bezel: enter-timeout\"";
        let encoded = url_encode(query);
        assert!(!encoded.contains(' '));
        assert!(encoded.contains("%22bezel%3A%20enter-timeout%22"));
    }
}
