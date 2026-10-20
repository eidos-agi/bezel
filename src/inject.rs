//! Mac-side cursor. Motion is applied here. Crossing back through x < 0
//! sends Leave. CoreGraphics posting stays in the macOS module.

use crate::frame::Frame;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct Cursor {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    active: bool,
    held_keys: BTreeSet<u16>,
    held_buttons: BTreeSet<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Place { x: f64, y: f64 },
    Button { button: u8, down: bool, x: f64, y: f64 },
    Wheel { dx: i16, dy: i16 },
    Key { hid: u16, down: bool },
    Send(Frame),
}

impl Cursor {
    pub fn new(w: f64, h: f64) -> Self {
        Self {
            x: 1.0,
            y: 0.0,
            w: w.max(1.0),
            h: h.max(1.0),
            active: false,
            held_keys: BTreeSet::new(),
            held_buttons: BTreeSet::new(),
        }
    }

    pub fn apply(&mut self, frame: Frame) -> Vec<Effect> {
        match frame {
            Frame::Enter { y, span, .. } => {
                self.active = true;
                self.x = 1.0;
                self.y = map_y(y, span, self.h);
                vec![Effect::Place { x: self.x, y: self.y }]
            }
            Frame::RelMove { dx, dy } if self.active => self.slide(dx, dy),
            Frame::Button { button, down } if self.active => {
                if down {
                    self.held_buttons.insert(button);
                } else {
                    self.held_buttons.remove(&button);
                }
                vec![Effect::Button {
                    button,
                    down,
                    x: self.x,
                    y: self.y,
                }]
            }
            Frame::Wheel { dx, dy } if self.active => vec![Effect::Wheel { dx, dy }],
            Frame::Key { hid, down } if self.active => {
                if down {
                    self.held_keys.insert(hid);
                } else {
                    self.held_keys.remove(&hid);
                }
                vec![Effect::Key { hid, down }]
            }
            Frame::Release | Frame::Leave => self.ups(),
            _ => Vec::new(),
        }
    }

    pub fn release_all(&mut self) -> Vec<Effect> {
        self.ups()
    }

    fn slide(&mut self, dx: i16, dy: i16) -> Vec<Effect> {
        self.x += f64::from(dx);
        self.y = (self.y + f64::from(dy)).clamp(0.0, self.h - 1.0);
        if self.x < 0.0 {
            self.active = false;
            self.x = 0.0;
            let mut effects = vec![Effect::Send(Frame::Leave)];
            effects.extend(self.ups());
            return effects;
        }
        if self.x > self.w - 1.0 {
            self.x = self.w - 1.0;
        }
        vec![Effect::Place { x: self.x, y: self.y }]
    }

    fn ups(&mut self) -> Vec<Effect> {
        self.active = false;
        let mut effects = Vec::new();
        for hid in std::mem::take(&mut self.held_keys) {
            effects.push(Effect::Key { hid, down: false });
        }
        for button in std::mem::take(&mut self.held_buttons) {
            effects.push(Effect::Button {
                button,
                down: false,
                x: self.x,
                y: self.y,
            });
        }
        effects
    }
}

fn map_y(y: u16, span: u16, height: f64) -> f64 {
    if span == 0 {
        return 0.0;
    }
    let t = f64::from(y) / f64::from(span);
    (t * (height - 1.0)).clamp(0.0, height - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_off_the_left_edge_sends_leave() {
        let mut cursor = Cursor::new(1800.0, 1169.0);
        cursor.apply(Frame::Enter {
            seq: 1,
            y: 100,
            span: 1200,
        });
        let stay = cursor.apply(Frame::RelMove { dx: 10, dy: 0 });
        assert!(matches!(stay[0], Effect::Place { x, .. } if (x - 11.0).abs() < 0.01));
        let left = cursor.apply(Frame::RelMove { dx: -20, dy: 0 });
        assert!(left.contains(&Effect::Send(Frame::Leave)));
        let ignored = cursor.apply(Frame::RelMove { dx: 5, dy: 0 });
        assert!(ignored.is_empty());
    }

    #[test]
    fn release_posts_a_held_key_up() {
        let mut cursor = Cursor::new(100.0, 100.0);
        cursor.apply(Frame::Enter {
            seq: 1,
            y: 0,
            span: 10,
        });
        cursor.apply(Frame::Key {
            hid: 0xE1,
            down: true,
        });
        let ups = cursor.apply(Frame::Release);
        assert!(ups.contains(&Effect::Key {
            hid: 0xE1,
            down: false,
        }));
    }
}
