//! Local IPC between `linksyd` and its front ends (GTK app, CLI).
//!
//! Newline-delimited JSON over a Unix socket. Clients send `{"id": n, "cmd": ...}`;
//! the daemon answers `{"id": n, "ok": ...}` or `{"id": n, "error": "..."}`, and pushes
//! `{"event": ...}` lines to every client as things happen, and `{"history": ...}` lines
//! when the activity history gains or updates an entry.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use linksy_core::{DeviceInfo, DeviceView, Direction, Event, Fingerprint, RemoteNotification, SessionKind, Settings, SmsMessage, SmsThread, Uuid};
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
    Clipboard { to: String, text: String },
    /// Put one file (≤ 10 MiB: an image, say) on a device's clipboard.
    ClipboardFile { to: String, path: PathBuf },
    /// Open a web link in the device's browser.
    OpenUrl { to: String, url: String },
    /// Make a device ring so it can be found (`on`), or stop it.
    Ring { to: String, on: bool },
    /// A phone notification shown here (for the reply window).
    PhoneNotification { from: String, key: String },
    /// Reply to a phone notification shown here.
    ReplyNotification { from: String, key: String, text: String },
    /// Whether phone notifications show here, and which apps are muted.
    NotificationPrefs,
    SetNotificationPrefs { prefs: NotificationPrefs },
    /// Texting through a phone: its conversations, a conversation's messages (newest
    /// first, older than `before_ms` to page back), and sending one.
    SmsThreads { phone: String },
    SmsMessages { phone: String, thread: String, before_ms: Option<u64> },
    SmsSend { phone: String, to: Vec<String>, text: String },
    /// Show a phone's files in the file manager (and its sidebar); answers the location.
    BrowseFiles { phone: String },
    /// Whether phones' photos are imported here, and where.
    PhotoPrefs,
    SetPhotoPrefs { prefs: PhotoPrefs },
    /// Which devices lock this computer when they leave, and after how long.
    LockPrefs,
    SetLockPrefs { prefs: LockPrefs },
    /// Allow everything from away for a device (screens, files, camera…), or only the
    /// small things (docs/protocol.md §4.13).
    SetAwayAll { device: String, on: bool },
    /// Accept files from a device without asking, or ask again.
    SetAutoAccept { device: String, on: bool },
    /// Let a device see and control the screen without asking, or ask again.
    SetAllowScreen { device: String, on: bool },
    /// Ask a phone where it is; answers its position or why not (docs/protocol.md §4.14).
    Locate { device: String },
    /// This version, the newest one if it's newer, and whether Linksy checks.
    Updates,
    SetUpdatePrefs { prefs: UpdatePrefs },
    /// Stop the service (the app starts the new one after an update).
    Quit,
    /// Terminals for phones (docs/protocol.md §4.15): what's set and running.
    TerminalStatus,
    /// Set the terminal code (6+ characters), or with none forget it (terminals off).
    SetTerminalCode { code: Option<String> },
    AllowTerminal { target: String, on: bool },
    UnlockTerminal { target: String },
    /// End a terminal. Its field is `terminal`, not `id`: the envelope has an `id` already.
    EndTerminal { terminal: String },
    /// Phones plugged in by USB cable, and how far each got.
    UsbStatus,
    /// Use this plugged-in phone with Linksy over the cable (and from now on).
    UsbConnect { phone: String },
    /// Switch this plugged-in phone on its own whenever it's plugged in, or stop.
    UsbAuto { phone: String, on: bool },
    Settings,
    UpdateSettings { settings: Settings },
    /// A pairing QR code (`linksy://pair?…`); valid five minutes or until cancelled.
    CreateInvite,
    CancelInvite,
    /// Activity, newest first; only with one device when `with` is given.
    History { with: Option<String> },
    /// Start a session with a device (see its screen, control it, or use it as a second
    /// display). Replies with a [`SessionInfo`] once its user allowed it.
    StartSession { with: String, kind: SessionKind },
    /// Sessions this device should show (a second display another device started),
    /// waiting for the app.
    PendingSessions,
    /// Sessions running now, so a window opened mid-session shows them.
    ActiveSessions,
    /// Stop a running session, from either side.
    EndSession { session: Uuid },
    /// Change a camera used here: `camera_control` keys (docs/protocol.md §4.8).
    CameraControl { session: Uuid, control: serde_json::Value },
    /// What a camera used here last said it can do and how it's set (`null` until it says).
    CameraState { session: Uuid },
    /// A device's picture, if it sent one: the image file's path.
    DevicePicture { device: String },
    ClearHistory { with: Option<String> },
    Forget { target: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub device: DeviceInfo,
    pub port: u16,
    pub download_dir: PathBuf,
    /// The service's version; empty from services older than 0.6.1.
    #[serde(default)]
    pub version: String,
}

/// A session the app can join: read the media from `socket` (a Unix socket; connect
/// within 60 seconds) and write control messages to it. No socket when this device is the
/// one sharing: the service runs that side itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session: Uuid,
    pub socket: Option<PathBuf>,
}

/// A session running now. `sharing`: this device is the one sending media.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveSession {
    pub session: Uuid,
    pub peer: DeviceInfo,
    pub kind: SessionKind,
    pub sharing: bool,
}

/// A session waiting for the app to show it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingSession {
    pub session: Uuid,
    pub peer: DeviceInfo,
    pub kind: SessionKind,
    pub socket: PathBuf,
}

/// One thing shared with a device: files, clipboard text or a ping, either way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The transfer id for files; random for the rest.
    pub id: Uuid,
    /// When it started, in milliseconds since the Unix epoch.
    pub at: u64,
    pub peer: Fingerprint,
    pub peer_name: String,
    pub direction: Direction,
    pub what: Shared,
    pub status: HistoryStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shared {
    /// `files` are relative paths as offered; `saved` where incoming ones landed.
    /// `import`: photos a phone imported into the picture library.
    Files {
        files: Vec<String>,
        total: u64,
        #[serde(default)]
        saved: Vec<PathBuf>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        import: bool,
    },
    /// The text is kept in memory only, never written to disk (it may be a password).
    Clipboard { #[serde(default)] text: String },
    Ping { title: String, body: String },
    /// A web link opened on the receiving device ("Open on PC").
    Link { url: String },
    /// This computer locked itself because the device left.
    Locked,
    /// A phone used as this computer's touchpad and keyboard; `seconds` once it ended.
    Remote {
        #[serde(default)]
        seconds: u64,
    },
    /// A camera used as a webcam, one way or the other; `seconds` once it ended.
    Camera {
        #[serde(default)]
        seconds: u64,
    },
    /// A microphone used here or shared; `seconds` once it ended.
    Microphone {
        #[serde(default)]
        seconds: u64,
    },
    /// A terminal a phone opened here; `seconds` once it ended (detached or closed).
    Terminal {
        #[serde(default)]
        seconds: u64,
    },
    /// A screen shared one way or the other; `seconds` once it ended.
    Screen {
        #[serde(default)]
        seconds: u64,
        /// A second display (Sidecar) rather than a mirrored screen.
        #[serde(default)]
        display: bool,
    },
}

/// Photo auto-import: photos a phone sends as imports land in `folder/<phone name>/`
/// without asking, while `import` is on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoPrefs {
    pub import: bool,
    pub folder: PathBuf,
}

/// "Lock when you walk away": this computer locks when one of `devices` hasn't checked in
/// for `grace_s` seconds. It never unlocks by itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LockPrefs {
    pub devices: Vec<Fingerprint>,
    pub grace_s: u32,
}

impl Default for LockPrefs {
    fn default() -> Self {
        Self { devices: Vec::new(), grace_s: 60 }
    }
}

/// Where a phone said it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: Option<f64>,
    /// When the phone measured it, in milliseconds since the epoch.
    pub at_ms: u64,
}

/// Terminals for phones: whether a code is set, which phones may open one, which are
/// locked out (too many wrong codes), and what runs now.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TerminalStatus {
    pub code_set: bool,
    pub allowed: Vec<Fingerprint>,
    pub locked: Vec<Fingerprint>,
    pub running: Vec<RunningTerminal>,
}

/// Phones plugged into this computer by USB cable (docs/protocol.md §4.11).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsbStatus {
    /// The service watches for phones (not when started with --no-usb, or without USB).
    pub watching: bool,
    pub phones: Vec<UsbPhone>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsbPhone {
    /// Its USB serial (or vendor:product): what `UsbConnect` takes.
    pub key: String,
    pub name: String,
    pub state: UsbState,
    /// Why it failed, when it did.
    pub error: Option<String>,
    /// Switched on its own whenever it's plugged in (desktop 0.8.7; off by default).
    #[serde(default)]
    pub auto: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsbState {
    /// Plugged in, not used with Linksy (yet).
    Plugged,
    /// Someone said "Not This Phone".
    Declined,
    /// Asked to restart as a Linksy accessory.
    Switching,
    /// In accessory mode: the cable carries Linksy once the phone app opens it.
    Accessory,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunningTerminal {
    pub id: String,
    pub device: Fingerprint,
    pub title: String,
    pub started_ms: u64,
    /// Someone's looking at it now (else it's detached, still running).
    pub attached: bool,
}

/// Whether Linksy checks linksy.diary.mg once a day for a newer version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatePrefs {
    pub check: bool,
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self { check: true }
    }
}

/// This version and, when there's a newer one, which and its release notes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: Option<String>,
    pub notes: Option<String>,
    pub prefs: UpdatePrefs,
}

/// How phone notifications show on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationPrefs {
    pub show: bool,
    pub muted: Vec<MutedApp>,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self { show: true, muted: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutedApp {
    /// The app's id on the phone (Android package name).
    pub app: String,
    pub app_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HistoryStatus {
    InProgress,
    Done,
    Declined,
    Cancelled,
    Failed { reason: String },
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
    History {
        history: HistoryEntry,
    },
}

/// Where Linksy's sockets live: `$XDG_RUNTIME_DIR`, or in a Flatpak, the app's own folder
/// in it, the one place every sandbox of the app shares.
pub fn runtime_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    Some(match std::env::var_os("FLATPAK_ID") {
        Some(id) => dir.join("app").join(id),
        None => dir,
    })
}

/// `$LINKSY_SOCKET`, else `linksyd.sock` in the [`runtime_dir`].
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("LINKSY_SOCKET") {
        return PathBuf::from(p);
    }
    match runtime_dir() {
        Some(dir) => dir.join("linksyd.sock"),
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
    history: broadcast::Sender<HistoryEntry>,
}

impl Client {
    pub async fn connect() -> io::Result<Self> {
        Self::connect_to(&socket_path()).await
    }

    pub async fn connect_to(path: &Path) -> io::Result<Self> {
        let (read, write) = UnixStream::connect(path).await?.into_split();
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(512);
        let (history, _) = broadcast::channel(128);
        let (p, ev, hist) = (pending.clone(), events.clone(), history.clone());
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
                    Ok(ServerMsg::History { history }) => {
                        let _ = hist.send(history);
                    }
                    Err(_) => {}
                }
            }
            // Daemon went away: fail everything still waiting. It's not you, it's the daemon.
            p.lock().unwrap().clear();
        });
        Ok(Self { writer: tokio::sync::Mutex::new(write), pending, next: AtomicU64::new(1), events, history })
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// History entries as they are added or change (a transfer finishing, say).
    pub fn history_updates(&self) -> broadcast::Receiver<HistoryEntry> {
        self.history.subscribe()
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

    pub async fn send_clipboard(&self, to: impl Into<String>, text: impl Into<String>) -> Result<(), ClientError> {
        self.call(Request::Clipboard { to: to.into(), text: text.into() }).await
    }

    pub async fn send_clipboard_file(&self, to: impl Into<String>, path: PathBuf) -> Result<Uuid, ClientError> {
        self.call(Request::ClipboardFile { to: to.into(), path }).await
    }

    pub async fn open_url(&self, to: impl Into<String>, url: impl Into<String>) -> Result<(), ClientError> {
        self.call(Request::OpenUrl { to: to.into(), url: url.into() }).await
    }

    pub async fn ring(&self, to: impl Into<String>, on: bool) -> Result<(), ClientError> {
        self.call(Request::Ring { to: to.into(), on }).await
    }

    pub async fn phone_notification(&self, from: impl Into<String>, key: impl Into<String>) -> Result<(DeviceInfo, RemoteNotification), ClientError> {
        self.call(Request::PhoneNotification { from: from.into(), key: key.into() }).await
    }

    pub async fn reply_notification(&self, from: impl Into<String>, key: impl Into<String>, text: impl Into<String>) -> Result<(), ClientError> {
        self.call(Request::ReplyNotification { from: from.into(), key: key.into(), text: text.into() }).await
    }

    pub async fn notification_prefs(&self) -> Result<NotificationPrefs, ClientError> {
        self.call(Request::NotificationPrefs).await
    }

    pub async fn set_notification_prefs(&self, prefs: NotificationPrefs) -> Result<(), ClientError> {
        self.call(Request::SetNotificationPrefs { prefs }).await
    }

    pub async fn sms_threads(&self, phone: impl Into<String>) -> Result<Vec<SmsThread>, ClientError> {
        self.call(Request::SmsThreads { phone: phone.into() }).await
    }

    /// Messages newest first, and whether older ones exist.
    pub async fn sms_messages(&self, phone: impl Into<String>, thread: impl Into<String>, before_ms: Option<u64>) -> Result<(Vec<SmsMessage>, bool), ClientError> {
        self.call(Request::SmsMessages { phone: phone.into(), thread: thread.into(), before_ms }).await
    }

    pub async fn sms_send(&self, phone: impl Into<String>, to: Vec<String>, text: impl Into<String>) -> Result<(), ClientError> {
        self.call(Request::SmsSend { phone: phone.into(), to, text: text.into() }).await
    }

    pub async fn browse_files(&self, phone: impl Into<String>) -> Result<String, ClientError> {
        self.call(Request::BrowseFiles { phone: phone.into() }).await
    }

    pub async fn photo_prefs(&self) -> Result<PhotoPrefs, ClientError> {
        self.call(Request::PhotoPrefs).await
    }

    pub async fn set_photo_prefs(&self, prefs: PhotoPrefs) -> Result<(), ClientError> {
        self.call(Request::SetPhotoPrefs { prefs }).await
    }

    pub async fn set_away_all(&self, device: impl Into<String>, on: bool) -> Result<(), ClientError> {
        self.call(Request::SetAwayAll { device: device.into(), on }).await
    }

    pub async fn set_auto_accept(&self, device: impl Into<String>, on: bool) -> Result<(), ClientError> {
        self.call(Request::SetAutoAccept { device: device.into(), on }).await
    }

    pub async fn set_allow_screen(&self, device: impl Into<String>, on: bool) -> Result<(), ClientError> {
        self.call(Request::SetAllowScreen { device: device.into(), on }).await
    }

    pub async fn locate(&self, device: impl Into<String>) -> Result<Location, ClientError> {
        self.call(Request::Locate { device: device.into() }).await
    }

    pub async fn updates(&self) -> Result<UpdateInfo, ClientError> {
        self.call(Request::Updates).await
    }

    pub async fn set_update_prefs(&self, prefs: UpdatePrefs) -> Result<(), ClientError> {
        self.call(Request::SetUpdatePrefs { prefs }).await
    }

    pub async fn quit(&self) -> Result<(), ClientError> {
        self.call(Request::Quit).await
    }

    pub async fn terminal_status(&self) -> Result<TerminalStatus, ClientError> {
        self.call(Request::TerminalStatus).await
    }

    pub async fn set_terminal_code(&self, code: Option<String>) -> Result<(), ClientError> {
        self.call(Request::SetTerminalCode { code }).await
    }

    pub async fn allow_terminal(&self, target: &str, on: bool) -> Result<(), ClientError> {
        self.call(Request::AllowTerminal { target: target.into(), on }).await
    }

    pub async fn unlock_terminal(&self, target: &str) -> Result<(), ClientError> {
        self.call(Request::UnlockTerminal { target: target.into() }).await
    }

    pub async fn end_terminal(&self, id: &str) -> Result<(), ClientError> {
        self.call(Request::EndTerminal { terminal: id.into() }).await
    }

    pub async fn usb_status(&self) -> Result<UsbStatus, ClientError> {
        self.call(Request::UsbStatus).await
    }

    pub async fn usb_connect(&self, phone: &str) -> Result<(), ClientError> {
        self.call(Request::UsbConnect { phone: phone.into() }).await
    }

    pub async fn usb_auto(&self, phone: &str, on: bool) -> Result<(), ClientError> {
        self.call(Request::UsbAuto { phone: phone.into(), on }).await
    }

    pub async fn lock_prefs(&self) -> Result<LockPrefs, ClientError> {
        self.call(Request::LockPrefs).await
    }

    pub async fn set_lock_prefs(&self, prefs: LockPrefs) -> Result<(), ClientError> {
        self.call(Request::SetLockPrefs { prefs }).await
    }

    pub async fn create_invite(&self) -> Result<String, ClientError> {
        self.call(Request::CreateInvite).await
    }

    pub async fn cancel_invite(&self) -> Result<(), ClientError> {
        self.call(Request::CancelInvite).await
    }

    pub async fn history(&self, with: Option<String>) -> Result<Vec<HistoryEntry>, ClientError> {
        self.call(Request::History { with }).await
    }

    pub async fn clear_history(&self, with: Option<String>) -> Result<(), ClientError> {
        self.call(Request::ClearHistory { with }).await
    }

    pub async fn start_session(&self, with: impl Into<String>, kind: SessionKind) -> Result<SessionInfo, ClientError> {
        self.call(Request::StartSession { with: with.into(), kind }).await
    }

    pub async fn pending_sessions(&self) -> Result<Vec<PendingSession>, ClientError> {
        self.call(Request::PendingSessions).await
    }

    pub async fn active_sessions(&self) -> Result<Vec<ActiveSession>, ClientError> {
        self.call(Request::ActiveSessions).await
    }

    pub async fn end_session(&self, session: Uuid) -> Result<bool, ClientError> {
        self.call(Request::EndSession { session }).await
    }

    pub async fn camera_control(&self, session: Uuid, control: serde_json::Value) -> Result<(), ClientError> {
        self.call(Request::CameraControl { session, control }).await
    }

    pub async fn camera_state(&self, session: Uuid) -> Result<Option<serde_json::Value>, ClientError> {
        self.call(Request::CameraState { session }).await
    }

    pub async fn device_picture(&self, device: impl Into<String>) -> Result<Option<PathBuf>, ClientError> {
        self.call(Request::DevicePicture { device: device.into() }).await
    }

    pub async fn settings(&self) -> Result<Settings, ClientError> {
        self.call(Request::Settings).await
    }

    pub async fn update_settings(&self, settings: Settings) -> Result<Settings, ClientError> {
        self.call(Request::UpdateSettings { settings }).await
    }

    pub async fn forget(&self, target: impl Into<String>) -> Result<bool, ClientError> {
        self.call(Request::Forget { target: target.into() }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_requests_survive_the_envelope() {
        // The envelope's own `id` is flattened beside the request's fields.
        let env = Envelope { id: 7, request: Request::EndTerminal { terminal: "t1".into() } };
        let back: Envelope = serde_json::from_str(&serde_json::to_string(&env).unwrap()).unwrap();
        assert_eq!(back.id, 7);
        assert!(matches!(back.request, Request::EndTerminal { terminal } if terminal == "t1"));
    }
}
