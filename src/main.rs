use bezel::desk::Edge;
use bezel::settings::{self, Settings};
use bezel::{fake, DEFAULT_MAC, DEFAULT_PORT, DEFAULT_WINDOWS};
use std::env;
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("bezel: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        Some("listen") => listen(&args[1..]),
        Some("update") => update(&args[1..]),
        Some("fake") => {
            let port = flag_u16(&args[1..], "--port")?.unwrap_or(DEFAULT_PORT);
            fake::listen(port)
        }
        Some("--to") => {
            let mut rest = &args[1..];
            let mut host = None;
            if rest.first().is_some_and(|arg| !arg.starts_with('-')) {
                host = rest.first().cloned();
                rest = &args[2..];
            }
            let (mut settings, path) = load_settings(rest)?;
            if let Some(host) = host {
                settings.host = host;
            }
            capture(&settings, &path, !flag_present(rest, "--no-upgrade"))
        }
        Some(other) => Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("unknown command {other}"),
        )),
    }
}

fn capture(settings: &Settings, config: &std::path::Path, upgrade: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        bezel::win::run(
            &settings.host,
            settings.port,
            settings.scale,
            settings.edge,
            config,
            upgrade,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (settings, config, upgrade);
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "bezel --to runs on Windows",
        ))
    }
}

fn listen(args: &[String]) -> io::Result<()> {
    let (settings, _path) = load_settings(args)?;
    let upgrade = !flag_present(args, "--no-upgrade");
    #[cfg(target_os = "macos")]
    {
        bezel::mac::listen(settings.port, &settings.allow, upgrade)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (settings, upgrade);
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "bezel listen runs on the Mac",
        ))
    }
}

fn update(args: &[String]) -> io::Result<()> {
    let (settings, _path) = load_settings(args)?;
    let root = bezel::update::find_root()?;
    let reply = bezel::update::push(&settings.host, settings.port, &root)?;
    let reply = reply.trim();
    if let Some(rest) = reply.strip_prefix("err") {
        return Err(io::Error::new(
            ErrorKind::Other,
            if rest.trim().is_empty() {
                "update failed".to_string()
            } else {
                rest.trim().to_string()
            },
        ));
    }
    println!("bezel: {reply}");
    Ok(())
}

fn load_settings(args: &[String]) -> io::Result<(Settings, PathBuf)> {
    let path = flag_str(args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(settings::default_path);
    let mut settings = settings::open(&path)?;
    eprintln!("bezel: settings {}", path.display());
    if let Some(port) = flag_u16(args, "--port")? {
        settings.port = port;
    }
    if let Some(scale) = flag_f64(args, "--scale")? {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(io::Error::new(ErrorKind::InvalidInput, "bad --scale"));
        }
        settings.scale = scale;
    }
    if let Some(edge) = flag_str(args, "--edge") {
        settings.edge = Edge::parse(&edge.to_ascii_lowercase())
            .map_err(|err| io::Error::new(ErrorKind::InvalidInput, err))?;
    }
    if let Some(allow) = flag_str(args, "--allow") {
        settings.allow = allow.to_string();
    }
    Ok((settings, path))
}

fn flag_present(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

fn flag_str<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn flag_u16(args: &[String], name: &str) -> io::Result<Option<u16>> {
    match flag_str(args, name) {
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| io::Error::new(ErrorKind::InvalidInput, format!("bad {name}"))),
        None => Ok(None),
    }
}

fn flag_f64(args: &[String], name: &str) -> io::Result<Option<f64>> {
    match flag_str(args, name) {
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| io::Error::new(ErrorKind::InvalidInput, format!("bad {name}"))),
        None => Ok(None),
    }
}

fn print_usage() {
    let path = settings::default_path().display().to_string();
    println!(
        "\
bezel --to [{DEFAULT_MAC}] [--port {DEFAULT_PORT}] [--scale 1.0] [--edge left] [--no-upgrade]
bezel listen [--port {DEFAULT_PORT}] [--allow {DEFAULT_WINDOWS}] [--no-upgrade]
bezel update
bezel fake [--port {DEFAULT_PORT}]

Settings: {path}
Flags override the file until the file is saved again. Windows sends the edge on the connection.
Both machines replace themselves from the latest GitHub release binary. --no-upgrade stays on this copy.
bezel update sends this tree to the Mac, which builds it and restarts listen.

Ctrl becomes Command. Alt becomes Option. Shift stays Shift.
The escape chord is Ctrl+Alt+Shift."
    );
}
