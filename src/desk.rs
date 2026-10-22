//! Which pixel hands the pointer to the Mac. `Edge::Left` is the outer left
//! edge, used when the Mac sits to the left of the Windows desktop.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
}

impl Edge {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            other => Err(format!("edge must be left or right, not {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    pub fn wire(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }

    pub fn from_wire(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Left),
            1 => Some(Self::Right),
            _ => None,
        }
    }
}

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

    pub fn monitors(&self) -> &[Mon] {
        &self.monitors
    }

    /// True when a move outward from this monitor can leave the desktop.
    /// A gap counts. A seam where another screen touches does not.
    pub fn edge_open(&self, mon: &Mon, edge: Edge) -> bool {
        if mon.w <= 1 || mon.h <= 0 {
            return false;
        }
        (0..mon.h).any(|dy| self.outer(mon, edge, mon.y.saturating_add(dy)))
    }

    pub fn arms(&self, edge: Edge, x: i32, y: i32, dx: i32) -> bool {
        let outward = match edge {
            Edge::Left => dx < 0,
            Edge::Right => dx > 0,
        };
        outward && self.on_exit(edge, x, y)
    }

    pub fn crossing(&self, edge: Edge, x: i32, y: i32) -> Option<Crossing> {
        let mon = self.edge_monitor(edge, x, y)?;
        let rel = y - mon.y;
        let park_x = match edge {
            Edge::Left => mon.x + 1,
            Edge::Right => mon.x + mon.w - 2,
        };
        Some(Crossing {
            park_x,
            park_y: y,
            y: rel as u16,
            span: mon.h as u16,
        })
    }

    fn on_exit(&self, edge: Edge, x: i32, y: i32) -> bool {
        self.edge_monitor(edge, x, y).is_some()
    }

    /// The monitor under the cursor, when this pixel is a hard edge.
    /// A neighbor on the exit side blocks it, so moving between touching
    /// Windows screens does not arm a cross. A gap is a hard edge.
    fn edge_monitor(&self, edge: Edge, x: i32, y: i32) -> Option<&Mon> {
        self.monitors.iter().find(|mon| {
            y >= mon.y
                && y < mon.y + mon.h
                && x == exit_x(mon, edge)
                && self.outer(mon, edge, y)
        })
    }

    fn outer(&self, mon: &Mon, edge: Edge, y: i32) -> bool {
        if mon.w <= 1 || mon.h <= 0 || y < mon.y || y >= mon.y + mon.h {
            return false;
        }
        let beside = match edge {
            Edge::Left => mon.x - 1,
            Edge::Right => mon.x + mon.w,
        };
        !self.covers(beside, y)
    }

    fn covers(&self, x: i32, y: i32) -> bool {
        self.monitors.iter().any(|mon| {
            x >= mon.x && x < mon.x + mon.w && y >= mon.y && y < mon.y + mon.h
        })
    }
}

fn exit_x(mon: &Mon, edge: Edge) -> i32 {
    match edge {
        Edge::Left => mon.x,
        Edge::Right => mon.x + mon.w - 1,
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
    fn only_the_outer_left_edge_arms() {
        let desk = darwin_desk();
        assert!(desk.arms(Edge::Left, -7280, 100, -1));
        assert!(!desk.arms(Edge::Left, -7280, 100, 0));
        assert!(!desk.arms(Edge::Left, -7280, 100, 3));
        assert!(!desk.arms(Edge::Left, -7279, 100, -4));
        assert!(!desk.arms(Edge::Left, -7280, -1, -1));
        assert!(!desk.arms(Edge::Left, -7280, 1440, -1));
        // The 2560 screen touches the ultrawide, so its left edge stays on Windows.
        assert!(!desk.arms(Edge::Left, -3840, 100, -2));
        // The primary does not touch the screen on its left. That hard edge crosses.
        assert!(desk.arms(Edge::Left, 0, 100, -2));
        assert!(!desk.arms(Edge::Left, 1919, 100, 2));
    }

    #[test]
    fn park_sits_one_pixel_inside_the_left_edge() {
        let desk = darwin_desk();
        let cross = desk.crossing(Edge::Left, -7280, 100).unwrap();
        assert_eq!(cross.park_x, -7279);
        assert_eq!(cross.park_y, 100);
        assert_eq!(cross.y, 100);
        assert_eq!(cross.span, 1440);
        let primary = desk.crossing(Edge::Left, 0, 100).unwrap();
        assert_eq!(primary.park_x, 1);
        assert_eq!(primary.span, 1200);
    }

    #[test]
    fn the_right_edge_is_the_primary() {
        let desk = darwin_desk();
        assert!(desk.arms(Edge::Right, 1919, 100, 1));
        assert!(!desk.arms(Edge::Right, -7280, 100, -1));
        let cross = desk.crossing(Edge::Right, 1919, 100).unwrap();
        assert_eq!(cross.park_x, 1918);
        assert_eq!(cross.span, 1200);
    }

    #[test]
    fn a_gap_is_open_and_a_seam_is_not() {
        let desk = darwin_desk();
        let mons = desk.monitors();
        assert!(desk.edge_open(&mons[0], Edge::Left));
        assert!(!desk.edge_open(&mons[1], Edge::Left));
        assert!(desk.edge_open(&mons[2], Edge::Left));

        let gap = Desk::new(vec![
            Mon { x: -3840, y: 0, w: 2560, h: 1440, primary: false },
            Mon { x: 0, y: 0, w: 1920, h: 1200, primary: true },
        ]);
        assert!(gap.monitors().iter().all(|mon| gap.edge_open(mon, Edge::Left)));

        let touching = Desk::new(vec![
            Mon { x: 0, y: 0, w: 1920, h: 1080, primary: true },
            Mon { x: 1920, y: 0, w: 1920, h: 1080, primary: false },
        ]);
        assert!(touching.edge_open(&touching.monitors()[0], Edge::Left));
        assert!(!touching.edge_open(&touching.monitors()[1], Edge::Left));
        assert!(!touching.edge_open(&touching.monitors()[0], Edge::Right));
        assert!(touching.edge_open(&touching.monitors()[1], Edge::Right));
    }
}
