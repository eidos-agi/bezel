//! Turn raw mouse mickeys into the pixels Windows would have moved.
//! Speed is the 1..20 value from SPI_GETMOUSESPEED. 10 is one pixel per mickey.
//! A remainder is kept so a slow pointer still arrives.

#[derive(Clone, Debug)]
pub struct Ballistics {
    speed: i32,
    carry_x: i32,
    carry_y: i32,
}

impl Ballistics {
    pub fn new(speed: i32) -> Self {
        Self {
            speed: speed.clamp(1, 20),
            carry_x: 0,
            carry_y: 0,
        }
    }

    pub fn pixels(&mut self, mickey_x: i32, mickey_y: i32) -> (i32, i32) {
        let x = self.axis(mickey_x, true);
        let y = self.axis(mickey_y, false);
        (x, y)
    }

    fn axis(&mut self, mickeys: i32, horizontal: bool) -> i32 {
        let carry = if horizontal {
            &mut self.carry_x
        } else {
            &mut self.carry_y
        };
        let scaled = mickeys.saturating_mul(self.speed).saturating_add(*carry);
        let pixels = scaled / 10;
        *carry = scaled % 10;
        pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_ten_is_one_pixel_per_mickey() {
        let mut ballistics = Ballistics::new(10);
        assert_eq!(ballistics.pixels(10, -3), (10, -3));
    }

    #[test]
    fn speed_five_keeps_the_remainder() {
        let mut ballistics = Ballistics::new(5);
        assert_eq!(ballistics.pixels(1, 0), (0, 0));
        assert_eq!(ballistics.pixels(1, 0), (1, 0));
    }
}
