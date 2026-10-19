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
        size: Option<(f64, f64)>,
    },
}

impl Input {
    pub async fn apply(&self, msg: &ControlMsg) {
        let Self::Desktop { proxy, session, node, size } = self else {
            eprintln!("input: {msg:?}");
            return;
        };
        let result = match *msg {
            ControlMsg::Pointer { x, y } => match size {
                Some((w, h)) => {
                    let (x, y) = (x.clamp(0.0, 1.0) * w, y.clamp(0.0, 1.0) * h);
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
            ControlMsg::Key { keysym, pressed } => proxy.notify_keyboard_keysym(session, keysym as i32, state(pressed), Default::default()).await,
        };
        if let Err(e) = result {
            eprintln!("linksy-screen: input not applied: {e}");
        }
    }
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
        let node = first(streams.streams())?.pipe_wire_node_id();
        let fd = screencast.open_pipe_wire_remote(&session, Default::default()).await.map_err(|e| e.to_string())?;
        let restore_token = streams.restore_token().map(str::to_string);
        // The session lives as long as this process.
        std::mem::forget(session);
        return Ok(Screen { fd, node, restore_token, input: None });
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
    let size = stream.size().map(|(w, h)| (w as f64, h as f64));
    let fd = screencast.open_pipe_wire_remote(&session, Default::default()).await.map_err(|e| e.to_string())?;
    let restore_token = selected.restore_token().map(str::to_string);
    Ok(Screen { fd, node, restore_token, input: Some(Input::Desktop { proxy: remote, session, node, size }) })
}

fn first(streams: &[Stream]) -> Result<&Stream, String> {
    streams.first().ok_or_else(|| "no screen was chosen".to_string())
}
