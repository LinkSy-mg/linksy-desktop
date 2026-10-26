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
    /// Restarts the webcam here in another mode, and the mode it runs in.
    rate: Option<mpsc::UnboundedSender<Mode>>,
    mode: Mode,
}

/// How the phone's camera streams: frames per second and picture size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mode {
    fps: u32,
    width: u32,
    height: u32,
}

impl Default for Mode {
    fn default() -> Self {
        Mode { fps: 30, width: 1280, height: 720 }
    }
}

impl Camera {
    /// Follow what the phone says (or the app picked), from `camera_state` or
    /// `camera_control` keys: `fps` 30 or 60, `size` [w, h] or `width`/`height`, 720p or
    /// 1080p. A change restarts the webcam here.
    fn follow(&mut self, keys: &serde_json::Map<String, serde_json::Value>) {
        let num = |k: &str| keys.get(k).and_then(serde_json::Value::as_u64).map(|v| v as u32);
        let mut next = self.mode;
        if let Some(fps) = num("fps").filter(|f| matches!(f, 30 | 60)) {
            next.fps = fps;
        }
        let size = keys
            .get("size")
            .and_then(|s| Some((s.get(0)?.as_u64()? as u32, s.get(1)?.as_u64()? as u32)))
            .or_else(|| Some((num("width")?, num("height")?)));
        if let Some((w, h)) = size.filter(|s| matches!(s, (1280, 720) | (1920, 1080))) {
            (next.width, next.height) = (w, h);
        }
        if next != self.mode {
            self.mode = next;
            if let Some(tx) = &self.rate {
                let _ = tx.send(next);
            }
        }
    }
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
                    let mut cameras = self.cameras.lock().unwrap();
                    let camera = cameras.entry(session).or_default();
                    if let Some(keys) = state.as_object() {
                        camera.follow(keys);
                    }
                    camera.state = Some(state);
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
        let mut cameras = self.cameras.lock().unwrap();
        let camera = cameras.get_mut(&session).ok_or("that camera isn't on")?;
        camera.control.as_ref().ok_or("that camera isn't on")?.send(frame).map_err(|_| "that camera isn't on".to_string())?;
        // A new frame rate or size: the webcam here restarts in it (the phone sends a
        // keyframe).
        camera.follow(&keys);
        Ok(())
    }

    /// What a camera used here last said it can do and how it's set.
    pub fn camera_state(&self, session: Uuid) -> Option<serde_json::Value> {
        self.cameras.lock().unwrap().get(&session).and_then(|c| c.state.clone())
    }

    /// Serve the sessions this device accepted.
    pub async fn run(self, node: Node) {
        let Some(mut incoming) = node.incoming_sessions() else { return };
        while let Some(session) = incoming.recv().await {
            if session.kind == SessionKind::Terminal {
                #[cfg(target_os = "linux")]
                if let Some(t) = crate::terminal::TERMINALS.get() {
                    t.attach(session);
                }
            } else if session.sharing {
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
    async fn run_helper(&self, session: Session, device: bool) -> std::io::Result<()> {
        if device && session.kind == SessionKind::Camera {
            return self.run_webcam(session).await;
        }
        // Said before anything starts, rather than failing in the middle
        if !device && session.kind == SessionKind::Display && !virtual_monitors().await {
            #[cfg(target_os = "linux")]
            crate::notify::show(linksy_i18n::t!("No Second Display Here Yet").into(), no_virtual_monitors());
            return Err(std::io::Error::other(no_virtual_monitors()));
        }
        // Hyprland and Sway: a headless output of their own, shared like a screen
        let headless = match (device, session.kind, Compositor::here()) {
            (false, SessionKind::Display, Some(c)) => Some(c.create_output(session.size.map(monitor_size).unwrap_or((1920, 1080))).await?),
            _ => None,
        };
        // The portal asks which screen to share: say which one is the new display
        #[cfg(target_os = "linux")]
        if let Some(output) = &headless {
            crate::notify::show(
                linksy_i18n::t!("Choose {output} to Share", output = output.name),
                linksy_i18n::t!("{output} is the second display for {device}. Choose it in the next window; Linksy remembers the choice.", output = output.name, device = session.peer.name),
            );
        }
        let result = self.run_helper_on(session, device, headless.is_some()).await;
        if let Some(output) = headless {
            output.remove().await;
        }
        result
    }

    async fn run_helper_on(&self, mut session: Session, device: bool, headless: bool) -> std::io::Result<()> {
        let role = match session.kind {
            _ if !device => "share",
            SessionKind::Microphone => "mic",
            _ => "webcam",
        };
        let path = socket_path(role, session.id);
        let listener = bind(&path)?;
        let mut cmd = helper_command("linksy-screen");
        cmd.arg(role).arg("--socket").arg(&path);
        match session.kind {
            SessionKind::Microphone if device => {
                cmd.arg("--name").arg(format!("Linksy Microphone ({})", session.peer.name));
            }
            _ if device => {
                cmd.arg("--name").arg(format!("Linksy Camera ({})", session.peer.name));
            }
            SessionKind::Screen => {}
            // The headless output, picked in the portal's dialog (the first time) like a
            // screen. Input from the phone needs the RemoteDesktop portal, which these
            // compositors lack: the computer's own mouse and keyboard work on it.
            SessionKind::Display if headless => {}
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
            // Run by the terminal module, never here.
            SessionKind::Terminal => return Err(std::io::Error::other("not a media session")),
        }
        // Each kind remembers its own answer to the desktop's dialog.
        let kind = format!("{:?}", session.kind).to_lowercase();
        if !device {
            let token = self.data_dir.join(format!("{kind}-token"));
            // Asked to pick the screen again: the dialog shows, and the new pick is kept
            if session.choose {
                let _ = std::fs::remove_file(&token);
            }
            cmd.arg("--token-file").arg(token);
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
        relay(&mut session, stream).await;
        let _ = child.kill().await;
        Ok(())
    }

    /// A camera used here as a webcam: `linksy-screen webcam` at the frame rate and size
    /// the phone sends, restarted when they change (from the app's controls or the
    /// phone's), within the same session. The phone answers a change with a keyframe, so
    /// the picture comes back at once; apps see the camera blink.
    async fn run_webcam(&self, mut session: Session) -> std::io::Result<()> {
        let (control_tx, mut control) = mpsc::unbounded_channel();
        let (rate_tx, mut rate) = mpsc::unbounded_channel();
        let mut mode = {
            let mut cameras = self.cameras.lock().unwrap();
            let camera = cameras.entry(session.id).or_default();
            camera.control = Some(control_tx);
            camera.rate = Some(rate_tx);
            camera.mode
        };
        info!("device session with {} (camera)", session.peer.name);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let path = socket_path("webcam", session.id);
            let listener = bind(&path)?;
            let mut child = helper_command("linksy-screen")
                .arg("webcam")
                .arg("--socket")
                .arg(&path)
                .arg("--name")
                .arg(format!("Linksy Camera ({})", session.peer.name))
                .arg("--fps")
                .arg(mode.fps.to_string())
                .kill_on_drop(true)
                .spawn()?;
            let accepted = tokio::select! {
                a = tokio::time::timeout(VIEW_TIMEOUT, listener.accept()) => a,
                status = child.wait() => {
                    let _ = std::fs::remove_file(&path);
                    return Err(std::io::Error::other(format!("the webcam stopped ({})", status?)));
                }
            };
            let _ = std::fs::remove_file(&path);
            let (mut local, _) = accepted.map_err(|_| std::io::Error::other("the webcam didn't start"))??;
            // Video to the helper, camera_control frames to the phone, until the session
            // ends (return) or the frame rate changes (start again).
            let next = loop {
                tokio::select! {
                    n = tokio::io::AsyncReadExt::read(&mut session.recv, &mut buf) => match n {
                        Ok(0) | Err(_) => {
                            let _ = child.kill().await;
                            let _ = session.send.shutdown().await;
                            return Ok(());
                        }
                        Ok(n) => {
                            if local.write_all(&buf[..n]).await.is_err() {
                                let _ = session.send.shutdown().await;
                                return Ok(());
                            }
                        }
                    },
                    Some(frame) = control.recv() => {
                        if session.send.write_all(&frame).await.is_err() {
                            let _ = child.kill().await;
                            return Ok(());
                        }
                    }
                    Some(new) = rate.recv() => break new,
                }
            };
            info!("webcam from {} now {}x{} at {} fps", session.peer.name, next.width, next.height, next.fps);
            let _ = child.kill().await;
            mode = next;
        }
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
            open_window(&self.socket);
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
                relay(&mut session, stream).await;
            }
            running.lock().unwrap().remove(&id);
        });
        self.running.lock().unwrap().insert(id, (task.abort_handle(), info));
    }

    /// Forget the screen picked in the desktop's dialog for seeing and for controlling
    /// this one: the dialog shows the next time a device asks. Whether one was kept.
    pub fn forget_screen(&self) -> bool {
        ["screen", "control"].into_iter().filter(|kind| std::fs::remove_file(self.data_dir.join(format!("{kind}-token"))).is_ok()).count() > 0
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

/// Copy media and control between the session and the helper's socket until either side
/// stops. Dropping the session afterwards closes it on the other device.
async fn relay(session: &mut Session, local: UnixStream) {
    let (mut local_rx, mut local_tx) = local.into_split();
    let (send, recv) = (&mut session.send, &mut session.recv);
    tokio::select! {
        _ = tokio::io::copy(&mut local_rx, send) => {}
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
/// Linksy's window: `linksy-open` picks the GTK or the Qt app, whichever this desktop uses
/// and is installed; without it (run from the source tree), the GTK app.
fn window_app() -> PathBuf {
    let beside = helper("linksy-open");
    let in_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("linksy-open").is_file()));
    if beside.is_absolute() || in_path { beside } else { helper("linksy-app") }
}

/// Whether this desktop's screen sharing can make a virtual monitor (a second display):
/// GNOME's and KDE's can, Hyprland's and the wlroots one can't yet. The portal says so in
/// its AvailableSourceTypes (4 = virtual). Unsure (no portal answer): yes, and the helper
/// finds out.
pub(crate) async fn virtual_monitors() -> bool {
    if std::env::var_os("LINKSY_SCREEN_TEST").is_some() {
        return true;
    }
    // Their portals can't (#42), but Linksy makes a headless output there itself
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if ["hyprland", "sway"].iter().any(|d| desktop.contains(d)) {
        return Compositor::here().is_some();
    }
    let Ok(bus) = zbus::Connection::session().await else { return true };
    let reply = bus
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.portal.ScreenCast", "AvailableSourceTypes"),
        )
        .await;
    let Ok(reply) = reply else { return true };
    match reply.body().deserialize::<zbus::zvariant::OwnedValue>().ok().and_then(|v| u32::try_from(v).ok()) {
        Some(types) => types & 4 != 0,
        None => true,
    }
}

/// Why there's no second display here, for the app and the notification.
pub(crate) fn no_virtual_monitors() -> String {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok().and_then(|d| d.split(':').next().map(str::to_string)).filter(|d| !d.is_empty());
    match desktop {
        Some(desktop) => linksy_i18n::t!("Second Display isn't available on {desktop} yet: its screen sharing can't make a virtual monitor. GNOME and KDE Plasma can.", desktop),
        None => linksy_i18n::t!("Second Display isn't available on this desktop yet: its screen sharing can't make a virtual monitor. GNOME and KDE Plasma can.").into(),
    }
}

/// A compositor that makes headless outputs on request (a second display there).
#[derive(Clone, Copy)]
enum Compositor {
    Hyprland,
    Sway,
}

/// A headless output made for a second display, removed when it ends.
struct Output {
    compositor: Compositor,
    name: String,
}

impl Compositor {
    fn here() -> Option<Self> {
        let has = |tool: &str| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()));
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() && has("hyprctl") {
            Some(Compositor::Hyprland)
        } else if std::env::var_os("SWAYSOCK").is_some() && has("swaymsg") {
            Some(Compositor::Sway)
        } else {
            None
        }
    }

    async fn run(self, args: &[&str]) -> std::io::Result<String> {
        let tool = match self {
            Compositor::Hyprland => "hyprctl",
            Compositor::Sway => "swaymsg",
        };
        let out = tokio::process::Command::new(tool).args(args).output().await?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        // hyprctl exits 0 either way, and says why not ("invalid…", "no such…", "error…")
        let refused = matches!(self, Compositor::Hyprland)
            && !args.contains(&"-j")
            && ["invalid", "no such", "error", "couldn't", "failed"].iter().any(|w| text.to_lowercase().contains(w));
        if !out.status.success() || refused {
            return Err(std::io::Error::other(format!("{tool} {}: {}", args.join(" "), text.trim())));
        }
        Ok(text)
    }

    /// The outputs' names now.
    async fn outputs(self) -> std::io::Result<Vec<String>> {
        let json = match self {
            Compositor::Hyprland => self.run(&["-j", "monitors", "all"]).await?,
            Compositor::Sway => self.run(&["-r", "-t", "get_outputs"]).await?,
        };
        let list: Vec<serde_json::Value> = serde_json::from_str(&json).map_err(std::io::Error::other)?;
        Ok(list.iter().filter_map(|o| o["name"].as_str().map(String::from)).collect())
    }

    /// A new headless output of `w`×`h`, to the right of the others.
    async fn create_output(self, (w, h): (u32, u32)) -> std::io::Result<Output> {
        let before = self.outputs().await?;
        match self {
            Compositor::Hyprland => self.run(&["output", "create", "headless"]).await?,
            Compositor::Sway => self.run(&["create_output"]).await?,
        };
        let name = self.outputs().await?.into_iter().find(|n| !before.contains(n)).ok_or_else(|| std::io::Error::other("no new output appeared"))?;
        let output = Output { compositor: self, name };
        let mode = match self {
            Compositor::Hyprland => self.run(&["keyword", "monitor", &format!("{},{w}x{h}@60,auto,1", output.name)]).await,
            Compositor::Sway => self.run(&["output", &output.name, "mode", &format!("{w}x{h}@60Hz")]).await,
        };
        if let Err(e) = mode {
            output.remove().await;
            return Err(e);
        }
        info!("second display: made {} ({w}x{h})", output.name);
        Ok(output)
    }
}

impl Output {
    async fn remove(self) {
        let done = match self.compositor {
            Compositor::Hyprland => self.compositor.run(&["output", "remove", &self.name]).await,
            Compositor::Sway => self.compositor.run(&["output", &self.name, "unplug"]).await,
        };
        if let Err(e) = done {
            warn!("could not remove the second display's output {}: {e}", self.name);
        }
    }
}

/// Whether programs can be started in a systemd scope of their own (not in the Flatpak,
/// nor without systemd).
fn scopes() -> bool {
    static SCOPES: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SCOPES.get_or_init(|| {
        std::env::var_os("FLATPAK_ID").is_none()
            && std::process::Command::new("systemd-run")
                .args(["--user", "--scope", "--quiet", "--collect", "--expand-environment=no", "--", "true"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    })
}

/// A helper's command line, in a memory budget of its own: a systemd scope capped at 512 MB
/// (a phone-shaped second display needs 150-250 MB), so it never runs into the service's
/// own small cap. The helper runs in place (same process), and the scope goes away with
/// it. Without a working `systemd-run` (the Flatpak, no systemd), started as is.
fn helper_command(name: &str) -> tokio::process::Command {
    if !scopes() {
        return tokio::process::Command::new(helper(name));
    }
    let mut cmd = tokio::process::Command::new("systemd-run");
    cmd.args(["--user", "--scope", "--quiet", "--collect", "--expand-environment=no", "-p", "MemoryMax=512M", "--"]).arg(helper(name));
    cmd
}

/// Open Linksy's window. In a scope of its own, like the helpers: started from the service,
/// it would otherwise share the service's small memory cap.
pub(crate) fn open_window(socket: &Path) {
    let mut cmd = if scopes() {
        let mut cmd = tokio::process::Command::new("systemd-run");
        cmd.args(["--user", "--scope", "--quiet", "--collect", "--expand-environment=no", "--"]).arg(window_app());
        cmd
    } else {
        tokio::process::Command::new(window_app())
    };
    // Not waited for: tokio collects it once it exits
    let _ = cmd.env("LINKSY_SOCKET", socket).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

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

/// Needs a running Sway or Hyprland (SWAYSOCK or HYPRLAND_INSTANCE_SIGNATURE), e.g. a
/// headless Sway: `WLR_BACKENDS=headless sway`.
#[cfg(test)]
mod compositor_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "needs Sway or Hyprland"]
    async fn makes_and_removes_a_headless_output() {
        let c = Compositor::here().expect("no Sway or Hyprland here");
        let before = c.outputs().await.unwrap();
        let output = c.create_output((2400, 1080)).await.unwrap();
        let name = output.name.clone();
        assert!(c.outputs().await.unwrap().contains(&name));
        output.remove().await;
        assert_eq!(c.outputs().await.unwrap(), before);
    }
}
