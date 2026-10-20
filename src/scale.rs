//! Optional pointer scale after Windows has produced pixels.
//! The default is 1.0. A fractional scale keeps a remainder.

#[derive(Clone, Debug)]
pub struct Scaler {
    scale: f64,
    carry_x: f64,
    carry_y: f64,
}

impl Scaler {
    pub fn new(scale: f64) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        Self {
            scale,
            carry_x: 0.0,
            carry_y: 0.0,
        }
    }

    pub fn apply(&mut self, dx: i32, dy: i32) -> (i32, i32) {
        (self.axis(dx, true), self.axis(dy, false))
    }

    fn axis(&mut self, pixels: i32, horizontal: bool) -> i32 {
        let carry = if horizontal {
            &mut self.carry_x
        } else {
            &mut self.carry_y
        };
        let scaled = pixels as f64 * self.scale + *carry;
        let whole = scaled.trunc();
        *carry = scaled - whole;
        whole.clamp(i32::MIN as f64, i32::MAX as f64) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_is_identity() {
        let mut scaler = Scaler::new(1.0);
        assert_eq!(scaler.apply(4, -2), (4, -2));
    }

    #[test]
    fn half_scale_accumulates() {
        let mut scaler = Scaler::new(0.5);
        assert_eq!(scaler.apply(1, 0), (0, 0));
        assert_eq!(scaler.apply(1, 0), (1, 0));
    }
}
