//! A stand-in for the Mac. It acks Enter and records frames. It injects nothing.

use crate::frame::{self, Frame, Reader};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub logical_w: u16,
    pub logical_h: u16,
    pub scale_milli: u16,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            logical_w: 1800,
            logical_h: 1169,
            scale_milli: 2000,
        }
    }
}

pub fn answer(frame: Frame, screen: Screen) -> Option<Frame> {
    match frame {
        Frame::Hello { .. } => Some(Frame::HelloAck {
            logical_w: screen.logical_w,
            logical_h: screen.logical_h,
            scale_milli: screen.scale_milli,
        }),
        Frame::Enter { seq, .. } => Some(Frame::EnterAck { seq }),
        Frame::Heartbeat => Some(Frame::Heartbeat),
        _ => None,
    }
}

pub fn serve(listener: TcpListener, screen: Screen) -> io::Result<()> {
    eprintln!(
        "bezel: fake listening on {}",
        listener.local_addr()?
    );
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let seen = serve_one(stream, screen)?;
                eprintln!("bezel: fake saw {}", summarize(&seen));
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

pub fn serve_one(mut stream: TcpStream, screen: Screen) -> io::Result<Vec<Frame>> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_millis(400)))?;
    let mut reader = Reader::new();
    let mut seen = Vec::new();
    let mut buf = [0u8; 1024];
    let mut idle = 0u32;
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return Ok(seen),
            Ok(n) => {
                idle = 0;
                let frames = reader.push(&buf[..n]).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "bad frame")
                })?;
                for frame in frames {
                    if let Some(reply) = answer(frame, screen) {
                        let mut bytes = Vec::new();
                        frame::encode(&reply, &mut bytes);
                        stream.write_all(&bytes)?;
                    }
                    let stop = matches!(frame, Frame::Release);
                    seen.push(frame);
                    if stop {
                        return Ok(seen);
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::TimedOut => {
                idle += 1;
                if idle > 20 {
                    return Ok(seen);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
}

fn summarize(frames: &[Frame]) -> String {
    let mut parts = Vec::new();
    for frame in frames {
        let name = match frame {
            Frame::Hello { .. } => "hello",
            Frame::Enter { .. } => "enter",
            Frame::RelMove { .. } => "move",
            Frame::Button { .. } => "button",
            Frame::Wheel { .. } => "wheel",
            Frame::Key { .. } => "key",
            Frame::Heartbeat => "heartbeat",
            Frame::Release => "release",
            Frame::Leave => "leave",
            _ => "other",
        };
        if parts.last().copied() != Some(name) {
            parts.push(name);
        }
    }
    if parts.is_empty() {
        "nothing".to_string()
    } else {
        parts.join(" ")
    }
}

pub fn listen(port: u16) -> io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    serve(listener, Screen::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{decode, encode, VERSION};
    use std::thread;

    #[test]
    fn fake_peer_acks_enter_and_records_a_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let joined = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_one(stream, Screen::default()).unwrap()
        });

        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_nodelay(true).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        write_frame(
            &mut stream,
            &Frame::Hello { version: VERSION },
        );
        let ack = read_frame(&mut stream);
        assert!(matches!(
            ack,
            Frame::HelloAck {
                logical_w: 1800,
                logical_h: 1169,
                scale_milli: 2000,
            }
        ));
        write_frame(
            &mut stream,
            &Frame::Enter {
                seq: 3,
                y: 80,
                span: 1200,
            },
        );
        assert_eq!(read_frame(&mut stream), Frame::EnterAck { seq: 3 });
        write_frame(
            &mut stream,
            &Frame::Key {
                hid: 0xE1,
                down: true,
            },
        );
        write_frame(&mut stream, &Frame::Release);
        let seen = joined.join().unwrap();
        assert!(seen.contains(&Frame::Key {
            hid: 0xE1,
            down: true,
        }));
        assert!(seen.contains(&Frame::Release));
    }

    fn write_frame(stream: &mut TcpStream, frame: &Frame) {
        let mut bytes = Vec::new();
        encode(frame, &mut bytes);
        stream.write_all(&bytes).unwrap();
    }

    fn read_frame(stream: &mut TcpStream) -> Frame {
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).unwrap();
        decode(&buf[..n]).unwrap().unwrap().0
    }
}
