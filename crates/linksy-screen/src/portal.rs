//! The desktop portals: ScreenCast to see a screen, RemoteDesktop to see it and drive its
//! pointer and keyboard. The desktop asks the user both times; a restore token remembers
//! the answer.

use std::os::fd::OwnedFd;

use ashpd::desktop::PersistMode;
use ashpd::desktop::Session;
use ashpd::desktop::remote_desktop::{Axis, DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType, StartCastOptions, Stream};
use linksy_core::ControlMsg;

pub struct Screen {
    /// PipeWire remote to read the screen from.
    pub fd: OwnedFd,
    pub node: u32,
    pub restore_token: Option<String>,
    pub input: Option<Input>,
    /// Where the shared monitor is on the desktop, and its size, when the portal says.
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
}

/// What the user allowed in the desktop's dialog: without pointer and keyboard ("Allow
/// remote interaction" off), the remembered choice is dropped so the dialog asks again.
pub fn input_allowed(granted: ashpd::enumflags2::BitFlags<DeviceType>) -> bool {
    granted.contains(DeviceType::Pointer) && granted.contains(DeviceType::Keyboard)
}

/// Where the viewer's input goes.
pub enum Input {
    /// Test mode: print it (linksyd logs the helper's stderr).
    Log,
    Desktop {
        proxy: RemoteDesktop,
        session: Session<RemoteDesktop>,
        node: u32,
        /// The stream's size in the compositor's coordinates, for absolute positions.
        size: crate::outputs::Size,
        /// Whether the user allowed input; when not, it's said once and dropped.
        allowed: bool,
        /// Keys pressed and not released yet: released when the session ends, so a release
        /// that never came (mobile#16) can't leave Shift held on this computer.
        held: std::sync::Mutex<std::collections::HashSet<u32>>,
    },
    /// An X11 display, driven through XTest.
    X11(Box<crate::x11::Control>),
    /// Hyprland, Sway: the compositor's virtual pointer and keyboard.
    Wlr(Box<crate::wlr::Control>),
}

static REFUSED_SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl Input {
    pub async fn apply(&self, msg: &ControlMsg) {
        let applied = match self {
            Self::X11(display) => Some(display.apply(msg)),
            Self::Wlr(desktop) => Some(desktop.apply(msg)),
            _ => None,
        };
        if let Some(applied) = applied {
            if let Err(e) = applied {
                eprintln!("linksy-screen: input not applied: {e}");
            }
            return;
        }
        let Self::Desktop { proxy, session, node, size, allowed, held } = self else {
            eprintln!("input: {msg:?}");
            return;
        };
        if !allowed {
            if !REFUSED_SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("linksy-screen: remote interaction wasn't allowed in the sharing dialog; input from the other device is ignored (it asks again next time)");
            }
            return;
        }
        let result = match *msg {
            ControlMsg::Pointer { x, y } => match { *size.lock().unwrap() } {
                Some((w, h)) => {
                    // Inside the stream: the portal refuses its right and bottom edge
                    // (x = width), where a touchpad drag to the edge ends up (KDE: "Invalid position")
                    let (x, y) = ((x.clamp(0.0, 1.0) * w).min(w - 1.0), (y.clamp(0.0, 1.0) * h).min(h - 1.0));
                    proxy.notify_pointer_motion_absolute(session, *node, x, y, Default::default()).await
                }
                None => Ok(()),
            },
            ControlMsg::Button { button, pressed } => proxy.notify_pointer_button(session, button.evdev(), state(pressed), Default::default()).await,
            ControlMsg::Scroll { dx, dy, discrete: true } => {
                let mut r = Ok(());
                if dy != 0.0 {
                    r = proxy.notify_pointer_axis_discrete(session, Axis::Vertical, dy.round() as i32, Default::default()).await;
                }
                if dx != 0.0 && r.is_ok() {
                    r = proxy.notify_pointer_axis_discrete(session, Axis::Horizontal, dx.round() as i32, Default::default()).await;
                }
                r
            }
            ControlMsg::Scroll { dx, dy, discrete: false } => proxy.notify_pointer_axis(session, dx, dy, Default::default()).await,
            ControlMsg::Key { keysym, pressed } => {
                {
                    let mut down = held.lock().unwrap();
                    if pressed { down.insert(keysym) } else { down.remove(&keysym) };
                }
                proxy.notify_keyboard_keysym(session, keysym as i32, state(pressed), Default::default()).await
            }
            ControlMsg::PointerBy { dx, dy } => proxy.notify_pointer_motion(session, dx, dy, Default::default()).await,
            ControlMsg::Text { ref text } => {
                let mut r = Ok(());
                for keysym in text.chars().map(keysym) {
                    r = proxy.notify_keyboard_keysym(session, keysym as i32, KeyState::Pressed, Default::default()).await;
                    if r.is_ok() {
                        r = proxy.notify_keyboard_keysym(session, keysym as i32, KeyState::Released, Default::default()).await;
                    }
                    if r.is_err() {
                        break;
                    }
                }
                r
            }
            // Terminal messages: never sent to a screen.
            ControlMsg::TermInput { .. } | ControlMsg::TermResize { .. } | ControlMsg::TermClose => Ok(()),
        };
        if let Err(e) = result {
            eprintln!("linksy-screen: input not applied: {e}");
        }
    }
}

/// The X11 keysym that types `c`: Latin-1 as itself, a few controls by name, the rest
/// through the Unicode keysym range.
pub(crate) fn keysym(c: char) -> u32 {
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        '\u{8}' => 0xff08,
        ' '..='~' | '\u{a0}'..='\u{ff}' => c as u32,
        _ => 0x0100_0000 + c as u32,
    }
}

/// Pointer and keyboard only, no screen: for `input` sessions (a phone as touchpad).
pub async fn input_only(token: Option<String>) -> Result<(Input, Option<String>), String> {
    let remote = RemoteDesktop::new().await.map_err(|e| format!("remote control isn't available: {e}"))?;
    let session = remote.create_session(Default::default()).await.map_err(|e| e.to_string())?;
    let devices = SelectDevicesOptions::default()
        .set_devices(Some(DeviceType::Keyboard | DeviceType::Pointer))
        .set_persist_mode(PersistMode::ExplicitlyRevoked)
        .set_restore_token(token.as_deref());
    remote.select_devices(&session, devices).await.map_err(|e| e.to_string())?;
    let selected = remote
        .start(&session, None, Default::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|_| "remote control was cancelled".to_string())?;
    let allowed = input_allowed(selected.devices());
    let restore_token = if allowed { selected.restore_token().map(str::to_string) } else { None };
    Ok((Input::Desktop { proxy: remote, session, node: 0, size: Default::default(), allowed, held: Default::default() }, restore_token))
}

fn state(pressed: bool) -> KeyState {
    if pressed { KeyState::Pressed } else { KeyState::Released }
}

/// Ask for a monitor (a new virtual one with `virtual_monitor`), and with `control` for
/// its pointer and keyboard too.
pub async fn screen(token: Option<String>, control: bool, virtual_monitor: bool) -> Result<Screen, String> {
    let source = if virtual_monitor { SourceType::Virtual } else { SourceType::Monitor };
    let sources = SelectSourcesOptions::default().set_cursor_mode(CursorMode::Embedded).set_sources(Some(source.into())).set_multiple(false);
    let screencast = Screencast::new().await.map_err(|e| format!("screen sharing isn't available: {e}"))?;
    if !control {
        let session = screencast.create_session(Default::default()).await.map_err(|e| e.to_string())?;
        let sources = sources.set_persist_mode(PersistMode::ExplicitlyRevoked).set_restore_token(token.as_deref());
        screencast.select_sources(&session, sources).await.map_err(|e| e.to_string())?;
        let streams = screencast
            .start(&session, None, StartCastOptions::default())
            .await
            .map_err(|e| e.to_string())?
            .response()
            .map_err(|_| "screen sharing was cancelled".to_string())?;
        let stream = first(streams.streams())?;
        let (node, position, size) = (stream.pipe_wire_node_id(), stream.position(), stream.size());
        let fd = screencast.open_pipe_wire_remote(&session, Default::default()).await.map_err(|e| e.to_string())?;
        let restore_token = streams.restore_token().map(str::to_string);
        // The session lives as long as this process.
        std::mem::forget(session);
        return Ok(Screen { fd, node, restore_token, input: None, position, size });
    }

    let remote = RemoteDesktop::new().await.map_err(|e| format!("remote control isn't available: {e}"))?;
    let session = remote.create_session(Default::default()).await.map_err(|e| e.to_string())?;
    // A remote desktop session remembers its devices and screen together.
    let devices = SelectDevicesOptions::default()
        .set_devices(Some(DeviceType::Keyboard | DeviceType::Pointer))
        .set_persist_mode(PersistMode::ExplicitlyRevoked)
        .set_restore_token(token.as_deref());
    remote.select_devices(&session, devices).await.map_err(|e| e.to_string())?;
    screencast.select_sources(&session, sources).await.map_err(|e| e.to_string())?;
    let selected = remote
        .start(&session, None, Default::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|_| "remote control was cancelled".to_string())?;
    let stream = first(selected.streams())?;
    let node = stream.pipe_wire_node_id();
    let pixels = stream.size().map(|(w, h)| (w as f64, h as f64));
    let size = crate::outputs::Size::new(std::sync::Mutex::new(pixels));
    // KDE's second display, scaled: positions go in its size on the desktop, not its pixels
    if let Some(pixels) = pixels.filter(|_| virtual_monitor) {
        crate::outputs::follow_kde_display(stream.mapping_id().map(str::to_string), pixels, size.clone());
    }
    let fd =screencast.open_pipe_wire_remote(&session, Default::default()).await.map_err(|e| e.to_string())?;
    let allowed = input_allowed(selected.devices());
    let restore_token = if allowed { selected.restore_token().map(str::to_string) } else { None };
    let input = Some(Input::Desktop { proxy: remote, session, node, size, allowed, held: Default::default() });
    Ok(Screen { fd, node, restore_token, input, position: stream.position(), size: stream.size() })
}

impl Input {
    /// Release the keys still held down (their releases never came).
    pub async fn release_keys(&self) {
        match self {
            Self::X11(display) => return display.release_keys(),
            Self::Wlr(desktop) => return desktop.release_keys(),
            _ => {}
        }
        let Self::Desktop { proxy, session, held, .. } = self else { return };
        let keys: Vec<u32> = held.lock().unwrap().drain().collect();
        for keysym in keys {
            let _ = proxy.notify_keyboard_keysym(session, keysym as i32, KeyState::Released, Default::default()).await;
        }
    }

    /// End the desktop's session: keys left down are released, its virtual monitor goes
    /// away, and its memory with it.
    pub async fn close(self) {
        self.release_keys().await;
        if let Self::Desktop { session, .. } = self {
            let _ = session.close().await;
        }
    }
}

fn first(streams: &[Stream]) -> Result<&Stream, String> {
    streams.first().ok_or_else(|| "no screen was chosen".to_string())
}
