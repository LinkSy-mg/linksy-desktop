//! Driving a phone from this computer over the cable, with no developer options: Android
//! Open Accessory 2.0 lets the computer plug a virtual touchscreen and keyboard into the
//! phone (requests 54 to 57, as scrcpy's OTG mode does). The phone's screen comes as a
//! normal screen session; the viewer's clicks and keys become these devices' reports.

use std::time::Duration;

use linksy_core::{ControlMsg, MouseButton};
use nusb::transfer::{ControlOut, ControlType, Recipient};

const REGISTER: u8 = 54;
const UNREGISTER: u8 = 55;
const SET_DESCRIPTOR: u8 = 56;
const SEND_EVENT: u8 = 57;
const TIMEOUT: Duration = Duration::from_secs(1);

const TOUCH: u16 = 1;
const KEYBOARD: u16 = 2;

/// A touchscreen with one finger: tip switch, in range, contact identifier, X and Y from 0
/// to 32767, contact count. The contact identifier is what makes Linux's multitouch driver
/// take it, and Android then sees a screen to touch rather than a mouse.
const TOUCH_DESCRIPTOR: &[u8] = &[
    0x05, 0x0d, 0x09, 0x04, 0xa1, 0x01, 0x09, 0x22, 0xa1, 0x02, 0x09, 0x42, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x01, 0x81, 0x02, 0x09, 0x32, 0x81, 0x02, 0x95, 0x06, 0x81, 0x03,
    0x09, 0x51, 0x25, 0x01, 0x75, 0x08, 0x95, 0x01, 0x81, 0x02, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x16, 0x00, 0x00, 0x26, 0xff, 0x7f, 0x75, 0x10, 0x95, 0x02, 0x81, 0x02, 0xc0,
    0x05, 0x0d, 0x09, 0x54, 0x15, 0x00, 0x25, 0x01, 0x75, 0x08, 0x95, 0x01, 0x81, 0x02, 0xc0,
];

/// A boot keyboard: modifiers, a reserved byte, six keys.
const KEYBOARD_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x06,
    0x75, 0x08, 0x15, 0x00, 0x25, 0x65, 0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x81, 0x00, 0xc0,
];

/// The virtual devices on one phone, and where the finger and keys are.
pub struct Hid {
    dev: nusb::Device,
    at: (u16, u16),
    touching: bool,
    modifiers: u8,
    keys: Vec<u8>,
}

impl Hid {
    /// Plug the touchscreen and keyboard into the phone behind `dev` (in accessory mode or
    /// not).
    pub async fn register(dev: nusb::Device) -> Result<Self, String> {
        for (id, descriptor) in [(TOUCH, TOUCH_DESCRIPTOR), (KEYBOARD, KEYBOARD_DESCRIPTOR)] {
            out(&dev, REGISTER, id, descriptor.len() as u16, &[]).await.map_err(|e| format!("the phone takes no virtual input ({e})"))?;
            out(&dev, SET_DESCRIPTOR, id, 0, descriptor).await?;
        }
        // The phone makes the devices a moment later; input sent before then is refused
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(Hid { dev, at: (16_384, 16_384), touching: false, modifiers: 0, keys: Vec::new() })
    }

    pub async fn unregister(self) {
        for id in [TOUCH, KEYBOARD] {
            let _ = out(&self.dev, UNREGISTER, id, 0, &[]).await;
        }
    }

    /// One of the viewer's messages, as the phone should feel it.
    pub async fn apply(&mut self, msg: &ControlMsg) -> Result<(), String> {
        match *msg {
            ControlMsg::Pointer { x, y } => {
                self.at = (scale(x), scale(y));
                if self.touching {
                    self.touch().await?;
                }
            }
            ControlMsg::Button { button: MouseButton::Left, pressed } => {
                self.touching = pressed;
                self.touch().await?;
            }
            // Android goes back on Escape
            ControlMsg::Button { button: MouseButton::Right | MouseButton::Back, pressed: true } => self.tap_key(0x29).await?,
            ControlMsg::Scroll { dy, discrete, .. } => self.swipe(if discrete { dy * 0.08 } else { dy / 800.0 }).await?,
            ControlMsg::Key { keysym, pressed, code } => self.key(keysym, code, pressed).await?,
            ControlMsg::Text { ref text } => {
                for c in text.chars() {
                    if let Some((usage, shift)) = ascii_usage(c) {
                        self.send_keys(if shift { 0x02 } else { 0 }, &[usage]).await?;
                        self.send_keys(0, &[]).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn touch(&self) -> Result<(), String> {
        let (x, y) = self.at;
        // Tip switch and in range while touching, nothing (and no contact) otherwise
        let (flags, contacts) = if self.touching { (0b11, 1) } else { (0, 0) };
        let report = [flags, 0, x as u8, (x >> 8) as u8, y as u8, (y >> 8) as u8, contacts];
        out(&self.dev, SEND_EVENT, TOUCH, 0, &report).await
    }

    /// A wheel turn as a short finger drag, `fraction` of the screen's height.
    async fn swipe(&mut self, fraction: f64) -> Result<(), String> {
        let start = self.at;
        let distance = (fraction.clamp(-0.4, 0.4) * 32_767.0) as i32;
        self.touching = true;
        for step in 0..=6 {
            let y = (start.1 as i32 - distance * step / 6).clamp(0, 32_767) as u16;
            self.at = (start.0, y);
            self.touch().await?;
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
        self.touching = false;
        self.touch().await?;
        self.at = start;
        Ok(())
    }

    /// The physical key when the viewer gives it (the phone reads it with its own layout, as
    /// the computer did), else the keysym as on a US keyboard.
    async fn key(&mut self, keysym: u32, code: Option<u32>, pressed: bool) -> Result<(), String> {
        let (bit, usage) = match code {
            Some(code) => (code_modifier_bit(code), code_usage(code)),
            None => (modifier_bit(keysym), keysym_usage(keysym)),
        };
        if let Some(bit) = bit {
            if pressed { self.modifiers |= bit } else { self.modifiers &= !bit }
        } else if let Some(usage) = usage {
            self.keys.retain(|k| *k != usage);
            if pressed && self.keys.len() < 6 {
                self.keys.push(usage);
            }
        } else {
            return Ok(());
        }
        let keys = self.keys.clone();
        self.send_keys(self.modifiers, &keys).await
    }

    /// Ctrl+V: what the phone's clipboard holds goes where its cursor is.
    pub async fn paste(&self) -> Result<(), String> {
        // Give the clipboard a moment to land
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.send_keys(0x01, &[0x19]).await?;
        self.send_keys(0, &[]).await
    }

    async fn tap_key(&self, usage: u8) -> Result<(), String> {
        self.send_keys(0, &[usage]).await?;
        self.send_keys(0, &[]).await
    }

    async fn send_keys(&self, modifiers: u8, keys: &[u8]) -> Result<(), String> {
        let mut report = [0u8; 8];
        report[0] = modifiers;
        report[2..2 + keys.len()].copy_from_slice(keys);
        out(&self.dev, SEND_EVENT, KEYBOARD, 0, &report).await
    }
}

async fn out(dev: &nusb::Device, request: u8, value: u16, index: u16, data: &[u8]) -> Result<(), String> {
    dev.control_out(ControlOut { control_type: ControlType::Vendor, recipient: Recipient::Device, request, value, index, data }, TIMEOUT).await.map_err(|e| e.to_string())
}

/// 0..1 of the screen to the touchscreen's 0..32767.
fn scale(f: f64) -> u16 {
    (f.clamp(0.0, 1.0) * 32_767.0).round() as u16
}

fn modifier_bit(keysym: u32) -> Option<u8> {
    Some(match keysym {
        0xffe3 => 0x01,
        0xffe1 => 0x02,
        0xffe9 => 0x04,
        0xffeb => 0x08,
        0xffe4 => 0x10,
        0xffe2 => 0x20,
        0xffea | 0xfe03 => 0x40,
        0xffec => 0x80,
        _ => return None,
    })
}

fn code_modifier_bit(code: u32) -> Option<u8> {
    Some(match code {
        29 => 0x01,
        42 => 0x02,
        56 => 0x04,
        125 => 0x08,
        97 => 0x10,
        54 => 0x20,
        100 => 0x40,
        126 => 0x80,
        _ => return None,
    })
}

/// A Linux evdev key code (a key's place on the keyboard) to its USB keyboard usage.
fn code_usage(code: u32) -> Option<u8> {
    const LETTERS_ROW_1: [u8; 12] = [0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13, 0x2f, 0x30];
    const LETTERS_ROW_2: [u8; 12] = [0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f, 0x33, 0x34, 0x35];
    const LETTERS_ROW_3: [u8; 11] = [0x31, 0x1d, 0x1b, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38];
    Some(match code {
        1 => 0x29,
        2..=11 => 0x1e + (code - 2) as u8,
        12 => 0x2d,
        13 => 0x2e,
        14 => 0x2a,
        15 => 0x2b,
        16..=27 => LETTERS_ROW_1[(code - 16) as usize],
        28 => 0x28,
        30..=41 => LETTERS_ROW_2[(code - 30) as usize],
        43..=53 => LETTERS_ROW_3[(code - 43) as usize],
        55 => 0x55,
        57 => 0x2c,
        58 => 0x39,
        59..=68 => 0x3a + (code - 59) as u8,
        // The extra key of European keyboards (<> on AZERTY)
        86 => 0x64,
        87 => 0x44,
        88 => 0x45,
        96 => 0x58,
        102 => 0x4a,
        103 => 0x52,
        104 => 0x4b,
        105 => 0x50,
        106 => 0x4f,
        107 => 0x4d,
        108 => 0x51,
        109 => 0x4e,
        110 => 0x49,
        111 => 0x4c,
        _ => return None,
    })
}

/// An X11 keysym (what GTK and Qt give) to a USB keyboard usage, as on a US keyboard.
fn keysym_usage(keysym: u32) -> Option<u8> {
    let named = match keysym {
        0xff0d | 0xff8d => 0x28,
        0xff1b => 0x29,
        0xff08 => 0x2a,
        0xff09 => 0x2b,
        0xffff => 0x4c,
        0xff50 => 0x4a,
        0xff51 => 0x50,
        0xff52 => 0x52,
        0xff53 => 0x4f,
        0xff54 => 0x51,
        0xff55 => 0x4b,
        0xff56 => 0x4e,
        0xff57 => 0x4d,
        0xffbe..=0xffc9 => 0x3a + (keysym - 0xffbe) as u8,
        _ => 0,
    };
    if named != 0 {
        return Some(named);
    }
    char::from_u32(keysym).and_then(ascii_usage).map(|(usage, _)| usage)
}

/// A printable ASCII character to its key on a US keyboard, and whether it needs Shift.
fn ascii_usage(c: char) -> Option<(u8, bool)> {
    const SHIFTED: &str = "!@#$%^&*()_+{}|:\"~<>?";
    const PLAIN: &str = "1234567890-=[]\\;'`,./";
    let (c, shift) = match c {
        'A'..='Z' => (c.to_ascii_lowercase(), true),
        _ => match SHIFTED.find(c) {
            Some(i) => (PLAIN.as_bytes()[i] as char, true),
            None => (c, false),
        },
    };
    let usage = match c {
        'a'..='z' => 0x04 + (c as u8 - b'a'),
        '1'..='9' => 0x1e + (c as u8 - b'1'),
        '0' => 0x27,
        ' ' => 0x2c,
        '\n' => 0x28,
        '\t' => 0x2b,
        '-' => 0x2d,
        '=' => 0x2e,
        '[' => 0x2f,
        ']' => 0x30,
        '\\' => 0x31,
        ';' => 0x33,
        '\'' => 0x34,
        '`' => 0x35,
        ',' => 0x36,
        '.' => 0x37,
        '/' => 0x38,
        _ => return None,
    };
    Some((usage, shift))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_like_a_us_keyboard() {
        assert_eq!(ascii_usage('a'), Some((0x04, false)));
        assert_eq!(ascii_usage('Z'), Some((0x1d, true)));
        assert_eq!(ascii_usage('!'), Some((0x1e, true)));
        assert_eq!(ascii_usage('0'), Some((0x27, false)));
        assert_eq!(ascii_usage('?'), Some((0x38, true)));
        assert_eq!(keysym_usage(0xff0d), Some(0x28));
        assert_eq!(keysym_usage(0xffc9), Some(0x45));
        assert_eq!(keysym_usage('q' as u32), Some(0x14));
        assert_eq!(ascii_usage('é'), None);
    }

    #[test]
    fn physical_keys_keep_their_place() {
        // KEY_Q, KEY_A, KEY_Z, KEY_M: the same places on QWERTY and AZERTY
        assert_eq!(code_usage(16), Some(0x14));
        assert_eq!(code_usage(30), Some(0x04));
        assert_eq!(code_usage(44), Some(0x1d));
        assert_eq!(code_usage(50), Some(0x10));
        assert_eq!(code_usage(39), Some(0x33));
        assert_eq!(code_usage(86), Some(0x64));
        assert_eq!(code_usage(2), Some(0x1e));
        assert_eq!(code_usage(11), Some(0x27));
        assert_eq!(code_modifier_bit(42), Some(0x02));
    }

    #[test]
    fn pointer_fills_the_touchscreen() {
        assert_eq!(scale(0.0), 0);
        assert_eq!(scale(1.0), 32_767);
        assert_eq!(scale(2.0), 32_767);
    }
}
