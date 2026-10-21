//! Media sessions (screen mirroring, remote control, second display): the service only
//! relays bytes. Whichever device shares runs `linksy-screen` for capture and encoding;
//! the other hands the stream to the app. Both meet the service on a private Unix socket
//! in the runtime dir.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use linksy_core::{Node, Session, SessionKind, Uuid};
use linksy_ipc::{ActiveSession, PendingSession};
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tracing::{info, warn};

/// How long the sharer has to pick a screen in the desktop's dialog.
const PICK_TIMEOUT: Duration = Duration::from_secs(120);
/// How long the app has to open the viewer.
const VIEW_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct Sessions {
    /// Every running session: its task, and what it is (for windows opened mid-session).
    running: Arc<Mutex<HashMap<Uuid, (AbortHandle, ActiveSession)>>>,
    /// Sessions to show that wait for the app to connect.
    pending: Arc<Mutex<HashMap<Uuid, PendingSession>>>,
    data_dir: PathBuf,
    /// This service's IPC socket, for an app it opens.
    socket: PathBuf,
    /// Share a test pattern instead of the screen, and never open the app (development,
    /// tests).
    test_pattern: bool,
    /// Cameras used here: where `camera_control` frames go, and the last `camera_state`
    /// each one sent (docs/protocol.md §4.8).
    cameras: Arc<Mutex<HashMap<Uuid, Camera>>>,
}

#[derive(Default)]
struct Camera {
    control: Option<mpsc::UnboundedSender<Vec<u8>>>,
    state: Option<serde_json::Value>,
}

impl Sessions {
    pub fn new(data_dir: PathBuf, socket: PathBuf) -> Self {
        let test_pattern = std::env::var_os("LINKSY_SCREEN_TEST").is_some();
        Self { running: Arc::default(), pending: Arc::default(), data_dir, socket, test_pattern, cameras: Arc::default() }
    }

    /// Keep each camera's last `camera_state`, for a window opened later.
    pub async fn watch_cameras(self, node: Node) {
        let mut events = node.subscribe();
        loop {
            match events.recv().await {
                // The phone may say before the webcam here is up: keep it either way.
                Ok(linksy_core::Event::CameraState { session, state, .. }) => {
                    self.cameras.lock().unwrap().entry(session).or_default().state = Some(state);
                }
                Ok(linksy_core::Event::SessionEnded { session }) => {
                    self.cameras.lock().unwrap().remove(&session);
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    }

    /// Change a camera used here: `control` holds `camera_control` keys (zoom, torch…).
    pub fn camera_control(&self, session: Uuid, control: serde_json::Value) -> Result<(), String> {
        let serde_json::Value::Object(mut keys) = control else { return Err("camera controls are an object".into()) };
        keys.insert("type".into(), "camera_control".into());
        let body = serde_json::to_vec(&keys).map_err(|e| e.to_string())?;
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend(body);
        let cameras = self.cameras.lock().unwrap();
        let tx = cameras.get(&session).and_then(|c| c.control.as_ref()).ok_or("that camera isn't on")?;
        tx.send(frame).map_err(|_| "that camera isn't on".to_string())
    }

    /// What a camera used here last said it can do and how it's set.
    pub fn camera_state(&self, session: Uuid) -> Option<serde_json::Value> {
        self.cameras.lock().unwrap().get(&session).and_then(|c| c.state.clone())
    }

    /// Serve the sessions this device accepted.
    pub async fn run(self, node: Node) {
        let Some(mut incoming) = node.incoming_sessions() else { return };
        while let Some(session) = incoming.recv().await {
            if session.sharing {
                self.spawn_share(session);
            } else if let Err(e) = self.show(session) {
                warn!("could not show a second display: {e}");
            }
        }
    }

    /// Run the sharing side of `session` until it ends.
    pub fn spawn_share(&self, session: Session) {
        self.spawn_helper(session, false);
    }

    /// Use the camera or microphone `session` receives as a device here until it ends.
    pub fn spawn_device(&self, session: Session) {
        self.spawn_helper(session, true);
    }

    /// `device`: this side receives a camera or microphone and publishes it to apps.
    fn spawn_helper(&self, session: Session, device: bool) {
        let id = session.id;
        let info = active(&session);
        let this = self.clone();
        let task = tokio::spawn(async move {
            if let Err(e) = this.run_helper(session, device).await {
                warn!("session ended: {e}");
            }
            this.running.lock().unwrap().remove(&id);
        });
        self.running.lock().unwrap().insert(id, (task.abort_handle(), info));
    }

    /// Start `linksy-screen` for this side of the session and relay its socket.
    async fn run_helper(&self, mut session: Session, device: bool) -> std::io::Result<()> {
        let role = match session.kind {
            _ if !device => "share",
            SessionKind::Microphone => "mic",
            _ => "webcam",
        };
        let path = socket_path(role, session.id);
        let listener = bind(&path)?;
        let mut cmd = tokio::process::Command::new(helper("linksy-screen"));
        cmd.arg(role).arg("--socket").arg(&path);
        match session.kind {
            SessionKind::Microphone if device => {
                cmd.arg("--name").arg(format!("Linksy Microphone ({})", session.peer.name));
            }
            _ if device => {
                cmd.arg("--name").arg(format!("Linksy Camera ({})", session.peer.name));
            }
            SessionKind::Screen => {}
            SessionKind::Camera => {
                cmd.arg("--camera");
            }
            SessionKind::Microphone => {
                cmd.arg("--mic");
            }
            SessionKind::Control => {
                cmd.arg("--control");
            }
            // A second display: a new virtual monitor, driven from the other device.
            SessionKind::Display => {
                cmd.arg("--control").arg("--virtual");
                // In the other screen's shape, when it said (a phone: about 20:9).
                if let Some((w, h)) = session.size.map(monitor_size) {
                    cmd.arg("--size").arg(format!("{w}x{h}"));
                }
            }
            // A phone as touchpad and keyboard: input only, nothing shared.
            SessionKind::Input => {
                cmd.arg("--input-only");
            }
        }
        // Each kind remembers its own answer to the desktop's dialog.
        let kind = format!("{:?}", session.kind).to_lowercase();
        if !device {
            cmd.arg("--token-file").arg(self.data_dir.join(format!("{kind}-token")));
        }
        if self.test_pattern && !device {
            cmd.arg("--test");
        }
        let mut child = cmd.kill_on_drop(true).spawn()?;
        info!("{} session with {} ({kind})", if device { "device" } else { "sharing" }, session.peer.name);
        let accepted = tokio::select! {
            a = tokio::time::timeout(PICK_TIMEOUT, listener.accept()) => a,
            // The helper gave up (dialog cancelled, no portal): nothing will connect.
            status = child.wait() => {
                let _ = std::fs::remove_file(&path);
                return Err(std::io::Error::other(format!("screen capture stopped ({})", status?)));
            }
        };
        let _ = std::fs::remove_file(&path);
        let (stream, _) = accepted.map_err(|_| std::io::Error::other("no screen was picked in time"))??;
        // A camera used here takes `camera_control` frames from the app instead of the
        // helper, which sends nothing back.
        let control = (device && session.kind == SessionKind::Camera).then(|| {
            let (tx, rx) = mpsc::unbounded_channel();
            self.cameras.lock().unwrap().entry(session.id).or_default().control = Some(tx);
            rx
        });
        relay(&mut session, stream, control).await;
        let _ = child.kill().await;
        Ok(())
    }

    /// Start viewing a session this device asked for: returns the socket the app reads.
    pub fn view(&self, session: Session) -> std::io::Result<PathBuf> {
        let path = socket_path("view", session.id);
        let listener = bind(&path)?;
        self.relay_when_joined(session, listener, path.clone());
        Ok(path)
    }

    /// Another device started showing its screen here (a second display): keep the stream
    /// for the app, opening the app if needed.
    fn show(&self, session: Session) -> std::io::Result<()> {
        let path = socket_path("view", session.id);
        let listener = bind(&path)?;
        self.pending.lock().unwrap().insert(
            session.id,
            PendingSession { session: session.id, peer: session.peer.clone(), kind: session.kind, socket: path.clone() },
        );
        info!("showing {}'s second display", session.peer.name);
        self.relay_when_joined(session, listener, path);
        // A running app opens the viewer on the session event; one that isn't running
        // finds the session when it starts.
        if !self.test_pattern {
            let _ = std::process::Command::new(helper("linksy-app"))
                .env("LINKSY_SOCKET", &self.socket)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        Ok(())
    }

    fn relay_when_joined(&self, mut session: Session, listener: UnixListener, path: PathBuf) {
        let id = session.id;
        let info = active(&session);
        let (running, pending) = (self.running.clone(), self.pending.clone());
        let task = tokio::spawn(async move {
            let accepted = tokio::time::timeout(VIEW_TIMEOUT, listener.accept()).await;
            let _ = std::fs::remove_file(&path);
            pending.lock().unwrap().remove(&id);
            if let Ok(Ok((stream, _))) = accepted {
                relay(&mut session, stream, None).await;
            }
            running.lock().unwrap().remove(&id);
        });
        self.running.lock().unwrap().insert(id, (task.abort_handle(), info));
    }

    pub fn pending(&self) -> Vec<PendingSession> {
        self.pending.lock().unwrap().values().cloned().collect()
    }

    pub fn active(&self) -> Vec<ActiveSession> {
        self.running.lock().unwrap().values().map(|(_, info)| info.clone()).collect()
    }

    /// Stop a session from either side. Returns false if it wasn't running.
    pub fn end(&self, id: Uuid) -> bool {
        self.pending.lock().unwrap().remove(&id);
        match self.running.lock().unwrap().remove(&id) {
            Some((task, _)) => {
                task.abort();
                true
            }
            None => false,
        }
    }
}

/// Copy media and control between the network and the local process until either side
/// stops. Dropping the session afterwards closes it on the other device.
/// Copy the session to the helper's socket and back; with `control`, what goes to the
/// other device comes from there instead of the helper.
async fn relay(session: &mut Session, local: UnixStream, control: Option<mpsc::UnboundedReceiver<Vec<u8>>>) {
    let (mut local_rx, mut local_tx) = local.into_split();
    let (send, recv) = (&mut session.send, &mut session.recv);
    let to_peer = async {
        match control {
            Some(mut frames) => {
                while let Some(frame) = frames.recv().await {
                    if send.write_all(&frame).await.is_err() {
                        break;
                    }
                }
            }
            None => {
                let _ = tokio::io::copy(&mut local_rx, send).await;
            }
        }
    };
    tokio::select! {
        _ = to_peer => {}
        _ = tokio::io::copy(recv, &mut local_tx) => {}
    }
    let _ = session.send.shutdown().await;
}

fn active(session: &Session) -> ActiveSession {
    ActiveSession { session: session.id, peer: session.peer.clone(), kind: session.kind, sharing: session.sharing }
}

fn socket_path(role: &str, id: Uuid) -> PathBuf {
    let dir = linksy_ipc::runtime_dir().unwrap_or_else(std::env::temp_dir);
    dir.join(format!("linksy-{role}-{}.sock", id.simple()))
}

fn bind(path: &Path) -> std::io::Result<UnixListener> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// A Linksy program next to this binary (installed or in target/), else from PATH.
pub(crate) fn helper(name: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name))).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(name))
}

/// A second display for a screen of `w`×`h`: the same shape, at most 1080 pixels high,
/// even sizes (what video encoders want).
fn monitor_size((w, h): (u32, u32)) -> (u32, u32) {
    // I hope nobody plugs in a 0-pixel-high phone. Just in case: max(1).
    let height = h.min(1080);
    let width = (u64::from(w) * u64::from(height) / u64::from(h.max(1))) as u32;
    (width.max(2) & !1, height.max(2) & !1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn monitor_takes_the_phone_shape() {
        assert_eq!(super::monitor_size((2400, 1080)), (2400, 1080));
        assert_eq!(super::monitor_size((2800, 1272)), (2376, 1080));
        assert_eq!(super::monitor_size((1280, 800)), (1280, 800));
    }
}
