use bezel::desk::Edge;
use bezel::inbox::{self, Note};
use bezel::settings::{self, Settings};
use bezel::{fake, who, DEFAULT_MAC, DEFAULT_PORT, DEFAULT_WINDOWS};
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
        Some("upgrade") => bezel::upgrade::install(),
        Some("who") => {
            println!("{}", who::name());
            Ok(())
        }
        Some("inbox") => inbox_cmd(&args[1..]),
        Some("report") => report_cmd(&args[1..]),
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
            arm_debug(rest);
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
    arm_debug(args);
    let (settings, path) = load_settings(args)?;
    let upgrade = !flag_present(args, "--no-upgrade");
    #[cfg(target_os = "macos")]
    {
        bezel::mac::listen(settings.port, &settings.allow, upgrade, &path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (settings, path, upgrade);
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

fn report_cmd(args: &[String]) -> io::Result<()> {
    let rest = without_config(args);
    if rest.is_empty() {
        return Err(io::Error::new(ErrorKind::InvalidInput, "missing problem"));
    }
    bezel::report::file_now("report", &rest.join(" "))
}

fn inbox_cmd(args: &[String]) -> io::Result<()> {
    let path = inbox::path_for(&config_path(args));
    let rest = without_config(args);
    let mut store = inbox::Store::open(path)?;
    match rest.first().map(String::as_str) {
        None => {
            println!("{}", who::name());
            let visible: Vec<&Note> = store.notes().iter().filter(|note| !note.deleted).collect();
            if visible.is_empty() {
                println!("inbox is empty");
            }
            for note in visible {
                if note.to.is_empty() {
                    println!("{}\t{}\t{}", note.id, note.from, note.text);
                } else {
                    println!("{}\t{} -> {}\t{}", note.id, note.from, note.to, note.text);
                }
            }
            Ok(())
        }
        Some("add") => {
            if rest.len() < 2 {
                return Err(io::Error::new(ErrorKind::InvalidInput, "missing message"));
            }
            let note = store.add(&rest[1..].join(" "))?;
            println!("{}", note.id);
            Ok(())
        }
        Some("update") => {
            if rest.len() < 3 {
                return Err(io::Error::new(ErrorKind::InvalidInput, "missing message"));
            }
            let id = parse_id(&rest[1])?;
            store.update(id, &rest[2..].join(" "))
        }
        Some("delete") => {
            if rest.len() != 2 {
                return Err(io::Error::new(ErrorKind::InvalidInput, "missing id"));
            }
            store.delete(parse_id(&rest[1])?)
        }
        Some(other) => Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("unknown command {other}"),
        )),
    }
}

fn parse_id(text: &str) -> io::Result<u32> {
    text.parse()
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "bad id"))
}

fn config_path(args: &[String]) -> PathBuf {
    flag_str(args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(settings::default_path)
}

fn without_config(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if arg == "--config" {
            skip = true;
            continue;
        }
        out.push(arg.clone());
    }
    out
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

fn arm_debug(args: &[String]) {
    let env = std::env::var_os("BEZEL_DEBUG").is_some_and(|value| value != "0");
    if flag_present(args, "--debug") || env {
        bezel::trace::set(true);
    }
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
bezel --to [{DEFAULT_MAC}] [--port {DEFAULT_PORT}] [--scale 1.0] [--edge left] [--debug] [--no-upgrade]
bezel listen [--port {DEFAULT_PORT}] [--allow {DEFAULT_WINDOWS}] [--debug] [--no-upgrade]
bezel who
bezel inbox
bezel inbox add <text>
bezel inbox update <id> <text>
bezel inbox delete <id>
bezel report <text>
bezel upgrade
bezel update
bezel fake [--port {DEFAULT_PORT}]

Settings: {path}
Flags override the file until the file is saved again. Windows sends the edge on the connection.
A newer release is printed. bezel upgrade installs it. --no-upgrade stays quiet.
bezel who prints the stable name of this machine. bezel inbox changes the shared messages.
Windows and the Mac each file a GitHub issue when a problem happens. bezel report files one now.
--debug prints each key and whether it was sent. BEZEL_DEBUG=1 does the same. The lines stay on the console.
bezel update sends this tree to the Mac, which builds it and restarts listen.

Ctrl becomes Command. Alt becomes Option. Shift stays Shift.
The escape chord is Ctrl+Alt+Shift."
    );
}
