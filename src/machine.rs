//! Who owns the pointer. Pending does not swallow input. A dead peer cannot
//! capture the mouse. Giving up ownership emits a key-up for every held key.

use crate::desk::{Desk, Edge};
use crate::frame::Frame;
use crate::hid;
use std::collections::BTreeSet;

pub const PENDING_MS: u64 = 150;
pub const HEARTBEAT_MS: u64 = 200;
pub const MISS_MS: u64 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Local,
    Pending,
    Remote,
}

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Pointer {
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
        now: u64,
    },
    Button {
        button: u8,
        down: bool,
        now: u64,
    },
    Wheel {
        dx: i32,
        dy: i32,
        now: u64,
    },
    Key {
        hid: u16,
        down: bool,
        now: u64,
    },
    EnterAck {
        seq: u32,
        now: u64,
    },
    Leave {
        now: u64,
    },
    Heartbeat {
        now: u64,
    },
    PeerUp {
        now: u64,
    },
    PeerDown {
        now: u64,
    },
    Tick {
        now: u64,
    },
    ForceLocal {
        now: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Send(Frame),
    HideCursor { x: i32, y: i32 },
    ShowCursor { x: i32, y: i32 },
    Note(&'static str),
}

#[derive(Clone, Debug)]
pub struct Machine {
    mode: Mode,
    held_keys: BTreeSet<u16>,
    held_buttons: BTreeSet<u8>,
    peer_up: bool,
    pending_at: u64,
    last_rx: u64,
    last_beat: u64,
    next_seq: u32,
    awaiting: u32,
    move_dx: i32,
    move_dy: i32,
    park_x: i32,
    park_y: i32,
    edge: Edge,
}

impl Machine {
    pub fn new(edge: Edge) -> Self {
        Self {
            mode: Mode::Local,
            held_keys: BTreeSet::new(),
            held_buttons: BTreeSet::new(),
            peer_up: false,
            pending_at: 0,
            last_rx: 0,
            last_beat: 0,
            next_seq: 1,
            awaiting: 0,
            move_dx: 0,
            move_dy: 0,
            park_x: 0,
            park_y: 0,
            edge,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn swallow(&self) -> bool {
        self.mode == Mode::Remote
    }

    pub fn held_keys(&self) -> &BTreeSet<u16> {
        &self.held_keys
    }

    pub fn park(&self) -> Option<(i32, i32)> {
        if self.mode == Mode::Remote {
            Some((self.park_x, self.park_y))
        } else {
            None
        }
    }

    pub fn handle(&mut self, event: Event, desk: &Desk) -> Vec<Action> {
        match event {
            Event::PeerUp { now } => {
                self.peer_up = true;
                self.last_rx = now;
                self.last_beat = now;
                Vec::new()
            }
            Event::PeerDown { now } => self.drop_peer(now),
            Event::Heartbeat { now } => {
                self.last_rx = now;
                Vec::new()
            }
            Event::EnterAck { seq, now } => self.enter_ack(seq, now),
            Event::Leave { now } => self.give_up(now, "left"),
            Event::ForceLocal { now } => self.give_up(now, "left"),
            Event::Tick { now } => self.tick(now),
            Event::Pointer { x, y, dx, dy, now } => self.pointer(desk, x, y, dx, dy, now),
            Event::Button { button, down, now } => self.button(button, down, now),
            Event::Wheel { dx, dy, now } => self.wheel(dx, dy, now),
            Event::Key { hid, down, now } => self.key(hid, down, now),
        }
    }

    fn pointer(&mut self, desk: &Desk, x: i32, y: i32, dx: i32, dy: i32, now: u64) -> Vec<Action> {
        match self.mode {
            Mode::Remote => {
                self.move_dx = self.move_dx.saturating_add(dx);
                self.move_dy = self.move_dy.saturating_add(dy);
                Vec::new()
            }
            Mode::Pending => Vec::new(),
            Mode::Local => {
                if self.peer_up && desk.arms(self.edge, x, y, dx) {
                    self.begin_pending(desk, y, now)
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn begin_pending(&mut self, desk: &Desk, y: i32, now: u64) -> Vec<Action> {
        let Some(crossing) = desk.crossing(self.edge, y) else {
            return Vec::new();
        };
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.awaiting = seq;
        self.pending_at = now;
        self.mode = Mode::Pending;
        self.park_x = crossing.park_x;
        self.park_y = crossing.park_y;
        vec![Action::Send(Frame::Enter {
            seq,
            y: crossing.y,
            span: crossing.span,
        })]
    }

    fn enter_ack(&mut self, seq: u32, now: u64) -> Vec<Action> {
        self.last_rx = now;
        if self.mode != Mode::Pending || seq != self.awaiting {
            return Vec::new();
        }
        self.mode = Mode::Remote;
        self.move_dx = 0;
        self.move_dy = 0;
        vec![
            Action::Note("entered"),
            Action::HideCursor {
                x: self.park_x,
                y: self.park_y,
            },
        ]
    }

    fn button(&mut self, button: u8, down: bool, now: u64) -> Vec<Action> {
        let _ = now;
        if self.mode != Mode::Remote {
            return Vec::new();
        }
        if down {
            if !self.held_buttons.insert(button) {
                return Vec::new();
            }
        } else if !self.held_buttons.remove(&button) {
            return Vec::new();
        }
        vec![Action::Send(Frame::Button { button, down })]
    }

    fn wheel(&mut self, dx: i32, dy: i32, now: u64) -> Vec<Action> {
        let _ = now;
        if self.mode != Mode::Remote || (dx == 0 && dy == 0) {
            return Vec::new();
        }
        vec![Action::Send(Frame::Wheel {
            dx: clamp_i16(dx),
            dy: clamp_i16(dy),
        })]
    }

    fn key(&mut self, hid: u16, down: bool, _now: u64) -> Vec<Action> {
        if hid::is_gui(hid) || self.mode != Mode::Remote {
            return Vec::new();
        }
        if down {
            if !self.held_keys.insert(hid) {
                return Vec::new();
            }
        } else if !self.held_keys.remove(&hid) {
            return Vec::new();
        }
        vec![Action::Send(Frame::Key { hid, down })]
    }

    fn tick(&mut self, now: u64) -> Vec<Action> {
        if self.peer_up && now.saturating_sub(self.last_rx) >= MISS_MS {
            return self.drop_peer(now);
        }
        let mut actions = Vec::new();
        if self.mode == Mode::Pending && now.saturating_sub(self.pending_at) >= PENDING_MS {
            self.mode = Mode::Local;
        }
        if self.mode == Mode::Remote && (self.move_dx != 0 || self.move_dy != 0) {
            actions.push(Action::Send(Frame::RelMove {
                dx: clamp_i16(self.move_dx),
                dy: clamp_i16(self.move_dy),
            }));
            self.move_dx = 0;
            self.move_dy = 0;
        }
        if self.peer_up && now.saturating_sub(self.last_beat) >= HEARTBEAT_MS {
            self.last_beat = now;
            actions.push(Action::Send(Frame::Heartbeat));
        }
        actions
    }

    fn drop_peer(&mut self, now: u64) -> Vec<Action> {
        self.peer_up = false;
        let mut actions = self.release_held();
        if self.mode == Mode::Remote {
            actions.push(Action::ShowCursor {
                x: self.park_x,
                y: self.park_y,
            });
        }
        self.mode = Mode::Local;
        self.move_dx = 0;
        self.move_dy = 0;
        actions.push(Action::Note("disconnected"));
        let _ = now;
        actions
    }

    fn give_up(&mut self, _now: u64, note: &'static str) -> Vec<Action> {
        if self.mode == Mode::Local {
            return Vec::new();
        }
        let was_remote = self.mode == Mode::Remote;
        let mut actions = self.release_held();
        if was_remote {
            actions.push(Action::ShowCursor {
                x: self.park_x,
                y: self.park_y,
            });
            actions.push(Action::Note(note));
        }
        self.mode = Mode::Local;
        self.move_dx = 0;
        self.move_dy = 0;
        actions
    }

    fn release_held(&mut self) -> Vec<Action> {
        if self.held_keys.is_empty() && self.held_buttons.is_empty() {
            return Vec::new();
        }
        let mut actions = vec![Action::Note("released")];
        for hid in std::mem::take(&mut self.held_keys) {
            actions.push(Action::Send(Frame::Key { hid, down: false }));
        }
        for button in std::mem::take(&mut self.held_buttons) {
            actions.push(Action::Send(Frame::Button { button, down: false }));
        }
        actions.push(Action::Send(Frame::Release));
        actions
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new(Edge::Left)
    }
}

fn clamp_i16(value: i32) -> i16 {
    value.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::{darwin_desk, Edge};
    use crate::frame::Frame;
    use crate::hid::LEFT_SHIFT;

    fn sends(actions: &[Action]) -> Vec<Frame> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Send(frame) => Some(*frame),
                _ => None,
            })
            .collect()
    }

    fn enter(machine: &mut Machine) -> u32 {
        let desk = darwin_desk();
        machine.handle(Event::PeerUp { now: 0 }, &desk);
        let actions = machine.handle(
            Event::Pointer {
                x: -7280,
                y: 100,
                dx: -2,
                dy: 0,
                now: 10,
            },
            &desk,
        );
        match sends(&actions)[0] {
            Frame::Enter { seq, .. } => {
                machine.handle(Event::EnterAck { seq, now: 20 }, &desk);
                seq
            }
            other => panic!("expected enter, got {other:?}"),
        }
    }

    #[test]
    fn pending_without_ack_stays_local() {
        let desk = darwin_desk();
        let mut machine = Machine::new(Edge::Left);
        machine.handle(Event::PeerUp { now: 0 }, &desk);
        machine.handle(
            Event::Pointer {
                x: -7280,
                y: 400,
                dx: -1,
                dy: 0,
                now: 10,
            },
            &desk,
        );
        assert_eq!(machine.mode(), Mode::Pending);
        assert!(!machine.swallow());
        machine.handle(Event::Tick { now: 159 }, &desk);
        assert_eq!(machine.mode(), Mode::Pending);
        machine.handle(Event::Tick { now: 160 }, &desk);
        assert_eq!(machine.mode(), Mode::Local);
    }

    #[test]
    fn ack_hides_the_cursor() {
        let mut machine = Machine::new(Edge::Left);
        enter(&mut machine);
        assert_eq!(machine.mode(), Mode::Remote);
        assert!(machine.swallow());
    }

    #[test]
    fn stale_ack_does_not_enter() {
        let desk = darwin_desk();
        let mut machine = Machine::new(Edge::Left);
        machine.handle(Event::PeerUp { now: 0 }, &desk);
        machine.handle(
            Event::Pointer {
                x: -7280,
                y: 10,
                dx: -1,
                dy: 0,
                now: 5,
            },
            &desk,
        );
        machine.handle(Event::EnterAck { seq: 99, now: 6 }, &desk);
        assert_eq!(machine.mode(), Mode::Pending);
    }

    #[test]
    fn shift_down_then_disconnect_releases_shift() {
        let desk = darwin_desk();
        let mut machine = Machine::new(Edge::Left);
        enter(&mut machine);
        machine.handle(
            Event::Key {
                hid: LEFT_SHIFT,
                down: true,
                now: 30,
            },
            &desk,
        );
        let actions = machine.handle(Event::PeerDown { now: 40 }, &desk);
        assert!(sends(&actions).contains(&Frame::Key {
            hid: LEFT_SHIFT,
            down: false,
        }));
        assert!(sends(&actions).contains(&Frame::Release));
        assert!(machine.held_keys().is_empty());
        assert_eq!(machine.mode(), Mode::Local);
        assert!(actions.iter().any(|action| action == &Action::Note("released")));
        assert!(actions
            .iter()
            .any(|action| action == &Action::Note("disconnected")));
    }

    #[test]
    fn missed_heartbeat_returns_local() {
        let desk = darwin_desk();
        let mut machine = Machine::new(Edge::Left);
        enter(&mut machine);
        machine.handle(Event::Tick { now: 619 }, &desk);
        assert_eq!(machine.mode(), Mode::Remote);
        let actions = machine.handle(Event::Tick { now: 620 }, &desk);
        assert_eq!(machine.mode(), Mode::Local);
        assert!(actions
            .iter()
            .any(|action| action == &Action::Note("disconnected")));
    }

    #[test]
    fn remote_motion_flushes_once_per_tick() {
        let desk = darwin_desk();
        let mut machine = Machine::new(Edge::Left);
        enter(&mut machine);
        machine.handle(
            Event::Pointer {
                x: 0,
                y: 0,
                dx: 2,
                dy: 3,
                now: 30,
            },
            &desk,
        );
        machine.handle(
            Event::Pointer {
                x: 0,
                y: 0,
                dx: 4,
                dy: -1,
                now: 31,
            },
            &desk,
        );
        let actions = machine.handle(Event::Tick { now: 40 }, &desk);
        assert!(sends(&actions).contains(&Frame::RelMove { dx: 6, dy: 2 }));
    }

    #[test]
    fn a_repeated_key_down_is_sent_once() {
        let desk = darwin_desk();
        let mut machine = Machine::new(Edge::Left);
        enter(&mut machine);
        let first = machine.handle(
            Event::Key {
                hid: 0x04,
                down: true,
                now: 30,
            },
            &desk,
        );
        let second = machine.handle(
            Event::Key {
                hid: 0x04,
                down: true,
                now: 50,
            },
            &desk,
        );
        assert_eq!(sends(&first), vec![Frame::Key { hid: 0x04, down: true }]);
        assert!(sends(&second).is_empty());
    }

    #[test]
    fn local_motion_off_the_edge_sends_nothing() {
        let desk = darwin_desk();
        let mut machine = Machine::new(Edge::Left);
        machine.handle(Event::PeerUp { now: 0 }, &desk);
        let actions = machine.handle(
            Event::Pointer {
                x: 100,
                y: 100,
                dx: 5,
                dy: 0,
                now: 10,
            },
            &desk,
        );
        assert!(actions.is_empty());
        assert_eq!(machine.mode(), Mode::Local);
    }

    #[test]
    fn dead_peer_does_not_arm() {
        let desk = darwin_desk();
        let mut machine = Machine::new(Edge::Left);
        let actions = machine.handle(
            Event::Pointer {
                x: -7280,
                y: 100,
                dx: -5,
                dy: 0,
                now: 10,
            },
            &desk,
        );
        assert!(actions.is_empty());
        assert_eq!(machine.mode(), Mode::Local);
    }
}
