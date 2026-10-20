use bezel::{fake, DEFAULT_MAC, DEFAULT_PORT, DEFAULT_WINDOWS};
use std::env;
use std::io::{self, ErrorKind};
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
        Some("fake") => {
            let port = flag_u16(&args[1..], "--port")?.unwrap_or(DEFAULT_PORT);
            fake::listen(port)
        }
        Some("--to") => {
            let host = args.get(1).map(String::as_str).unwrap_or(DEFAULT_MAC);
            capture(host, &args[2..])
        }
        Some(other) => Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("unknown command {other}"),
        )),
    }
}

fn capture(host: &str, args: &[String]) -> io::Result<()> {
    let port = flag_u16(args, "--port")?.unwrap_or(DEFAULT_PORT);
    let scale = flag_f64(args, "--scale")?.unwrap_or(1.0);
    #[cfg(windows)]
    {
        bezel::win::run(host, port, scale)
    }
    #[cfg(not(windows))]
    {
        let _ = (host, port, scale);
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "bezel --to runs on Windows",
        ))
    }
}

fn listen(args: &[String]) -> io::Result<()> {
    let port = flag_u16(args, "--port")?.unwrap_or(DEFAULT_PORT);
    let allow = flag_str(args, "--allow").unwrap_or(DEFAULT_WINDOWS);
    #[cfg(target_os = "macos")]
    {
        bezel::mac::listen(port, allow)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (port, allow);
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "bezel listen runs on the Mac",
        ))
    }
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
    println!(
        "\
bezel --to {DEFAULT_MAC} [--port {DEFAULT_PORT}] [--scale 1.0]
bezel listen [--port {DEFAULT_PORT}] [--allow {DEFAULT_WINDOWS}]
bezel fake [--port {DEFAULT_PORT}]

Ctrl becomes Command. Alt becomes Option. Shift stays Shift.
The escape chord is Ctrl+Alt+Shift."
    );
}
