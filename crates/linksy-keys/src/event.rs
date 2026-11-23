//! An input event as the kernel names it, without its timestamp: what Continuous Keys
//! carries from the keyboard and mouse of one computer to the virtual ones of another.

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const SYN_REPORT: u16 = 0;
pub const KEY_MAX: u16 = 0x2ff;

pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
pub const REL_HWHEEL: u16 = 0x06;
pub const REL_WHEEL: u16 = 0x08;
pub const REL_WHEEL_HI_RES: u16 = 0x0b;
pub const REL_HWHEEL_HI_RES: u16 = 0x0c;

pub const KEY_ENTER: u16 = 28;
pub const KEY_A: u16 = 30;
pub const BTN_LEFT: u16 = 0x110;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

impl Event {
    pub const SIZE: usize = 8;

    pub fn key(code: u16, down: bool) -> Self {
        Self { kind: EV_KEY, code, value: down as i32 }
    }

    pub fn sync() -> Self {
        Self { kind: EV_SYN, code: SYN_REPORT, value: 0 }
    }

    pub fn is_key(self) -> bool {
        self.kind == EV_KEY
    }

    /// Worth carrying over: keys and buttons (not the kernel's auto-repeat, the other
    /// computer repeats on its own), pointer motion and wheels, and the end of each batch.
    pub fn carried(self) -> bool {
        match self.kind {
            EV_KEY => self.value == 0 || self.value == 1,
            EV_REL => matches!(self.code, REL_X | REL_Y | REL_HWHEEL | REL_WHEEL | REL_WHEEL_HI_RES | REL_HWHEEL_HI_RES),
            EV_SYN => self.code == SYN_REPORT,
            _ => false,
        }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0; Self::SIZE];
        out[0..2].copy_from_slice(&self.kind.to_le_bytes());
        out[2..4].copy_from_slice(&self.code.to_le_bytes());
        out[4..8].copy_from_slice(&self.value.to_le_bytes());
        out
    }

    pub fn from_bytes(bytes: [u8; Self::SIZE]) -> Self {
        Self {
            kind: u16::from_le_bytes([bytes[0], bytes[1]]),
            code: u16::from_le_bytes([bytes[2], bytes[3]]),
            value: i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_survives_the_wire() {
        for e in [Event::key(30, true), Event::key(0x110, false), Event { kind: EV_REL, code: REL_X, value: -42 }, Event::sync()] {
            assert_eq!(Event::from_bytes(e.to_bytes()), e);
        }
    }

    #[test]
    fn auto_repeat_and_other_noise_stay_home() {
        assert!(!Event { kind: EV_KEY, code: 30, value: 2 }.carried());
        assert!(!Event { kind: 4, code: 4, value: 30 }.carried());
        assert!(!Event { kind: EV_REL, code: 0x09, value: 1 }.carried());
        assert!(Event::key(30, true).carried() && Event::sync().carried());
        assert!(Event { kind: EV_REL, code: REL_WHEEL_HI_RES, value: 120 }.carried());
    }
}
