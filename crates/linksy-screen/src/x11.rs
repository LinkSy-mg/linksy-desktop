//! X11 desktops (Openbox, XFCE, i3…), where no ScreenCast portal works: the screen through
//! GStreamer's ximagesrc, the viewer's input through XTest. X11 lets any program on the
//! display do both; Linksy has asked before the session started.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use linksy_core::{ControlMsg, protocol::MouseButton};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::randr::{ConnectionExt as _, MonitorInfo};
use x11rb::protocol::xproto::{BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt as _, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT, Window};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// Whether this is an X11 session (the way the rest of Linksy tells: no Wayland display).
pub fn session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some()
}

/// A monitor in the root window's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

fn monitor(conn: &RustConnection, root: Window, screen: (u16, u16)) -> Area {
    let monitors = conn.randr_get_monitors(root, true).ok().and_then(|c| c.reply().ok()).map(|r| r.monitors).unwrap_or_default();
    pick(&monitors, screen)
}

/// The primary monitor (else the first), or the whole screen without RandR.
fn pick(monitors: &[MonitorInfo], screen: (u16, u16)) -> Area {
    match monitors.iter().find(|m| m.primary).or(monitors.first()) {
        Some(m) if m.width > 0 && m.height > 0 => Area { x: m.x, y: m.y, width: m.width, height: m.height },
        _ => Area { x: 0, y: 0, width: screen.0, height: screen.1 },
    }
}

/// The capture for the shared monitor, and its input when `control`.
pub fn capture(control: bool) -> Result<(String, Option<Control>), String> {
    let input = Control::open()?;
    let a = input.area;
    eprintln!("linksy-screen: X11, sharing {}x{} at {},{}", a.width, a.height, a.x, a.y);
    // Damage tracking sends only what changed, which leaves stale strips on some drivers
    let source = format!(
        "ximagesrc use-damage=false show-pointer=true startx={} starty={} endx={} endy={} ! video/x-raw,framerate=30/1",
        a.x,
        a.y,
        a.x as i32 + a.width as i32 - 1,
        a.y as i32 + a.height as i32 - 1
    );
    Ok((source, control.then_some(input)))
}

/// Keysyms to the keycodes that type them (level 1 needs Shift), and the keycodes with
/// nothing on them, borrowed in turn for anything else (as xdotool does).
struct Keymap {
    codes: HashMap<u32, (u8, bool)>,
    spares: Vec<u8>,
    next: usize,
    /// When each spare last typed: apps read a key's meaning when they get to its event,
    /// so a spare isn't given a new one too soon.
    used: HashMap<u8, std::time::Instant>,
    shift: Option<u8>,
}

const SHIFT_L: u32 = 0xffe1;

/// The viewer's pointer and keyboard, on this display.
pub struct Control {
    conn: RustConnection,
    root: Window,
    pub area: Area,
    keymap: Mutex<Keymap>,
    /// Keycodes down: released when the session ends.
    held: Mutex<HashSet<u8>>,
    /// Smooth scrolling not yet a whole wheel click.
    scrolled: Mutex<(f64, f64)>,
}

impl Control {
    pub fn open() -> Result<Self, String> {
        let (conn, n) = x11rb::connect(None).map_err(|e| format!("can't open the X display: {e}"))?;
        if conn.extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME).ok().flatten().is_none() {
            return Err("this X server has no XTest, so the other device can't control it".into());
        }
        let screen = &conn.setup().roots[n];
        let root = screen.root;
        let area = monitor(&conn, root, (screen.width_in_pixels, screen.height_in_pixels));
        let keymap = Mutex::new(keymap(&conn)?);
        Ok(Self { conn, root, area, keymap, held: Default::default(), scrolled: Default::default() })
    }

    pub fn apply(&self, msg: &ControlMsg) -> Result<(), String> {
        match *msg {
            ControlMsg::Pointer { x, y } => {
                let a = self.area;
                let x = a.x as f64 + (x.clamp(0.0, 1.0) * a.width as f64).min(a.width as f64 - 1.0);
                let y = a.y as f64 + (y.clamp(0.0, 1.0) * a.height as f64).min(a.height as f64 - 1.0);
                self.fake(MOTION_NOTIFY_EVENT, 0, x as i16, y as i16)?;
            }
            // Relative motion: detail 1
            ControlMsg::PointerBy { dx, dy } => self.fake(MOTION_NOTIFY_EVENT, 1, dx.round() as i16, dy.round() as i16)?,
            ControlMsg::Button { button, pressed } => {
                let n = match button {
                    MouseButton::Left => 1,
                    MouseButton::Middle => 2,
                    MouseButton::Right => 3,
                    MouseButton::Back => 8,
                    MouseButton::Forward => 9,
                };
                self.fake(if pressed { BUTTON_PRESS_EVENT } else { BUTTON_RELEASE_EVENT }, n, 0, 0)?;
            }
            ControlMsg::Scroll { dx, dy, discrete: true } => self.wheel(dx.round() as i32, dy.round() as i32)?,
            ControlMsg::Scroll { dx, dy, discrete: false } => {
                // About 10 pixels a click, as Wayland desktops count a wheel's
                let (cx, cy) = {
                    let mut s = self.scrolled.lock().unwrap();
                    s.0 += dx / 10.0;
                    s.1 += dy / 10.0;
                    let whole = (s.0.trunc(), s.1.trunc());
                    s.0 -= whole.0;
                    s.1 -= whole.1;
                    whole
                };
                self.wheel(cx as i32, cy as i32)?;
            }
            ControlMsg::Key { keysym, pressed } => self.key(keysym, pressed)?,
            ControlMsg::Text { ref text } => {
                for c in text.chars() {
                    let keysym = crate::portal::keysym(c);
                    self.key(keysym, true)?;
                    self.key(keysym, false)?;
                }
            }
            ControlMsg::TermInput { .. } | ControlMsg::TermResize { .. } | ControlMsg::TermClose => {}
        }
        self.conn.flush().map_err(|e| e.to_string())
    }

    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) -> Result<(), String> {
        self.conn.xtest_fake_input(kind, detail, x11rb::CURRENT_TIME, self.root, x, y, 0).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Wheel clicks: buttons 4 and 5 up and down, 6 and 7 left and right.
    fn wheel(&self, dx: i32, dy: i32) -> Result<(), String> {
        for (steps, less, more) in [(dy, 4, 5), (dx, 6, 7)] {
            let button = if steps < 0 { less } else { more };
            for _ in 0..steps.unsigned_abs().min(50) {
                self.fake(BUTTON_PRESS_EVENT, button, 0, 0)?;
                self.fake(BUTTON_RELEASE_EVENT, button, 0, 0)?;
            }
        }
        Ok(())
    }

    fn key(&self, keysym: u32, pressed: bool) -> Result<(), String> {
        let mut map = self.keymap.lock().unwrap();
        let (code, shifted) = match map.codes.get(&keysym) {
            Some(&found) => found,
            None => {
                let Some(&spare) = map.spares.get(map.next % map.spares.len().max(1)) else { return Err(format!("no free key to type keysym {keysym:#x}")) };
                map.next += 1;
                if let Some(at) = map.used.get(&spare) {
                    std::thread::sleep(std::time::Duration::from_millis(150).saturating_sub(at.elapsed()));
                }
                // Both levels, so Shift held on this computer doesn't change it
                self.conn.change_keyboard_mapping(1, spare, 2, &[keysym, keysym]).map_err(|e| e.to_string())?;
                self.conn.get_input_focus().map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
                // Apps hear of the new mapping in a moment
                std::thread::sleep(std::time::Duration::from_millis(20));
                // The previous borrower's keysym types something else now
                map.codes.retain(|_, (c, _)| *c != spare);
                map.codes.insert(keysym, (spare, false));
                (spare, false)
            }
        };
        // Shift for the key's second level, unless the viewer holds Shift already
        let shift = map.shift.filter(|s| shifted && pressed && !self.held.lock().unwrap().contains(s));
        if let Some(shift) = shift {
            self.fake(KEY_PRESS_EVENT, shift, 0, 0)?;
        }
        self.fake(if pressed { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT }, code, 0, 0)?;
        if let Some(shift) = shift {
            self.fake(KEY_RELEASE_EVENT, shift, 0, 0)?;
        }
        if map.spares.contains(&code) {
            map.used.insert(code, std::time::Instant::now());
        }
        drop(map);
        let mut held = self.held.lock().unwrap();
        if pressed { held.insert(code) } else { held.remove(&code) };
        Ok(())
    }

    /// Release what's still down: a release that never came can't leave a key held here.
    pub fn release_keys(&self) {
        let keys: Vec<u8> = self.held.lock().unwrap().drain().collect();
        for code in keys {
            let _ = self.fake(KEY_RELEASE_EVENT, code, 0, 0);
        }
        // The borrowed keys go back to typing nothing
        let mut map = self.keymap.lock().unwrap();
        let borrowed: Vec<u8> = map.used.drain().map(|(code, _)| code).collect();
        for code in borrowed {
            let _ = self.conn.change_keyboard_mapping(1, code, 2, &[0, 0]);
        }
        let Keymap { codes, spares, .. } = &mut *map;
        codes.retain(|_, (c, _)| !spares.contains(c));
        let _ = self.conn.flush();
    }
}

fn keymap(conn: &RustConnection) -> Result<Keymap, String> {
    let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let reply = conn.get_keyboard_mapping(min, max - min + 1).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
    let per = reply.keysyms_per_keycode.max(1) as usize;
    let mut codes = HashMap::new();
    // The free keycodes, highest first, away from the keys people have
    let mut spares: Vec<u8> = reply.keysyms.chunks(per).enumerate().filter(|(_, syms)| syms.iter().all(|&s| s == 0)).map(|(i, _)| min + i as u8).collect();
    spares.reverse();
    // Unshifted first, wherever it is
    for level in 0..2 {
        for (i, syms) in reply.keysyms.chunks(per).enumerate() {
            if let Some(&sym) = syms.get(level).filter(|&&s| s != 0) {
                codes.entry(sym).or_insert((min + i as u8, level == 1));
            }
        }
    }
    let shift = codes.get(&SHIFT_L).map(|&(c, _)| c);
    Ok(Keymap { codes, spares, next: 0, used: HashMap::new(), shift })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(primary: bool, x: i16, width: u16) -> MonitorInfo {
        MonitorInfo { name: 0, primary, automatic: false, x, y: 0, width, height: 900, width_in_millimeters: 0, height_in_millimeters: 0, outputs: vec![] }
    }

    #[test]
    fn shares_the_primary_monitor() {
        let both = [monitor(false, 0, 1920), monitor(true, 1920, 2560)];
        assert_eq!(pick(&both, (4480, 1440)), Area { x: 1920, y: 0, width: 2560, height: 900 });
        let none_primary = [monitor(false, 0, 1920), monitor(false, 1920, 2560)];
        assert_eq!(pick(&none_primary, (4480, 1440)).x, 0);
        assert_eq!(pick(&[], (1366, 768)), Area { x: 0, y: 0, width: 1366, height: 768 });
    }
}
