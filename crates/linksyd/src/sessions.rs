//! Media sessions (screen mirroring, remote control, second display): the service only
//! relays bytes. Whichever device shares runs `linksy-screen` for capture and encoding;
//! the other hands the stream to the app. Both meet the service on a private Unix socket
//! in the runtime dir.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use linksy_core::{Node, Session, SessionKind, Uuid};
use linksy_ipc::PendingSession;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::task::AbortHandle;
use tracing::{info, warn};

/// How long the sharer has to pick a screen in the desktop's dialog.
const PICK_TIMEOUT: Duration = Duration::from_secs(120);
/// How long the app has to open the viewer.
const VIEW_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct Sessions {
    running: Arc<Mutex<HashMap<Uuid, AbortHandle>>>,
    /// Sessions to show that wait for the app to connect.
    pending: Arc<Mutex<HashMap<Uuid, PendingSession>>>,
    data_dir: PathBuf,
    /// This service's IPC socket, for an app it opens.
    socket: PathBuf,
    /// Share a test pattern instead of the screen, and never open the app (development,
    /// tests).
    test_pattern: bool,
}

impl Sessions {
    pub fn new(data_dir: PathBuf, socket: PathBuf) -> Self {
        let test_pattern = std::env::var_os("LINKSY_SCREEN_TEST").is_some();
        Self { running: Arc::default(), pending: Arc::default(), data_dir, socket, test_pattern }
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
        let id = session.id;
        let this = self.clone();
        let task = tokio::spawn(async move {
            if let Err(e) = this.share(session).await {
                warn!("screen sharing ended: {e}");
            }
            this.running.lock().unwrap().remove(&id);
        });
        self.running.lock().unwrap().insert(id, task.abort_handle());
    }

    async fn share(&self, mut session: Session) -> std::io::Result<()> {
        let path = socket_path("share", session.id);
        let listener = bind(&path)?;
        let mut cmd = tokio::process::Command::new(helper("linksy-screen"));
        cmd.arg("share").arg("--socket").arg(&path);
        match session.kind {
            SessionKind::Screen => {}
            SessionKind::Control => {
                cmd.arg("--control");
            }
            // A second display: a new virtual monitor, driven from the other device.
            SessionKind::Display => {
                cmd.arg("--control").arg("--virtual");
            }
        }
        // Each kind remembers its own answer to the desktop's dialog.
        let kind = format!("{:?}", session.kind).to_lowercase();
        cmd.arg("--token-file").arg(self.data_dir.join(format!("{kind}-token")));
        if self.test_pattern {
            cmd.arg("--test");
        }
        let mut child = cmd.kill_on_drop(true).spawn()?;
        info!("sharing the screen with {} ({kind})", session.peer.name);
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
        self.running.lock().unwrap().insert(id, task.abort_handle());
    }

    pub fn pending(&self) -> Vec<PendingSession> {
        self.pending.lock().unwrap().values().cloned().collect()
    }

    /// Stop a session from either side. Returns false if it wasn't running.
    pub fn end(&self, id: Uuid) -> bool {
        self.pending.lock().unwrap().remove(&id);
        match self.running.lock().unwrap().remove(&id) {
            Some(task) => {
                task.abort();
                true
            }
            None => false,
        }
    }
}

/// Copy media and control between the network and the local process until either side
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

fn socket_path(role: &str, id: Uuid) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
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
fn helper(name: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name))).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(name))
}
