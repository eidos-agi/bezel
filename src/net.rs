//! One TCP connection with Nagle off. The hook thread never calls this.

use crate::frame::{self, Frame, Reader};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};

pub fn resolve(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    let mut addrs: Vec<SocketAddr> = (host, port).to_socket_addrs()?.collect();
    addrs.sort();
    addrs.dedup();
    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no address for {host}"),
        ));
    }
    Ok(addrs)
}

/// Prefer a Tailscale CGNAT address when a name resolves to more than one.
pub fn prefer_tailscale(addrs: &[SocketAddr]) -> Option<SocketAddr> {
    addrs
        .iter()
        .copied()
        .find(|addr| matches!(addr.ip(), IpAddr::V4(ip) if is_cgnat(ip)))
        .or_else(|| addrs.first().copied())
}

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0xc0) == 0x40
}

pub struct Link {
    stream: TcpStream,
    reader: Reader,
    pending: Vec<u8>,
}

impl Link {
    pub fn new(stream: TcpStream) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            reader: Reader::new(),
            pending: Vec::new(),
        })
    }

    pub fn send(&mut self, frame: &Frame) -> io::Result<()> {
        frame::encode(frame, &mut self.pending);
        self.flush()
    }

    pub fn flush(&mut self) -> io::Result<()> {
        while !self.pending.is_empty() {
            match self.stream.write(&self.pending) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "connection closed",
                    ))
                }
                Ok(n) => {
                    self.pending.drain(..n);
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    pub fn recv(&mut self) -> io::Result<Vec<Frame>> {
        let mut tmp = [0u8; 2048];
        let mut out = Vec::new();
        loop {
            match self.stream.read(&mut tmp) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed",
                    ))
                }
                Ok(n) => {
                    let frames = self.reader.push(&tmp[..n]).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "bad frame")
                    })?;
                    out.extend(frames);
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(out),
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
    }
}
