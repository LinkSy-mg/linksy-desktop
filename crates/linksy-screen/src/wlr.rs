//! Hyprland, Sway and other wlroots desktops, whose portals share the screen but can't drive
//! it: the viewer's pointer and keyboard through the compositor's virtual pointer
//! (wlr-virtual-pointer) and virtual keyboard (virtual-keyboard-unstable-v1), as wayvnc
//! does. Both belong to the session; nothing outside it can type with them.

use std::collections::HashSet;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd};
use std::sync::Mutex;
use std::time::Instant;

use linksy_core::ControlMsg;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

#[allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, unused_imports, unused_unsafe, unused_variables, clippy::all)]
mod keyboard {
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/virtual-keyboard-unstable-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/virtual-keyboard-unstable-v1.xml");
}

use keyboard::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use keyboard::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

/// Where an output is on the desktop, as the compositor says.
#[derive(Default, Debug, Clone, PartialEq)]
struct Place {
    name: Option<String>,
    position: Option<(i32, i32)>,
    logical: Option<(i32, i32)>,
    /// Its current mode, in pixels.
    mode: Option<(i32, i32)>,
}

#[derive(Default)]
struct State {
    places: Vec<Place>,
    outputs: Vec<WlOutput>,
}

/// This desktop's virtual pointer and keyboard, when it has both.
pub struct Desktop {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    seat: WlSeat,
    pointers: ZwlrVirtualPointerManagerV1,
    keyboards: ZwpVirtualKeyboardManagerV1,
}

/// The shared screen, to find its output: by name (a headless output Linksy made), or where
/// the portal says the stream is, and its size.
#[derive(Default)]
pub struct Want {
    pub name: Option<String>,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
}

/// Hyprland, Sway, river, labwc, Wayfire: the desktop's virtual input. None on GNOME and KDE
/// (their RemoteDesktop portal drives the screen there) and outside Wayland.
pub fn desktop() -> Option<Desktop> {
    let conn = crate::outputs::connect().ok()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
    let qh = queue.handle();
    let pointers = globals.bind::<ZwlrVirtualPointerManagerV1, _, _>(&qh, 1..=2, ()).ok()?;
    let keyboards = globals.bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ()).ok()?;
    let seat = globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()).ok()?;
    let xdg = globals.bind::<ZxdgOutputManagerV1, _, _>(&qh, 2..=3, ()).ok();
    let mut state = State::default();
    for global in globals.contents().clone_list().into_iter().filter(|g| g.interface == WlOutput::interface().name) {
        let i = state.outputs.len();
        let output: WlOutput = globals.registry().bind(global.name, global.version.min(4), &qh, i);
        if let Some(xdg) = &xdg {
            xdg.get_xdg_output(&output, &qh, i);
        }
        state.outputs.push(output);
        state.places.push(Place::default());
    }
    queue.roundtrip(&mut state).ok()?;
    Some(Desktop { conn, queue, state, seat, pointers, keyboards })
}

/// The output `want` is: by name, else the one at its position, else the one its size,
/// else the only one. None: the pointer spans the whole desktop.
fn pick(places: &[Place], want: &Want) -> Option<usize> {
    if let Some(name) = &want.name {
        return places.iter().position(|p| p.name.as_ref() == Some(name));
    }
    let sized = |p: &Place| want.size.is_some_and(|s| p.logical == Some(s) || p.mode == Some(s));
    let only = |found: Vec<usize>| (found.len() == 1).then(|| found[0]);
    only((0..places.len()).filter(|&i| want.position.is_some() && places[i].position == want.position).collect())
        .or_else(|| only((0..places.len()).filter(|&i| sized(&places[i])).collect()))
        .or_else(|| (places.len() == 1).then_some(0))
}

/// Absolute positions, as a share of this.
const EXTENT: u32 = 0xffff;

/// Keycodes stop at 255 (X11 apps, through Xwayland); the first is 8.
const MAX_KEYS: usize = 247;

/// The keys typed most, ready before the first: the printable ASCII, the controls, F1 to
/// F12 and the modifiers. Anything else gets a key when it's typed.
fn usual_keys() -> Vec<u32> {
    let controls = [0xff0d, 0xff08, 0xff09, 0xff1b, 0xffff, 0xff50, 0xff51, 0xff52, 0xff53, 0xff54, 0xff55, 0xff56, 0xff57, 0xff63, 0xff61, 0xff67];
    let modifiers = [0xffe1, 0xffe2, 0xffe3, 0xffe4, 0xffe5, 0xffe9, 0xffea, 0xffeb, 0xffec, 0xfe03];
    (0x20..=0x7e).chain(controls).chain(0xffbe..=0xffc9).chain(modifiers).collect()
}

/// The modifier a keysym holds, as its bit (Shift, Lock, Control, Mod1 for Alt, Mod4 for
/// Super, Mod5 for AltGr).
fn modifier(keysym: u32) -> Option<(u32, &'static str)> {
    match keysym {
        0xffe1 | 0xffe2 => Some((1 << 0, "Shift")),
        0xffe5 => Some((1 << 1, "Lock")),
        0xffe3 | 0xffe4 => Some((1 << 2, "Control")),
        0xffe7..=0xffea => Some((1 << 3, "Mod1")),
        0xffeb | 0xffec => Some((1 << 6, "Mod4")),
        0xfe03 => Some((1 << 7, "Mod5")),
        _ => None,
    }
}

/// An XKB keymap with a key for each of `keysyms`, in order: evdev code 1 for the first.
/// Lowercase letters have their capital on the second level, for Shift held by the viewer.
fn keymap(keysyms: &[u32]) -> String {
    let mut codes = String::new();
    let mut symbols = String::new();
    for (i, &sym) in keysyms.iter().enumerate() {
        codes += &format!("<K{i}> = {};\n", i + 9);
        symbols += &match sym {
            0x61..=0x7a => format!("key <K{i}> {{ [ {sym:#x}, {:#x} ] }};\n", sym - 0x20),
            _ => format!("key <K{i}> {{ [ {sym:#x} ] }};\n"),
        };
        if let Some((_, name)) = modifier(sym) {
            symbols += &format!("modifier_map {name} {{ <K{i}> }};\n");
        }
    }
    format!(
        "xkb_keymap {{\nxkb_keycodes \"linksy\" {{\nminimum = 8;\nmaximum = {};\n{codes}}};\nxkb_types \"linksy\" {{ include \"complete\" }};\nxkb_compatibility \"linksy\" {{ include \"complete\" }};\nxkb_symbols \"linksy\" {{\n{symbols}}};\n}};\n",
        keysyms.len() + 8
    )
}

struct Keys {
    /// The keymap's keysyms: evdev code `i + 1` types `keysyms[i]`.
    keysyms: Vec<u32>,
    /// Codes down: released when the session ends.
    held: HashSet<u32>,
}

/// The viewer's pointer and keyboard, on this desktop.
pub struct Control {
    conn: Connection,
    pointer: ZwlrVirtualPointerV1,
    keyboard: ZwpVirtualKeyboardV1,
    keys: Mutex<Keys>,
    start: Instant,
}

impl Desktop {
    /// Input for the screen `want` is: absolute positions land on its output.
    pub fn control(self, want: &Want) -> Result<Control, String> {
        let Desktop { conn, mut queue, mut state, seat, pointers, keyboards } = self;
        let qh = queue.handle();
        let found = pick(&state.places, want);
        let output = found.filter(|_| pointers.version() >= 2).map(|i| state.outputs[i].clone());
        let looked = want.name.is_some() || want.position.is_some() || want.size.is_some();
        match found.and_then(|i| state.places[i].name.clone()) {
            Some(name) => eprintln!("linksy-screen: the viewer's pointer and keyboard drive {name} (virtual input)"),
            None if looked => eprintln!("linksy-screen: the viewer's pointer and keyboard drive this desktop (virtual input); the shared screen's output wasn't found, so taps span every screen"),
            None => eprintln!("linksy-screen: the viewer's pointer and keyboard drive this desktop (virtual input)"),
        }
        let pointer = match &output {
            Some(o) => pointers.create_virtual_pointer_with_output(Some(&seat), Some(o), &qh, ()),
            None => pointers.create_virtual_pointer(Some(&seat), &qh, ()),
        };
        let keyboard = keyboards.create_virtual_keyboard(&seat, &qh, ());
        let control = Control { conn, pointer, keyboard, keys: Mutex::new(Keys { keysyms: usual_keys(), held: HashSet::new() }), start: Instant::now() };
        control.upload(&control.keys.lock().unwrap().keysyms)?;
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        // Events (outputs changing, an error) are read on their own, so they never pile up
        std::thread::spawn(move || while queue.blocking_dispatch(&mut state).is_ok() {});
        Ok(control)
    }
}

impl Control {
    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    pub fn apply(&self, msg: &ControlMsg) -> Result<(), String> {
        let (p, t) = (&self.pointer, self.time());
        match *msg {
            ControlMsg::Pointer { x, y } => {
                let at = |v: f64| ((v.clamp(0.0, 1.0) * EXTENT as f64) as u32).min(EXTENT - 1);
                p.motion_absolute(t, at(x), at(y), EXTENT, EXTENT);
                p.frame();
            }
            ControlMsg::PointerBy { dx, dy } => {
                p.motion(t, dx, dy);
                p.frame();
            }
            ControlMsg::Button { button, pressed } => {
                p.button(t, button.evdev() as u32, if pressed { ButtonState::Pressed } else { ButtonState::Released });
                p.frame();
            }
            ControlMsg::Scroll { dx, dy, discrete: true } => {
                p.axis_source(AxisSource::Wheel);
                for (axis, steps) in [(Axis::VerticalScroll, dy.round() as i32), (Axis::HorizontalScroll, dx.round() as i32)] {
                    if steps != 0 {
                        p.axis_discrete(t, axis, steps as f64 * 15.0, steps);
                    }
                }
                p.frame();
            }
            ControlMsg::Scroll { dx, dy, discrete: false } => {
                p.axis_source(AxisSource::Continuous);
                for (axis, value) in [(Axis::VerticalScroll, dy), (Axis::HorizontalScroll, dx)] {
                    if value != 0.0 {
                        p.axis(t, axis, value);
                    }
                }
                p.frame();
            }
            ControlMsg::Key { keysym, pressed, .. } => self.key(keysym, pressed)?,
            ControlMsg::Text { ref text } => {
                for keysym in text.chars().map(crate::portal::keysym) {
                    self.key(keysym, true)?;
                    self.key(keysym, false)?;
                }
            }
            ControlMsg::TermInput { .. } | ControlMsg::TermResize { .. } | ControlMsg::TermClose => {}
        }
        self.conn.flush().map_err(|e| e.to_string())
    }

    fn key(&self, keysym: u32, pressed: bool) -> Result<(), String> {
        let mut keys = self.keys.lock().unwrap();
        let code = match keys.keysyms.iter().position(|&s| s == keysym) {
            Some(i) => i as u32 + 1,
            None => {
                if keys.keysyms.len() >= MAX_KEYS {
                    // Full: back to the usual keys, nothing held across the change
                    for code in std::mem::take(&mut keys.held) {
                        self.keyboard.key(self.time(), code, 0);
                    }
                    keys.keysyms = usual_keys();
                }
                keys.keysyms.push(keysym);
                self.upload(&keys.keysyms)?;
                keys.keysyms.len() as u32
            }
        };
        self.keyboard.key(self.time(), code, pressed as u32);
        if pressed { keys.held.insert(code) } else { keys.held.remove(&code) };
        if modifier(keysym).is_some() {
            let mods = keys.held.iter().filter_map(|&c| modifier(keys.keysyms[c as usize - 1])).fold(0, |all, (bit, _)| all | bit);
            self.keyboard.modifiers(mods, 0, 0, 0);
        }
        Ok(())
    }

    /// Give the compositor the keymap for `keysyms`.
    fn upload(&self, keysyms: &[u32]) -> Result<(), String> {
        let text = keymap(keysyms);
        // SAFETY: a new file of our own, owned by `file` from here
        let mut file = unsafe {
            let fd = libc::memfd_create(c"linksy-keymap".as_ptr(), libc::MFD_CLOEXEC);
            if fd < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            std::fs::File::from_raw_fd(fd)
        };
        file.write_all(text.as_bytes()).and_then(|_| file.write_all(&[0])).map_err(|e| e.to_string())?;
        // Format 1: XKB, as text
        self.keyboard.keymap(1, file.as_fd(), text.len() as u32 + 1);
        Ok(())
    }

    /// Release what's still down: a release that never came can't leave a key held here.
    pub fn release_keys(&self) {
        let mut keys = self.keys.lock().unwrap();
        for code in std::mem::take(&mut keys.held) {
            self.keyboard.key(self.time(), code, 0);
        }
        self.keyboard.modifiers(0, 0, 0, 0);
        let _ = self.conn.flush();
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        self.pointer.destroy();
        self.keyboard.destroy();
        let _ = self.conn.flush();
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlOutput, usize> for State {
    fn event(state: &mut Self, _: &WlOutput, event: wl_output::Event, &i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_output::Event::Mode { flags: WEnum::Value(flags), width, height, .. } if flags.contains(wl_output::Mode::Current) => state.places[i].mode = Some((width, height)),
            wl_output::Event::Name { name } => state.places[i].name = Some(name),
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, usize> for State {
    fn event(state: &mut Self, _: &ZxdgOutputV1, event: zxdg_output_v1::Event, &i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => state.places[i].position = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => state.places[i].logical = Some((width, height)),
            zxdg_output_v1::Event::Name { name } => {
                state.places[i].name.get_or_insert(name);
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ZxdgOutputManagerV1);
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    fn place(name: &str, position: (i32, i32), logical: (i32, i32), mode: (i32, i32)) -> Place {
        Place { name: Some(name.into()), position: Some(position), logical: Some(logical), mode: Some(mode) }
    }

    #[test]
    fn finds_the_shared_output() {
        let two = [place("DP-1", (0, 0), (1920, 1080), (1920, 1080)), place("HDMI-A-1", (1920, 0), (1280, 720), (2560, 1440))];
        let want = |name: Option<&str>, position, size| Want { name: name.map(Into::into), position, size };
        // A headless output by its name
        assert_eq!(pick(&two, &want(Some("HDMI-A-1"), None, None)), Some(1));
        assert_eq!(pick(&two, &want(Some("HEADLESS-1"), None, None)), None);
        // Where the portal says the stream is
        assert_eq!(pick(&two, &want(None, Some((1920, 0)), Some((1920, 1080)))), Some(1));
        // Only its size, in pixels or on the desktop
        assert_eq!(pick(&two, &want(None, None, Some((2560, 1440)))), Some(1));
        assert_eq!(pick(&two, &want(None, None, Some((1920, 1080)))), Some(0));
        // Two the same: neither, unless one is where the stream is
        let twins = [place("DP-1", (0, 0), (1920, 1080), (1920, 1080)), place("DP-2", (1920, 0), (1920, 1080), (1920, 1080))];
        assert_eq!(pick(&twins, &want(None, None, Some((1920, 1080)))), None);
        assert_eq!(pick(&twins, &want(None, Some((0, 0)), Some((1920, 1080)))), Some(0));
        // One output: that one
        assert_eq!(pick(&twins[..1], &Want::default()), Some(0));
    }

    #[test]
    fn keymap_has_a_key_for_each() {
        let map = keymap(&[0x61, 0xf1, 0x10020ac, 0xffe1]);
        assert!(map.contains("maximum = 12;"));
        assert!(map.contains("<K0> = 9;") && map.contains("<K3> = 12;"));
        assert!(map.contains("key <K0> { [ 0x61, 0x41 ] };"));
        assert!(map.contains("key <K2> { [ 0x10020ac ] };"));
        assert!(map.contains("modifier_map Shift { <K3> };"));
        assert!(usual_keys().len() < MAX_KEYS);
    }
}

/// Needs a wlroots compositor, e.g. a headless Sway (`WLR_BACKENDS=headless sway`), with
/// WAYLAND_DISPLAY set to it.
#[cfg(test)]
mod compositor_tests {
    use super::*;

    #[test]
    #[ignore = "needs Sway or another wlroots compositor"]
    fn types_and_points_on_a_named_output() {
        let name = std::env::var("LINKSY_TEST_OUTPUT").ok();
        let control = desktop().expect("no virtual input here").control(&Want { name, ..Default::default() }).unwrap();
        for msg in [
            ControlMsg::Pointer { x: 0.5, y: 0.5 },
            ControlMsg::Button { button: linksy_core::protocol::MouseButton::Left, pressed: true },
            ControlMsg::Button { button: linksy_core::protocol::MouseButton::Left, pressed: false },
            ControlMsg::Text { text: "Hi, ñ€ç ÑÉ!".into() },
            ControlMsg::Key { keysym: 0xffe1, pressed: true, code: None },
            ControlMsg::Text { text: "a".into() },
            ControlMsg::Key { keysym: 0xffe1, pressed: false, code: None },
            ControlMsg::Scroll { dx: 0.0, dy: 2.0, discrete: true },
            ControlMsg::Scroll { dx: 0.0, dy: 30.0, discrete: false },
            ControlMsg::PointerBy { dx: 10.0, dy: -5.0 },
        ] {
            control.apply(&msg).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        control.release_keys();
    }
}
