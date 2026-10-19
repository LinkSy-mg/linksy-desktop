//! Local IPC between `linksyd` and its front ends (GTK app, CLI).
//!
//! Newline-delimited JSON over a Unix socket. Clients send `{"id": n, "cmd": ...}`;
//! the daemon answers `{"id": n, "ok": ...}` or `{"id": n, "error": "..."}`, and pushes
//! `{"event": ...}` lines to every client as things happen.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use linksy_core::{DeviceInfo, DeviceView, Event, Uuid};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{broadcast, oneshot};

pub use linksy_core;

/// Devices are addressed by full id, short id ("A1B2-C3D4") or name; `pair` also takes
/// an `ip:port` address.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    Devices,
    Pair { target: String },
    Respond { request: Uuid, accept: bool },
    Send { to: String, paths: Vec<PathBuf> },
    Cancel { transfer: Uuid },
    Notify { to: String, title: String, body: String },
    Forget { target: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub device: DeviceInfo,
    pub port: u16,
    pub download_dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct Envelope {
    pub id: u64,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum ServerMsg {
    Reply {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Event {
        event: Event,
    },
}

/// `$LINKSY_SOCKET`, else `$XDG_RUNTIME_DIR/linksyd.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("LINKSY_SOCKET") {
        return PathBuf::from(p);
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => Path::new(&dir).join("linksyd.sock"),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            std::env::temp_dir().join(format!("linksyd-{user}.sock"))
        }
    }
}

#[derive(Debug)]
pub enum ClientError {
    /// The daemon is not running or the socket is gone.
    Disconnected,
    /// The daemon refused the request; the message is meant for the user.
    Daemon(String),
    Decode(serde_json::Error),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => f.write_str("Linksy service is not running"),
            Self::Daemon(m) => f.write_str(m),
            Self::Decode(e) => write!(f, "unexpected reply from Linksy service: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

pub struct Client {
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    pending: Pending,
    next: AtomicU64,
    events: broadcast::Sender<Event>,
}

impl Client {
    pub async fn connect() -> io::Result<Self> {
        Self::connect_to(&socket_path()).await
    }

    pub async fn connect_to(path: &Path) -> io::Result<Self> {
        let (read, write) = UnixStream::connect(path).await?.into_split();
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(512);
        let (p, ev) = (pending.clone(), events.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match serde_json::from_str::<ServerMsg>(&line) {
                    Ok(ServerMsg::Reply { id, ok, error }) => {
                        if let Some(tx) = p.lock().unwrap().remove(&id) {
                            let _ = tx.send(match error {
                                Some(e) => Err(e),
                                None => Ok(ok.unwrap_or(Value::Null)),
                            });
                        }
                    }
                    Ok(ServerMsg::Event { event }) => {
                        let _ = ev.send(event);
                    }
                    Err(_) => {}
                }
            }
            // Daemon went away: fail everything still waiting.
            p.lock().unwrap().clear();
        });
        Ok(Self { writer: tokio::sync::Mutex::new(write), pending, next: AtomicU64::new(1), events })
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub async fn call<T: DeserializeOwned>(&self, request: Request) -> Result<T, ClientError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut line = serde_json::to_vec(&Envelope { id, request }).map_err(ClientError::Decode)?;
        line.push(b'\n');
        if self.writer.lock().await.write_all(&line).await.is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err(ClientError::Disconnected);
        }
        match rx.await {
            Ok(Ok(v)) => serde_json::from_value(v).map_err(ClientError::Decode),
            Ok(Err(e)) => Err(ClientError::Daemon(e)),
            Err(_) => Err(ClientError::Disconnected),
        }
    }

    pub async fn status(&self) -> Result<Status, ClientError> {
        self.call(Request::Status).await
    }

    pub async fn devices(&self) -> Result<Vec<DeviceView>, ClientError> {
        self.call(Request::Devices).await
    }

    pub async fn pair(&self, target: impl Into<String>) -> Result<DeviceInfo, ClientError> {
        self.call(Request::Pair { target: target.into() }).await
    }

    pub async fn respond(&self, request: Uuid, accept: bool) -> Result<bool, ClientError> {
        self.call(Request::Respond { request, accept }).await
    }

    pub async fn send(&self, to: impl Into<String>, paths: Vec<PathBuf>) -> Result<Uuid, ClientError> {
        self.call(Request::Send { to: to.into(), paths }).await
    }

    pub async fn cancel(&self, transfer: Uuid) -> Result<bool, ClientError> {
        self.call(Request::Cancel { transfer }).await
    }

    pub async fn notify(&self, to: impl Into<String>, title: impl Into<String>, body: impl Into<String>) -> Result<(), ClientError> {
        self.call(Request::Notify { to: to.into(), title: title.into(), body: body.into() }).await
    }

    pub async fn forget(&self, target: impl Into<String>) -> Result<bool, ClientError> {
        self.call(Request::Forget { target: target.into() }).await
    }
}
