//! USB HID keyboard usages, plus the two maps Bezel needs.
//! Scancodes are Windows set-1 make codes. Mac virtual keycodes are the
//! positional ANSI codes. Ctrl is Command and Alt is Option on the Mac.

pub const LEFT_CTRL: u16 = 0xE0;
pub const LEFT_SHIFT: u16 = 0xE1;
pub const LEFT_ALT: u16 = 0xE2;
pub const LEFT_GUI: u16 = 0xE3;
pub const RIGHT_CTRL: u16 = 0xE4;
pub const RIGHT_SHIFT: u16 = 0xE5;
pub const RIGHT_ALT: u16 = 0xE6;
pub const RIGHT_GUI: u16 = 0xE7;

pub fn is_gui(hid: u16) -> bool {
    hid == LEFT_GUI || hid == RIGHT_GUI
}

pub fn is_ctrl(hid: u16) -> bool {
    hid == LEFT_CTRL || hid == RIGHT_CTRL
}

pub fn is_shift(hid: u16) -> bool {
    hid == LEFT_SHIFT || hid == RIGHT_SHIFT
}

pub fn is_alt(hid: u16) -> bool {
    hid == LEFT_ALT || hid == RIGHT_ALT
}

pub fn is_modifier(hid: u16) -> bool {
    is_ctrl(hid) || is_shift(hid) || is_alt(hid) || is_gui(hid)
}

/// `(make_code, extended)` from a Windows raw keyboard packet.
pub fn from_set1(make: u16, extended: bool) -> Option<u16> {
    if extended {
        return match make {
            0x1D => Some(RIGHT_CTRL),
            0x38 => Some(RIGHT_ALT),
            0x5B => Some(LEFT_GUI),
            0x5C => Some(RIGHT_GUI),
            0x48 => Some(0x52),
            0x50 => Some(0x51),
            0x4B => Some(0x50),
            0x4D => Some(0x4F),
            0x47 => Some(0x4A),
            0x4F => Some(0x4D),
            0x49 => Some(0x4B),
            0x51 => Some(0x4E),
            0x53 => Some(0x4C),
            _ => None,
        };
    }
    match make {
        0x01 => Some(0x29),
        0x02 => Some(0x1E),
        0x03 => Some(0x1F),
        0x04 => Some(0x20),
        0x05 => Some(0x21),
        0x06 => Some(0x22),
        0x07 => Some(0x23),
        0x08 => Some(0x24),
        0x09 => Some(0x25),
        0x0A => Some(0x26),
        0x0B => Some(0x27),
        0x0C => Some(0x2D),
        0x0D => Some(0x2E),
        0x0E => Some(0x2A),
        0x0F => Some(0x2B),
        0x10 => Some(0x14),
        0x11 => Some(0x1A),
        0x12 => Some(0x08),
        0x13 => Some(0x15),
        0x14 => Some(0x17),
        0x15 => Some(0x1C),
        0x16 => Some(0x18),
        0x17 => Some(0x0C),
        0x18 => Some(0x12),
        0x19 => Some(0x13),
        0x1A => Some(0x2F),
        0x1B => Some(0x30),
        0x1C => Some(0x28),
        0x1D => Some(LEFT_CTRL),
        0x1E => Some(0x04),
        0x1F => Some(0x16),
        0x20 => Some(0x07),
        0x21 => Some(0x09),
        0x22 => Some(0x0A),
        0x23 => Some(0x0B),
        0x24 => Some(0x0D),
        0x25 => Some(0x0E),
        0x26 => Some(0x0F),
        0x27 => Some(0x33),
        0x28 => Some(0x34),
        0x29 => Some(0x35),
        0x2A => Some(LEFT_SHIFT),
        0x2B => Some(0x31),
        0x2C => Some(0x1D),
        0x2D => Some(0x1B),
        0x2E => Some(0x06),
        0x2F => Some(0x19),
        0x30 => Some(0x05),
        0x31 => Some(0x11),
        0x32 => Some(0x10),
        0x33 => Some(0x36),
        0x34 => Some(0x37),
        0x35 => Some(0x38),
        0x36 => Some(RIGHT_SHIFT),
        0x38 => Some(LEFT_ALT),
        0x39 => Some(0x2C),
        0x3B => Some(0x3A),
        0x3C => Some(0x3B),
        0x3D => Some(0x3C),
        0x3E => Some(0x3D),
        0x3F => Some(0x3E),
        0x40 => Some(0x3F),
        0x41 => Some(0x40),
        0x42 => Some(0x41),
        0x43 => Some(0x42),
        0x44 => Some(0x43),
        _ => None,
    }
}

/// A low-level keyboard hook packet. Bit 0 is the extended prefix. Bit 7 is key up.
pub fn from_ll_hook(scan: u32, flags: u32) -> Option<(u16, bool)> {
    let hid = from_set1(scan as u16, flags & 1 != 0)?;
    let down = flags & 0x80 == 0;
    Some((hid, down))
}

/// Mac virtual keycode for a HID usage. Ctrl becomes Command. Alt becomes Option.
/// The Windows key is not forwarded, so it has no Mac code.
pub fn mac_keycode(hid: u16) -> Option<u16> {
    match hid {
        LEFT_CTRL => Some(0x37),
        RIGHT_CTRL => Some(0x36),
        LEFT_SHIFT => Some(0x38),
        RIGHT_SHIFT => Some(0x3C),
        LEFT_ALT => Some(0x3A),
        RIGHT_ALT => Some(0x3D),
        LEFT_GUI | RIGHT_GUI => None,
        hid => ANSI.get(&hid).copied(),
    }
}

/// HID usage to a positional ANSI virtual keycode.
static ANSI: phf_less::Map = phf_less::Map::new(&[
    (0x04, 0x00), // A
    (0x05, 0x0B), // B
    (0x06, 0x08), // C
    (0x07, 0x02), // D
    (0x08, 0x0E), // E
    (0x09, 0x03), // F
    (0x0A, 0x05), // G
    (0x0B, 0x04), // H
    (0x0C, 0x22), // I
    (0x0D, 0x26), // J
    (0x0E, 0x28), // K
    (0x0F, 0x25), // L
    (0x10, 0x2E), // M
    (0x11, 0x2D), // N
    (0x12, 0x1F), // O
    (0x13, 0x23), // P
    (0x14, 0x0C), // Q
    (0x15, 0x0F), // R
    (0x16, 0x01), // S
    (0x17, 0x11), // T
    (0x18, 0x20), // U
    (0x19, 0x09), // V
    (0x1A, 0x0D), // W
    (0x1B, 0x07), // X
    (0x1C, 0x10), // Y
    (0x1D, 0x06), // Z
    (0x1E, 0x12), // 1
    (0x1F, 0x13), // 2
    (0x20, 0x14), // 3
    (0x21, 0x15), // 4
    (0x22, 0x17), // 5
    (0x23, 0x16), // 6
    (0x24, 0x1A), // 7
    (0x25, 0x1C), // 8
    (0x26, 0x19), // 9
    (0x27, 0x1D), // 0
    (0x28, 0x24), // Enter
    (0x29, 0x35), // Escape
    (0x2A, 0x33), // Backspace
    (0x2B, 0x30), // Tab
    (0x2C, 0x31), // Space
    (0x2D, 0x1B), // -
    (0x2E, 0x18), // =
    (0x2F, 0x21), // [
    (0x30, 0x1E), // ]
    (0x31, 0x2A), // \
    (0x33, 0x29), // ;
    (0x34, 0x27), // '
    (0x35, 0x32), // `
    (0x36, 0x2B), // ,
    (0x37, 0x2F), // .
    (0x38, 0x2C), // /
    (0x3A, 0x7A), // F1
    (0x3B, 0x78), // F2
    (0x3C, 0x63), // F3
    (0x3D, 0x76), // F4
    (0x3E, 0x60), // F5
    (0x3F, 0x61), // F6
    (0x40, 0x62), // F7
    (0x41, 0x64), // F8
    (0x42, 0x65), // F9
    (0x43, 0x6D), // F10
    (0x44, 0x67), // F11
    (0x45, 0x6F), // F12
    (0x4A, 0x73), // Home
    (0x4B, 0x74), // PageUp
    (0x4C, 0x75), // Delete forward
    (0x4D, 0x77), // End
    (0x4E, 0x79), // PageDown
    (0x4F, 0x7C), // Right
    (0x50, 0x7B), // Left
    (0x51, 0x7D), // Down
    (0x52, 0x7E), // Up
]);

/// A tiny static map so the key table does not need another crate.
mod phf_less {
    pub struct Map {
        pairs: &'static [(u16, u16)],
    }

    impl Map {
        pub const fn new(pairs: &'static [(u16, u16)]) -> Self {
            Self { pairs }
        }

        pub fn get(&self, key: &u16) -> Option<&u16> {
            self.pairs
                .binary_search_by_key(key, |pair| pair.0)
                .ok()
                .map(|index| &self.pairs[index].1)
        }

        #[cfg(test)]
        pub(super) fn assert_sorted(&self) {
            let mut prev = 0;
            for (hid, _) in self.pairs {
                assert!(*hid > prev, "hid {hid:#x} is out of order");
                prev = *hid;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_packet_matches_the_raw_scan() {
        assert_eq!(from_ll_hook(0x1E, 0), Some((0x04, true)));
        assert_eq!(from_ll_hook(0x1E, 0x80), Some((0x04, false)));
        assert_eq!(from_ll_hook(0x1D, 1), Some((RIGHT_CTRL, true)));
    }

    #[test]
    fn set1_maps_the_keys_the_edge_cares_about() {
        assert_eq!(from_set1(0x1E, false), Some(0x04));
        assert_eq!(from_set1(0x2A, false), Some(LEFT_SHIFT));
        assert_eq!(from_set1(0x1D, false), Some(LEFT_CTRL));
        assert_eq!(from_set1(0x1D, true), Some(RIGHT_CTRL));
        assert_eq!(from_set1(0x5B, true), Some(LEFT_GUI));
    }

    #[test]
    fn ctrl_becomes_command_and_a_stays_positional() {
        assert_eq!(mac_keycode(0x04), Some(0x00));
        assert_eq!(mac_keycode(LEFT_CTRL), Some(0x37));
        assert_eq!(mac_keycode(RIGHT_CTRL), Some(0x36));
        assert_eq!(mac_keycode(LEFT_ALT), Some(0x3A));
        assert_eq!(mac_keycode(LEFT_SHIFT), Some(0x38));
        assert_eq!(mac_keycode(LEFT_GUI), None);
    }

    #[test]
    fn ansi_table_is_sorted() {
        ANSI.assert_sorted();
    }
}
