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

    pub fn arms(&self, edge: Edge, x: i32, y: i32, dx: i32) -> bool {
        let outward = match edge {
            Edge::Left => dx < 0,
            Edge::Right => dx > 0,
        };
        outward && self.on_exit(edge, x, y)
    }

    pub fn crossing(&self, edge: Edge, y: i32) -> Option<Crossing> {
        let mon = self.exit_monitor(edge, y)?;
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
        self.exit_monitor(edge, y)
            .is_some_and(|mon| x == exit_x(mon, edge))
    }

    /// A neighbor on the exit side blocks the edge, so moving between Windows
    /// monitors does not arm a cross.
    fn exit_monitor(&self, edge: Edge, y: i32) -> Option<&Mon> {
        let found = self.monitors.iter().filter(|mon| self.outer(mon, edge, y));
        match edge {
            Edge::Left => found.min_by_key(|mon| mon.x),
            Edge::Right => found.max_by_key(|mon| mon.x + mon.w),
        }
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
        // Edges between Windows monitors, and the primary's right edge.
        assert!(!desk.arms(Edge::Left, -3840, 100, -2));
        assert!(!desk.arms(Edge::Left, 0, 100, -2));
        assert!(!desk.arms(Edge::Left, 1919, 100, 2));
    }

    #[test]
    fn park_sits_one_pixel_inside_the_left_edge() {
        let desk = darwin_desk();
        let cross = desk.crossing(Edge::Left, 100).unwrap();
        assert_eq!(cross.park_x, -7279);
        assert_eq!(cross.park_y, 100);
        assert_eq!(cross.y, 100);
        assert_eq!(cross.span, 1440);
    }

    #[test]
    fn the_right_edge_is_the_primary() {
        let desk = darwin_desk();
        assert!(desk.arms(Edge::Right, 1919, 100, 1));
        assert!(!desk.arms(Edge::Right, -7280, 100, -1));
        let cross = desk.crossing(Edge::Right, 100).unwrap();
        assert_eq!(cross.park_x, 1918);
        assert_eq!(cross.span, 1200);
    }
}
