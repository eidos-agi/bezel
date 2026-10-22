//! A shared inbox. The file is the store, so a machine can change it while the
//! other side is down. The open connection merges by revision.

use crate::frame::{self, Frame};
use crate::who;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub id: u32,
    pub rev: u32,
    pub deleted: bool,
    pub from: String,
    pub to: String,
    pub text: String,
}

pub struct Store {
    path: PathBuf,
    notes: Vec<Note>,
    sent: BTreeMap<u32, u32>,
    peer: String,
    said: bool,
    pushed: bool,
}

impl Store {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let mut store = Self::blank(path);
        if store.path.exists() {
            store.reload()?;
        }
        Ok(store)
    }

    /// The running capture keeps going when the file is unreadable.
    pub fn attach(path: PathBuf) -> Self {
        match Self::open(path.clone()) {
            Ok(store) => store,
            Err(err) => {
                eprintln!("bezel: inbox ignored ({err})");
                Self::blank(path)
            }
        }
    }

    pub fn blank(path: PathBuf) -> Self {
        Self {
            path,
            notes: Vec::new(),
            sent: BTreeMap::new(),
            peer: String::new(),
            said: false,
            pushed: false,
        }
    }

    pub fn has_said(&self) -> bool {
        self.said
    }

    pub fn pause(&mut self) {
        self.said = false;
        self.pushed = false;
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// Who the next new message is for. The running capture learns this from
    /// the other machine. A one-shot command can set it from the inbox.
    pub fn set_peer(&mut self, name: &str) {
        if !name.is_empty() {
            self.peer = name.to_string();
        }
    }

    pub fn greet(&mut self) -> Frame {
        self.sent.clear();
        self.said = true;
        self.pushed = false;
        Frame::InboxHello {
            name: who::name(),
        }
    }

    pub fn observe(&mut self, frame: Frame) -> io::Result<Vec<Frame>> {
        match frame {
            Frame::InboxHello { name } => {
                self.learn(&name)?;
                if self.pushed {
                    return Ok(Vec::new());
                }
                self.pushed = true;
                Ok(self.take_pending())
            }
            Frame::Note {
                id,
                rev,
                deleted,
                from,
                to,
                text,
            } => {
                self.apply(Note {
                    id,
                    rev,
                    deleted,
                    from,
                    to,
                    text,
                })?;
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    pub fn poll(&mut self) -> io::Result<Vec<Frame>> {
        if self.path.as_os_str().is_empty() || !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&self.path)?;
        let notes = parse(&text)?;
        if notes == self.notes {
            return Ok(Vec::new());
        }
        self.notes = notes;
        if self.pushed {
            Ok(self.take_pending())
        } else {
            Ok(Vec::new())
        }
    }

    pub fn add(&mut self, text: &str) -> io::Result<Note> {
        let text = clean_text(text)?;
        let note = Note {
            id: fresh_id(&self.notes),
            rev: 1,
            deleted: false,
            from: who::name(),
            to: self.peer.clone(),
            text,
        };
        self.notes.push(note.clone());
        self.save()?;
        Ok(note)
    }

    pub fn update(&mut self, id: u32, text: &str) -> io::Result<()> {
        let text = clean_text(text)?;
        self.revise(id, |note| note.text = text)?;
        self.save()
    }

    pub fn delete(&mut self, id: u32) -> io::Result<()> {
        self.revise(id, |note| note.deleted = true)?;
        self.save()
    }

    fn revise(&mut self, id: u32, change: impl FnOnce(&mut Note)) -> io::Result<()> {
        let me = who::name();
        let peer = self.peer.clone();
        let note = self
            .notes
            .iter_mut()
            .find(|note| note.id == id && !note.deleted)
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "no such message"))?;
        if note.from != me {
            note.to = std::mem::take(&mut note.from);
        }
        note.from = me;
        if note.to == note.from || note.to.is_empty() {
            note.to = peer;
        }
        change(note);
        note.rev = note.rev.saturating_add(1);
        Ok(())
    }

    fn learn(&mut self, name: &str) -> io::Result<()> {
        if name.is_empty() || name == self.peer {
            return Ok(());
        }
        self.peer = name.to_string();
        let me = who::name();
        let mut changed = false;
        for note in &mut self.notes {
            if !note.deleted && note.from == me && note.to.is_empty() {
                note.to = name.to_string();
                note.rev = note.rev.saturating_add(1);
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(())
    }

    fn apply(&mut self, incoming: Note) -> io::Result<()> {
        if let Some(existing) = self.notes.iter_mut().find(|note| note.id == incoming.id) {
            if incoming.rev <= existing.rev {
                return Ok(());
            }
            *existing = incoming.clone();
        } else {
            self.notes.push(incoming.clone());
        }
        self.sent.insert(incoming.id, incoming.rev);
        self.save()
    }

    fn take_pending(&mut self) -> Vec<Frame> {
        let mut out = Vec::new();
        for note in &self.notes {
            let sent = self.sent.get(&note.id).copied().unwrap_or(0);
            if note.rev > sent {
                out.push(note.frame());
            }
        }
        for frame in &out {
            if let Frame::Note { id, rev, .. } = frame {
                self.sent.insert(*id, *rev);
            }
        }
        out
    }

    fn reload(&mut self) -> io::Result<()> {
        let text = fs::read_to_string(&self.path)?;
        self.notes = parse(&text)?;
        Ok(())
    }

    fn save(&mut self) -> io::Result<()> {
        if self.path.as_os_str().is_empty() {
            self.notes.sort_by_key(|note| note.id);
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        self.notes.sort_by_key(|note| note.id);
        fs::write(&self.path, render(&self.notes))?;
        Ok(())
    }
}

impl Note {
    fn frame(&self) -> Frame {
        Frame::Note {
            id: self.id,
            rev: self.rev,
            deleted: self.deleted,
            from: self.from.clone(),
            to: self.to.clone(),
            text: self.text.clone(),
        }
    }
}

pub fn path_for(settings: &Path) -> PathBuf {
    settings.with_file_name("inbox.txt")
}

pub fn clean_text(text: &str) -> io::Result<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(io::Error::new(ErrorKind::InvalidInput, "empty message"));
    }
    if text.len() > frame::MAX_NOTE_TEXT {
        return Err(io::Error::new(ErrorKind::InvalidInput, "message is too long"));
    }
    if text.chars().any(|c| c == '\t' || c == '\n' || c == '\r') {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "message cannot contain a tab or a new line",
        ));
    }
    Ok(text.to_string())
}

fn render(notes: &[Note]) -> String {
    let mut out = String::new();
    for note in notes {
        out.push_str(&note.id.to_string());
        out.push('\t');
        out.push_str(&note.rev.to_string());
        out.push('\t');
        out.push(if note.deleted { '1' } else { '0' });
        out.push('\t');
        out.push_str(&note.from);
        out.push('\t');
        out.push_str(&note.to);
        out.push('\t');
        out.push_str(&note.text);
        out.push('\n');
    }
    out
}

fn parse(text: &str) -> io::Result<Vec<Note>> {
    let mut notes = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(6, '\t');
        let bad = || {
            io::Error::new(
                ErrorKind::InvalidData,
                format!("bad inbox line {}", index + 1),
            )
        };
        let id: u32 = parts.next().unwrap().parse().map_err(|_| bad())?;
        let rev: u32 = parts.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
        let deleted = parts.next().ok_or_else(bad)?;
        let from = parts.next().ok_or_else(bad)?.to_string();
        let to = parts.next().ok_or_else(bad)?.to_string();
        let text = parts.next().ok_or_else(bad)?.to_string();
        if id == 0 || rev == 0 || (deleted != "0" && deleted != "1") || from.is_empty() {
            return Err(bad());
        }
        notes.push(Note {
            id,
            rev,
            deleted: deleted == "1",
            from,
            to,
            text,
        });
    }
    notes.sort_by_key(|note| note.id);
    Ok(notes)
}

fn fresh_id(notes: &[Note]) -> u32 {
    let tick = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_nanos())
        .unwrap_or(0);
    let mut n = 0u64;
    loop {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in who::machine_id().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        for byte in tick.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        for byte in n.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let id = (hash as u32) ^ ((hash >> 32) as u32);
        if id != 0 && notes.iter().all(|note| note.id != id) {
            return id;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "bezel-inbox-{}-{}",
            std::process::id(),
            fresh_id(&[])
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn note(id: u32, rev: u32, text: &str) -> Frame {
        Frame::Note {
            id,
            rev,
            deleted: false,
            from: "Ada West".into(),
            to: "Bea North".into(),
            text: text.into(),
        }
    }

    #[test]
    fn higher_revision_replaces_and_an_equal_one_stays() {
        let mut store = Store::blank(PathBuf::new());
        store.observe(note(7, 1, "one")).unwrap();
        store.observe(note(7, 2, "two")).unwrap();
        assert_eq!(store.notes()[0].text, "two");
        store.observe(note(7, 2, "other")).unwrap();
        assert_eq!(store.notes()[0].text, "two");
        let echoed = store.observe(note(7, 2, "other")).unwrap();
        assert!(echoed.is_empty());
    }

    #[test]
    fn a_delete_is_not_resurrected_by_an_older_copy() {
        let mut store = Store::blank(PathBuf::new());
        store.observe(note(4, 1, "keep")).unwrap();
        store
            .observe(Frame::Note {
                id: 4,
                rev: 2,
                deleted: true,
                from: "Bea North".into(),
                to: "Ada West".into(),
                text: "keep".into(),
            })
            .unwrap();
        assert!(store.notes().iter().all(|item| item.deleted));
        store.observe(note(4, 1, "keep")).unwrap();
        assert!(store.notes()[0].deleted);
    }

    #[test]
    fn the_file_round_trips_and_a_later_edit_is_pending() {
        let root = dir();
        let path = root.join("inbox.txt");
        let mut store = Store::open(path.clone()).unwrap();
        let created = store.add("buy milk").unwrap();
        assert_eq!(Store::open(path.clone()).unwrap().notes(), store.notes());
        let mut notes = store.notes().to_vec();
        notes[0].text = "buy oats".to_string();
        notes[0].rev = 5;
        notes[0].to = "Bea North".to_string();
        fs::write(&path, render(&notes)).unwrap();
        let mut live = Store::open(path.clone()).unwrap();
        let _ = live.greet();
        let outbound = live
            .observe(Frame::InboxHello {
                name: "Bea North".into(),
            })
            .unwrap();
        assert!(outbound.iter().any(|frame| matches!(
            frame,
            Frame::Note { id, text, rev: 5, .. } if *id == created.id && text == "buy oats"
        )));
        notes[0].text = "buy bread".to_string();
        notes[0].rev = 6;
        fs::write(&path, render(&notes)).unwrap();
        let again = live.poll().unwrap();
        assert!(again.iter().any(|frame| matches!(
            frame,
            Frame::Note { text, rev: 6, .. } if text == "buy bread"
        )));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_and_tabbed_text_is_refused() {
        let mut store = Store::blank(PathBuf::from("unused"));
        assert!(store.add("  ").is_err());
        assert!(store.add("a\tb").is_err());
    }
}
