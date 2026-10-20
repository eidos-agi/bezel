//! Which pixel of which monitor is allowed to hand the pointer to the Mac.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mon {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desk {
    monitors: Vec<Mon>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crossing {
    pub park_x: i32,
    pub park_y: i32,
    pub y: u16,
    pub span: u16,
}

impl Desk {
    pub fn new(monitors: Vec<Mon>) -> Self {
        Self { monitors }
    }

    pub fn arms(&self, x: i32, y: i32, dx: i32) -> bool {
        dx > 0 && self.primary_edge(x, y).is_some()
    }

    pub fn crossing(&self, y: i32) -> Option<Crossing> {
        let primary = self.primary()?;
        let rel = y - primary.y;
        if rel < 0 || rel >= primary.h {
            return None;
        }
        Some(Crossing {
            park_x: primary.x + primary.w - 2,
            park_y: primary.y + rel,
            y: rel as u16,
            span: primary.h as u16,
        })
    }

    fn primary_edge(&self, x: i32, y: i32) -> Option<&Mon> {
        let primary = self.primary()?;
        let on_edge = x == primary.x + primary.w - 1;
        let in_band = y >= primary.y && y < primary.y + primary.h;
        if on_edge && in_band {
            Some(primary)
        } else {
            None
        }
    }

    fn primary(&self) -> Option<&Mon> {
        self.monitors.iter().find(|mon| mon.primary && mon.w > 1 && mon.h > 0)
    }
}

/// The layout measured on darwin-dell-xps: ultrawide, 2560x1440, then the primary.
pub fn darwin_desk() -> Desk {
    Desk::new(vec![
        Mon {
            x: -7280,
            y: 0,
            w: 3440,
            h: 1440,
            primary: false,
        },
        Mon {
            x: -3840,
            y: 0,
            w: 2560,
            h: 1440,
            primary: false,
        },
        Mon {
            x: 0,
            y: 0,
            w: 1920,
            h: 1200,
            primary: true,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_primary_right_edge_arms() {
        let desk = darwin_desk();
        assert!(desk.arms(1919, 100, 1));
        assert!(!desk.arms(1919, 100, 0));
        assert!(!desk.arms(1919, 100, -3));
        assert!(!desk.arms(1918, 100, 4));
        assert!(!desk.arms(1919, -1, 1));
        assert!(!desk.arms(1919, 1200, 1));
        // Right edge of the ultrawide, then the 2560x1440 panel.
        assert!(!desk.arms(-7280 + 3440 - 1, 100, 2));
        assert!(!desk.arms(-3840 + 2560 - 1, 100, 2));
    }

    #[test]
    fn park_sits_one_pixel_inside_the_primary() {
        let desk = darwin_desk();
        let cross = desk.crossing(100).unwrap();
        assert_eq!(cross.park_x, 1918);
        assert_eq!(cross.park_y, 100);
        assert_eq!(cross.y, 100);
        assert_eq!(cross.span, 1200);
    }
}
