use bezel::desk::Edge;
use bezel::inbox::{self, Note};
use bezel::settings::{self, Settings};
use bezel::{fake, who, DEFAULT_PORT};
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
        Some("host") => bezel::host::print(),
        Some("status") => {
            let path = config_path(&args[1..]);
            bezel::status::show(&path)
        }
        Some("install") => bezel::install::install(),
        Some("version") => {
            println!("{}", bezel::upgrade::current_version());
            Ok(())
        }
        Some("doctor") => doctor_cmd(&args[1..]),
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
            bezel::report::configure(&settings.report);
            if settings.host.is_empty() {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    "host is unset. Run bezel host on the other machine and write that name here.",
                ));
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
    arm_debug(args);
    let (settings, path) = load_settings(args)?;
    bezel::report::configure(&settings.report);
    if settings.allow.is_empty() {
        eprintln!("bezel: allow is unset. Run bezel host on Windows and write that name here.");
    }
    let upgrade = !flag_present(args, "--no-upgrade");
    #[cfg(target_os = "macos")]
    {
        bezel::mac::listen(settings.port, &settings.allow, upgrade, &path)
    }
    #[cfg(target_os = "linux")]
    {
        bezel::linux::listen(settings.port, &settings.allow, upgrade, &path)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (settings, path, upgrade);
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "bezel listen runs on the Mac or on Linux",
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

fn doctor_cmd(args: &[String]) -> io::Result<()> {
    let path = config_path(args);
    let rest = without_config(args);
    match rest.first().map(String::as_str) {
        Some("add") => {
            if rest.len() < 2 {
                return Err(io::Error::new(ErrorKind::InvalidInput, "missing message"));
            }
            let note = bezel::doctor::add_message(&path, &rest[1..].join(" "))?;
            println!("{}", note.id);
            if note.to.is_empty() {
                println!("saved. The running bezel sends it on the open connection.");
            } else {
                println!(
                    "saved for {}. The running bezel sends it on the open connection.",
                    note.to
                );
            }
            Ok(())
        }
        Some("update") => {
            if rest.len() < 3 {
                return Err(io::Error::new(ErrorKind::InvalidInput, "missing message"));
            }
            let id = parse_id(&rest[1])?;
            bezel::doctor::update_message(&path, id, &rest[2..].join(" "))?;
            println!("updated {id}");
            Ok(())
        }
        Some("delete") => {
            if rest.len() != 2 {
                return Err(io::Error::new(ErrorKind::InvalidInput, "missing id"));
            }
            let id = parse_id(&rest[1])?;
            bezel::doctor::delete_message(&path, id)?;
            println!("deleted {id}");
            Ok(())
        }
        _ => {
            let mut settings = settings::load(&path)?;
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
            let messages = bezel::doctor::read_messages(&path)
                .unwrap_or_else(|err| format!("messages: {err}"));
            println!("{}\n{messages}", bezel::doctor::checkup(&settings, &path));
            Ok(())
        }
    }
}

fn report_cmd(args: &[String]) -> io::Result<()> {
    let (settings, _) = load_settings(args)?;
    bezel::report::configure(&settings.report);
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
bezel --to [host] [--port {DEFAULT_PORT}] [--scale 1.0] [--edge left] [--debug] [--no-upgrade]
bezel listen [--port {DEFAULT_PORT}] [--allow host] [--debug] [--no-upgrade]
bezel who
bezel host
bezel status
bezel install
bezel version
bezel doctor
bezel doctor add <text>
bezel doctor update <id> <text>
bezel doctor delete <id>
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
bezel who prints the stable name of this machine. bezel host prints its MagicDNS name.
bezel status prints what the running copy last recorded. It does not move the pointer or open a connection.
bezel install writes the service and starts it when nothing is running. The operating system still asks once.
bezel version prints this release.
bezel doctor says who is connected and whether a move off the chosen edge could hand the pointer over. It does not move the pointer or open the connection.
bezel doctor add, update, and delete change the shared messages. The running bezel sends them. bezel doctor lists them.
bezel inbox changes the same messages.
bezel report files a problem only when settings set report to owner/name and a GitHub token is present. Otherwise it stays quiet.
--debug prints each key and whether it was sent. BEZEL_DEBUG=1 does the same. The lines stay on the console.
bezel update sends this tree to the Mac, which builds it and restarts listen.

Ctrl becomes Command. Alt becomes Option. Shift stays Shift.
The escape chord is Ctrl+Alt+Shift."
    );
}
