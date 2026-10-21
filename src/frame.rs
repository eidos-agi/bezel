//! Length-prefixed frames. The type byte is original to Bezel.

use crate::desk::Edge;
use std::convert::TryInto;

pub const VERSION: u8 = 1;
/// Pointer frames stay small. A note needs room for text. An old peer still
/// rejects a body longer than 32, so a note is sent only after that peer asks.
const MAX_BODY: usize = 512;
pub const MAX_NOTE_TEXT: usize = 320;
pub const CAP_CONFIG: u8 = 1;
pub const CAP_INBOX: u8 = 2;

const HELLO: u8 = 1;
const HELLO_ACK: u8 = 2;
const ENTER: u8 = 3;
const ENTER_ACK: u8 = 4;
const LEAVE: u8 = 5;
const REL_MOVE: u8 = 6;
const BUTTON: u8 = 7;
const WHEEL: u8 = 8;
const KEY: u8 = 9;
const HEARTBEAT: u8 = 10;
const RELEASE: u8 = 11;
const CONFIG: u8 = 12;
const NOTE: u8 = 13;
const INBOX_HELLO: u8 = 14;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Hello {
        version: u8,
    },
    HelloAck {
        logical_w: u16,
        logical_h: u16,
        scale_milli: u16,
        /// The Mac applies an edge sent by Windows. An old listener omits this.
        accepts_config: bool,
        /// The peer can create, read, update, and delete shared messages.
        accepts_inbox: bool,
    },
    /// One machine tells the other its stable name, then the notes may flow.
    InboxHello {
        name: String,
    },
    Note {
        id: u32,
        rev: u32,
        deleted: bool,
        from: String,
        to: String,
        text: String,
    },
    /// Windows tells the Mac which edge faces it. A later frame replaces it.
    Config {
        edge: Edge,
    },
    Enter {
        seq: u32,
        y: u16,
        span: u16,
    },
    EnterAck {
        seq: u32,
    },
    Leave,
    RelMove {
        dx: i16,
        dy: i16,
    },
    Button {
        button: u8,
        down: bool,
    },
    Wheel {
        dx: i16,
        dy: i16,
    },
    Key {
        hid: u16,
        down: bool,
    },
    Heartbeat,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeError;

pub struct Reader {
    buf: Vec<u8>,
}

impl Reader {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, DecodeError> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            match decode(&self.buf)? {
                Some((frame, used)) => {
                    self.buf.drain(..used);
                    out.push(frame);
                }
                None => break,
            }
        }
        Ok(out)
    }
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

pub fn encode(frame: &Frame, out: &mut Vec<u8>) {
    let mut body = Vec::with_capacity(16);
    match frame {
        Frame::Hello { version } => {
            body.push(HELLO);
            body.push(*version);
        }
        Frame::HelloAck {
            logical_w,
            logical_h,
            scale_milli,
            accepts_config,
            accepts_inbox,
        } => {
            body.push(HELLO_ACK);
            body.extend_from_slice(&logical_w.to_le_bytes());
            body.extend_from_slice(&logical_h.to_le_bytes());
            body.extend_from_slice(&scale_milli.to_le_bytes());
            let mut caps = 0u8;
            if *accepts_config {
                caps |= CAP_CONFIG;
            }
            if *accepts_inbox {
                caps |= CAP_INBOX;
            }
            if caps != 0 {
                body.push(caps);
            }
        }
        Frame::InboxHello { name } => {
            body.push(INBOX_HELLO);
            body.extend_from_slice(name.as_bytes());
        }
        Frame::Note {
            id,
            rev,
            deleted,
            from,
            to,
            text,
        } => {
            body.push(NOTE);
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&rev.to_le_bytes());
            body.push(u8::from(*deleted));
            push_counted(&mut body, from);
            push_counted(&mut body, to);
            body.extend_from_slice(text.as_bytes());
        }
        Frame::Config { edge } => {
            body.push(CONFIG);
            body.push(edge.wire());
        }
        Frame::Enter { seq, y, span } => {
            body.push(ENTER);
            body.extend_from_slice(&seq.to_le_bytes());
            body.extend_from_slice(&y.to_le_bytes());
            body.extend_from_slice(&span.to_le_bytes());
        }
        Frame::EnterAck { seq } => {
            body.push(ENTER_ACK);
            body.extend_from_slice(&seq.to_le_bytes());
        }
        Frame::Leave => body.push(LEAVE),
        Frame::RelMove { dx, dy } => {
            body.push(REL_MOVE);
            body.extend_from_slice(&dx.to_le_bytes());
            body.extend_from_slice(&dy.to_le_bytes());
        }
        Frame::Button { button, down } => {
            body.push(BUTTON);
            body.push(*button);
            body.push(u8::from(*down));
        }
        Frame::Wheel { dx, dy } => {
            body.push(WHEEL);
            body.extend_from_slice(&dx.to_le_bytes());
            body.extend_from_slice(&dy.to_le_bytes());
        }
        Frame::Key { hid, down } => {
            body.push(KEY);
            body.extend_from_slice(&hid.to_le_bytes());
            body.push(u8::from(*down));
        }
        Frame::Heartbeat => body.push(HEARTBEAT),
        Frame::Release => body.push(RELEASE),
    }
    let len = (body.len() as u32).to_le_bytes();
    out.extend_from_slice(&len);
    out.extend_from_slice(&body);
}

pub fn decode(buf: &[u8]) -> Result<Option<(Frame, usize)>, DecodeError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    if len == 0 || len > MAX_BODY {
        return Err(DecodeError);
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let body = &buf[4..4 + len];
    let frame = decode_body(body)?;
    Ok(Some((frame, 4 + len)))
}

fn decode_body(body: &[u8]) -> Result<Frame, DecodeError> {
    let kind = *body.first().ok_or(DecodeError)?;
    let rest = &body[1..];
    match kind {
        HELLO => {
            let version = *rest.first().ok_or(DecodeError)?;
            if rest.len() != 1 {
                return Err(DecodeError);
            }
            Ok(Frame::Hello { version })
        }
        HELLO_ACK => {
            if rest.len() != 6 && rest.len() != 7 {
                return Err(DecodeError);
            }
            let caps = if rest.len() == 7 { rest[6] } else { 0 };
            Ok(Frame::HelloAck {
                logical_w: u16::from_le_bytes(rest[0..2].try_into().unwrap()),
                logical_h: u16::from_le_bytes(rest[2..4].try_into().unwrap()),
                scale_milli: u16::from_le_bytes(rest[4..6].try_into().unwrap()),
                accepts_config: caps & CAP_CONFIG != 0,
                accepts_inbox: caps & CAP_INBOX != 0,
            })
        }
        INBOX_HELLO => {
            if rest.is_empty() || rest.len() > 64 {
                return Err(DecodeError);
            }
            let name = std::str::from_utf8(rest).map_err(|_| DecodeError)?;
            if !plain_name(name) {
                return Err(DecodeError);
            }
            Ok(Frame::InboxHello {
                name: name.to_string(),
            })
        }
        NOTE => decode_note(rest),
        ENTER => {
            if rest.len() != 8 {
                return Err(DecodeError);
            }
            Ok(Frame::Enter {
                seq: u32::from_le_bytes(rest[0..4].try_into().unwrap()),
                y: u16::from_le_bytes(rest[4..6].try_into().unwrap()),
                span: u16::from_le_bytes(rest[6..8].try_into().unwrap()),
            })
        }
        ENTER_ACK => {
            if rest.len() != 4 {
                return Err(DecodeError);
            }
            Ok(Frame::EnterAck {
                seq: u32::from_le_bytes(rest[0..4].try_into().unwrap()),
            })
        }
        LEAVE => empty(rest, Frame::Leave),
        REL_MOVE => pair_i16(rest).map(|(dx, dy)| Frame::RelMove { dx, dy }),
        BUTTON => {
            if rest.len() != 2 {
                return Err(DecodeError);
            }
            Ok(Frame::Button {
                button: rest[0],
                down: rest[1] != 0,
            })
        }
        WHEEL => pair_i16(rest).map(|(dx, dy)| Frame::Wheel { dx, dy }),
        KEY => {
            if rest.len() != 3 {
                return Err(DecodeError);
            }
            Ok(Frame::Key {
                hid: u16::from_le_bytes(rest[0..2].try_into().unwrap()),
                down: rest[2] != 0,
            })
        }
        HEARTBEAT => empty(rest, Frame::Heartbeat),
        RELEASE => empty(rest, Frame::Release),
        CONFIG => {
            if rest.len() != 1 {
                return Err(DecodeError);
            }
            let edge = Edge::from_wire(rest[0]).ok_or(DecodeError)?;
            Ok(Frame::Config { edge })
        }
        _ => Err(DecodeError),
    }
}

fn decode_note(rest: &[u8]) -> Result<Frame, DecodeError> {
    if rest.len() < 11 {
        return Err(DecodeError);
    }
    let id = u32::from_le_bytes(rest[0..4].try_into().unwrap());
    let rev = u32::from_le_bytes(rest[4..8].try_into().unwrap());
    let deleted = rest[8];
    if id == 0 || rev == 0 || deleted > 1 {
        return Err(DecodeError);
    }
    let (from, rest) = take_counted(&rest[9..])?;
    if !plain_name(from) {
        return Err(DecodeError);
    }
    let (to, text_bytes) = take_counted(rest)?;
    if !to.is_empty() && !plain_name(to) {
        return Err(DecodeError);
    }
    let text = std::str::from_utf8(text_bytes).map_err(|_| DecodeError)?;
    if text.len() > MAX_NOTE_TEXT || text.chars().any(|c| c == '\t' || c == '\n' || c == '\r') {
        return Err(DecodeError);
    }
    Ok(Frame::Note {
        id,
        rev,
        deleted: deleted == 1,
        from: from.to_string(),
        to: to.to_string(),
        text: text.to_string(),
    })
}

fn push_counted(body: &mut Vec<u8>, text: &str) {
    let bytes = text.as_bytes();
    let n = bytes.len().min(64);
    body.push(n as u8);
    body.extend_from_slice(&bytes[..n]);
}

fn take_counted(buf: &[u8]) -> Result<(&str, &[u8]), DecodeError> {
    let n = *buf.first().ok_or(DecodeError)? as usize;
    if n > 64 || buf.len() < 1 + n {
        return Err(DecodeError);
    }
    let text = std::str::from_utf8(&buf[1..1 + n]).map_err(|_| DecodeError)?;
    Ok((text, &buf[1 + n..]))
}

fn plain_name(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    text.chars()
        .all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '-' || c == '\'')
        && !text.ends_with(' ')
        && !text.contains("  ")
}

fn empty(rest: &[u8], frame: Frame) -> Result<Frame, DecodeError> {
    if rest.is_empty() {
        Ok(frame)
    } else {
        Err(DecodeError)
    }
}

fn pair_i16(rest: &[u8]) -> Result<(i16, i16), DecodeError> {
    if rest.len() != 4 {
        return Err(DecodeError);
    }
    Ok((
        i16::from_le_bytes(rest[0..2].try_into().unwrap()),
        i16::from_le_bytes(rest[2..4].try_into().unwrap()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(frame: Frame) {
        let mut bytes = Vec::new();
        encode(&frame, &mut bytes);
        let (got, used) = decode(&bytes).unwrap().unwrap();
        assert_eq!(used, bytes.len());
        assert_eq!(got, frame);
    }

    #[test]
    fn every_frame_round_trips() {
        let frames = [
            Frame::Hello { version: VERSION },
            Frame::HelloAck {
                logical_w: 1800,
                logical_h: 1169,
                scale_milli: 2000,
                accepts_config: true,
                accepts_inbox: true,
            },
            Frame::InboxHello {
                name: "Ada West".into(),
            },
            Frame::Note {
                id: 9,
                rev: 2,
                deleted: false,
                from: "Ada West".into(),
                to: "Bea North".into(),
                text: "buy milk".into(),
            },
            Frame::Config { edge: Edge::Left },
            Frame::Enter {
                seq: 7,
                y: 400,
                span: 1200,
            },
            Frame::EnterAck { seq: 7 },
            Frame::Leave,
            Frame::RelMove { dx: -4, dy: 9 },
            Frame::Button {
                button: 0,
                down: true,
            },
            Frame::Wheel { dx: 0, dy: -1 },
            Frame::Key {
                hid: 0xE1,
                down: false,
            },
            Frame::Heartbeat,
            Frame::Release,
        ];
        for frame in frames {
            round_trip(frame);
        }
    }

    #[test]
    fn truncated_frame_waits() {
        let mut bytes = Vec::new();
        encode(&Frame::Heartbeat, &mut bytes);
        assert!(decode(&bytes[..3]).unwrap().is_none());
        let mut reader = Reader::new();
        assert!(reader.push(&bytes[..3]).unwrap().is_empty());
        let got = reader.push(&bytes[3..]).unwrap();
        assert_eq!(got, vec![Frame::Heartbeat]);
    }

    #[test]
    fn an_old_hello_ack_cannot_take_an_edge() {
        let mut body = vec![HELLO_ACK];
        body.extend_from_slice(&1800u16.to_le_bytes());
        body.extend_from_slice(&1169u16.to_le_bytes());
        body.extend_from_slice(&1000u16.to_le_bytes());
        let mut buf = Vec::new();
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);
        let (frame, _) = decode(&buf).unwrap().unwrap();
        assert_eq!(
            frame,
            Frame::HelloAck {
                logical_w: 1800,
                logical_h: 1169,
                scale_milli: 1000,
                accepts_config: false,
                accepts_inbox: false,
            }
        );
    }

    #[test]
    fn capability_bits_keep_an_old_config_byte_working() {
        match hello_ack_with_caps(CAP_CONFIG) {
            Frame::HelloAck {
                accepts_config: true,
                accepts_inbox: false,
                ..
            } => {}
            other => panic!("config bit {other:?}"),
        }
        match hello_ack_with_caps(CAP_CONFIG | CAP_INBOX) {
            Frame::HelloAck {
                accepts_config: true,
                accepts_inbox: true,
                ..
            } => {}
            other => panic!("both bits {other:?}"),
        }
        match hello_ack_with_caps(CAP_INBOX) {
            Frame::HelloAck {
                accepts_config: false,
                accepts_inbox: true,
                ..
            } => {}
            other => panic!("inbox bit {other:?}"),
        }
    }

    fn hello_ack_with_caps(caps: u8) -> Frame {
        let mut body = vec![HELLO_ACK];
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&1000u16.to_le_bytes());
        body.push(caps);
        let mut buf = Vec::new();
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);
        decode(&buf).unwrap().unwrap().0
    }

    #[test]
    fn unknown_type_is_an_error() {
        let buf = [2, 0, 0, 0, 99, 0];
        assert_eq!(decode(&buf), Err(DecodeError));
    }
}
