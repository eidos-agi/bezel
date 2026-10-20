//! Send this tree to the Mac. The Mac builds it and restarts `bezel listen`.
//! The bytes ride the same Tailscale connection as the pointer.

use crate::net;
use std::fs;
use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub const MAGIC: &[u8; 4] = b"BEZU";

pub fn push(host: &str, port: u16, root: &Path) -> io::Result<String> {
    let packed = pack(root)?;
    let addrs = net::resolve(host, port)?;
    let addr = net::prefer_tailscale(&addrs).ok_or_else(|| {
        io::Error::new(ErrorKind::NotFound, format!("no address for {host}"))
    })?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(600)))?;
    if let Err(err) = stream
        .write_all(&packed)
        .and_then(|_| stream.shutdown(std::net::Shutdown::Write))
    {
        return Err(old_listener_or(err));
    }
    let mut reply = String::new();
    if let Err(err) = stream.read_to_string(&mut reply) {
        if reply.is_empty() {
            return Err(old_listener_or(err));
        }
    }
    if reply.trim().is_empty() {
        return Err(old_listener());
    }
    Ok(reply)
}

fn old_listener() -> io::Error {
    io::Error::new(
        ErrorKind::ConnectionReset,
        "the Mac closed the connection. It is running a bezel that cannot take an update yet",
    )
}

fn old_listener_or(err: io::Error) -> io::Error {
    match err.kind() {
        ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::UnexpectedEof
        | ErrorKind::BrokenPipe => old_listener(),
        _ => err,
    }
}

pub fn find_root() -> io::Result<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            starts.push(dir.to_path_buf());
        }
    }
    for start in starts {
        let mut dir = start;
        loop {
            let manifest = dir.join("Cargo.toml");
            if manifest.is_file()
                && fs::read_to_string(&manifest)
                    .map(|text| text.contains("name = \"bezel\""))
                    .unwrap_or(false)
            {
                return Ok(dir);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    Err(io::Error::new(
        ErrorKind::NotFound,
        "could not find the bezel tree",
    ))
}

pub fn pack(root: &Path) -> io::Result<Vec<u8>> {
    let mut files = Vec::new();
    for name in ["Cargo.toml", "Cargo.lock", "README.md", ".gitignore"] {
        let path = root.join(name);
        if path.is_file() {
            files.push((name.to_string(), fs::read(&path)?));
        }
    }
    let src = root.join("src");
    if src.is_dir() {
        for entry in fs::read_dir(&src)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let name = format!("src/{}", entry.file_name().to_string_lossy());
            files.push((name, fs::read(&path)?));
        }
    }
    if files.is_empty() {
        return Err(io::Error::new(ErrorKind::NotFound, "no source files"));
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, bytes) in files {
        check_name(&name).map_err(|err| io::Error::new(ErrorKind::InvalidInput, err))?;
        let len = u16::try_from(name.len())
            .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "file name is too long"))?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

pub fn unpack(bytes: &[u8], dest: &Path) -> io::Result<()> {
    let mut rest = bytes;
    let magic = read_exact(&mut rest, 4)?;
    if magic != MAGIC {
        return Err(io::Error::new(ErrorKind::InvalidData, "not an update"));
    }
    let count = u32::from_le_bytes(read_exact(&mut rest, 4)?.try_into().unwrap());
    if count > 64 {
        return Err(io::Error::new(ErrorKind::InvalidData, "too many files"));
    }
    for _ in 0..count {
        let name_len = u16::from_le_bytes(read_exact(&mut rest, 2)?.try_into().unwrap()) as usize;
        let name = String::from_utf8(read_exact(&mut rest, name_len)?.to_vec())
            .map_err(|_| io::Error::new(ErrorKind::InvalidData, "file name"))?;
        check_name(&name).map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
        let size = u32::from_le_bytes(read_exact(&mut rest, 4)?.try_into().unwrap()) as usize;
        if size > 1_048_576 {
            return Err(io::Error::new(ErrorKind::InvalidData, "file is too large"));
        }
        let data = read_exact(&mut rest, size)?;
        let path = dest.join(&name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, data)?;
    }
    if !rest.is_empty() {
        return Err(io::Error::new(ErrorKind::InvalidData, "trailing update bytes"));
    }
    Ok(())
}

pub fn take(stream: &mut TcpStream, root: &Path) -> io::Result<()> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    unpack(&bytes, root)?;
    let status = std::process::Command::new(cargo_program())
        .current_dir(root)
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .status()?;
    if !status.success() {
        return Err(io::Error::new(ErrorKind::Other, "cargo build failed"));
    }
    Ok(())
}

pub fn restart(root: &Path) -> io::Result<()> {
    let built = root.join("target").join("release").join(binary_name());
    let program = if built.is_file() {
        built
    } else {
        std::env::current_exe()?
    };
    std::process::Command::new(&program)
        .arg("listen")
        .env("BEZEL_RESTART", "1")
        .spawn()?;
    std::process::exit(0);
}

fn cargo_program() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let name = if cfg!(windows) { "cargo.exe" } else { "cargo" };
        let candidate = PathBuf::from(home).join(".cargo").join("bin").join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("cargo")
}

pub fn source_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let maybe = dir.parent().and_then(|target| {
                if target.file_name().and_then(|name| name.to_str()) == Some("target") {
                    target.parent().map(Path::to_path_buf)
                } else {
                    None
                }
            });
            if let Some(root) = maybe {
                if root.join("Cargo.toml").is_file() {
                    return root;
                }
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("bezel");
    }
    PathBuf::from("bezel")
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "bezel.exe"
    } else {
        "bezel"
    }
}

fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty file name".to_string());
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return Err(format!("refusing absolute path {name}"));
    }
    let mut depth = 0i32;
    for part in path.components() {
        match part {
            Component::Normal(text) => {
                let text = text.to_string_lossy();
                if text.contains(':') || text.contains('\\') {
                    return Err(format!("refusing {name}"));
                }
                depth += 1;
            }
            Component::CurDir => {}
            _ => return Err(format!("refusing {name}")),
        }
    }
    if depth == 0 {
        return Err(format!("refusing {name}"));
    }
    Ok(())
}

fn read_exact<'a>(rest: &mut &'a [u8], len: usize) -> io::Result<&'a [u8]> {
    if rest.len() < len {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "short update"));
    }
    let (head, tail) = rest.split_at(len);
    *rest = tail;
    Ok(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_round_trips_and_parent_paths_are_refused() {
        let root = std::env::temp_dir().join(format!("bezel-pack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Cargo.toml"), "name = \"bezel\"\n").unwrap();
        fs::write(root.join("src").join("lib.rs"), "fn main() {}\n").unwrap();
        let packed = pack(&root).unwrap();
        let dest = std::env::temp_dir().join(format!("bezel-unpack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dest);
        unpack(&packed, &dest).unwrap();
        let text = fs::read_to_string(dest.join("src").join("lib.rs")).unwrap();
        assert!(text.contains("fn main"));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&dest);

        let mut bad = Vec::new();
        bad.extend_from_slice(MAGIC);
        bad.extend_from_slice(&1u32.to_le_bytes());
        let name = b"../secret";
        bad.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bad.extend_from_slice(name);
        bad.extend_from_slice(&0u32.to_le_bytes());
        assert!(unpack(&bad, &std::env::temp_dir()).is_err());
    }

    #[test]
    fn a_closed_peer_reports_an_old_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let root = std::env::temp_dir().join(format!("bezel-push-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Cargo.toml"), "name = \"bezel\"\n").unwrap();
        fs::write(root.join("src").join("lib.rs"), "fn main() {}\n").unwrap();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            let _ = std::io::Read::read(&mut sock, &mut buf);
        });
        let err = push("127.0.0.1", port, &root).unwrap_err();
        assert!(
            err.to_string().contains("cannot take an update yet"),
            "{err}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
