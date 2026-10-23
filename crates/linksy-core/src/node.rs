//! A running Linksy device: listens for peers, knows which paired devices are nearby, and
//! reports everything that happens as [`Event`]s.
//!
//! Connections are made on demand: sending something dials the device, and a connection
//! with nothing going on closes after [`Config::idle_after`], so idle devices exchange no
//! traffic at all. Presence comes from discovery alone. [`Config::stay_connected`] keeps
//! links up instead, for devices on mains power that want the lowest latency.
//!
//! Anything that needs the user's consent (pairing, incoming files) is surfaced as a
//! request event with an id, and waits until [`Node::respond`] is called or it times out.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::{AbortHandle, JoinSet};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::discovery::{Discovery, DiscoveryEvent};
use crate::error::{Error, Result, invalid, protocol};
use crate::identity::{DeviceInfo, DeviceKind, Fingerprint, Identity};
use crate::fs::FsCall;
use crate::invite::{self, Invite};
use crate::settings::Settings;
use crate::protocol::{Battery, CallState, FsRequest, SmsMessage, SmsThread, MediaCommand, MediaState, RemoteNotification, 
    FileEntry, Handshake, Intent, Message, PROTOCOL_VERSION, RejectReason, SessionKind, SessionMsg, TransferMsg, read_msg,
    read_value, write_msg,
};
use crate::transfer::{self, Progress};
use crate::trust::{TrustStore, TrustedDevice};
use crate::tls;

const PAIR_TIMEOUT: Duration = Duration::from_secs(60);
const OFFER_TIMEOUT: Duration = Duration::from_secs(120);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Clipboard text limit; well under the protocol frame cap even after JSON escaping.
pub const MAX_CLIPBOARD: usize = 256 * 1024;
/// Most conversations or messages in one texting answer.
const MAX_SMS: u32 = 100;
/// Largest file one device may put on another's clipboard.
const MAX_CLIPBOARD_FILE: u64 = 10 << 20;
/// Longest link accepted for "Open on PC".
const MAX_URL: usize = 8 * 1024;
/// Mirrored notifications: longest key, and largest icon (base64 of a 64 KiB PNG).
const MAX_KEY: usize = 1024;
const MAX_ICON: usize = 64 * 1024 * 4 / 3 + 4;
/// A device picture: 128 KiB, in base64.
const MAX_PICTURE: usize = 128 * 1024 * 4 / 3 + 4;
// Fifteen seconds. Fourteen felt rushed.
const RECONNECT_EVERY: Duration = Duration::from_secs(15);
/// Per-address dial budget. LAN handshakes take milliseconds; a dead address never answers.
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a pairing QR code stays valid.
const INVITE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct Config {
    /// Holds the device identity and the list of paired devices.
    pub data_dir: PathBuf,
    /// Where received files are saved.
    pub download_dir: PathBuf,
    pub name: String,
    pub kind: DeviceKind,
    /// UDP port; 0 picks a free one.
    pub port: u16,
    pub bind: IpAddr,
    /// Advertise and browse with mDNS. Off in tests.
    pub discovery: bool,
    /// Keep paired devices connected all the time instead of connecting on demand.
    pub stay_connected: bool,
    /// Close a connection after this long with nothing going on (unless `stay_connected`).
    pub idle_after: Duration,
    /// The system other devices see ("linux", "android"…); the one built for by default.
    pub os: String,
    /// What this device can do for others, told in the handshake (see [`DeviceInfo::features`]).
    pub features: Vec<String>,
    /// Reach paired devices away from home (docs/protocol.md §4.13), through `relay` when the
    /// networks don't allow a direct path. Off in tests and by default; the setting
    /// `away` can still turn it off for the user.
    pub away: bool,
    pub relay: Option<String>,
}

impl Config {
    pub fn new(data_dir: PathBuf, download_dir: PathBuf, name: impl Into<String>) -> Self {
        Self {
            data_dir,
            download_dir,
            name: name.into(),
            kind: DeviceKind::Desktop,
            port: 0,
            bind: IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            discovery: true,
            stay_connected: false,
            idle_after: Duration::from_secs(60),
            os: std::env::consts::OS.to_string(),
            features: Vec::new(),
            away: false,
            relay: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    DeviceFound { device: DeviceInfo, addrs: Vec<SocketAddr> },
    DeviceLost { id: Fingerprint },
    Connected { device: DeviceInfo },
    Disconnected { id: Fingerprint },
    /// Show `code` and ask the user to confirm it matches the other screen.
    PairingRequest { request: Uuid, device: DeviceInfo, code: String, incoming: bool },
    Paired { device: DeviceInfo },
    PairingFailed { device: DeviceInfo, reason: String },
    /// Ask the user whether to accept these files.
    TransferRequest { transfer: Uuid, from: DeviceInfo, files: Vec<FileEntry>, total: u64 },
    /// `import`: photos a phone imports into this computer's pictures (no prompt).
    TransferStarted { transfer: Uuid, peer: DeviceInfo, direction: Direction, files: Vec<FileEntry>, total: u64, import: bool },
    TransferProgress { transfer: Uuid, bytes: u64, total: u64 },
    TransferComplete { transfer: Uuid, saved: Vec<PathBuf> },
    TransferFailed { transfer: Uuid, reason: String },
    SettingsChanged { settings: Settings },
    /// Ask the user whether `from` may start a media session (see their screen…).
    SessionRequest { request: Uuid, from: DeviceInfo, kind: SessionKind },
    /// `sharing`: this device is the one sharing (its screen, camera…).
    SessionStarted { session: Uuid, peer: DeviceInfo, kind: SessionKind, sharing: bool },
    SessionEnded { session: Uuid },
    Notification { from: DeviceInfo, title: String, body: String },
    /// A paired device changed its name.
    DeviceUpdated { device: DeviceInfo },
    /// A paired device shared its clipboard; front ends decide where it goes.
    Clipboard { from: DeviceInfo, text: String },
    /// A paired device asked to open a web link here (always `http` or `https`).
    OpenUrl { from: DeviceInfo, url: String },
    /// A paired device reported its battery (also in [`DeviceView::battery`]).
    Battery { from: DeviceInfo, battery: Battery },
    /// A paired device asks this one to ring so it can be found (`on`), or to stop.
    Ring { from: DeviceInfo, on: bool },
    /// A paired device (a phone) shows a notification, new or changed (same `key`).
    NotificationPosted { from: DeviceInfo, notification: RemoteNotification },
    /// …and it's gone there.
    NotificationRemoved { from: DeviceInfo, key: String },
    /// Requests about this device's own notifications, from the device mirroring them.
    NotificationReply { from: DeviceInfo, key: String, text: String },
    NotificationAction { from: DeviceInfo, key: String, action: String },
    NotificationDismissed { from: DeviceInfo, key: String },
    /// A device's media players, newest snapshot (older ones are dropped here).
    MediaState { from: DeviceInfo, state: MediaState },
    /// A device asks to control one of this device's players.
    MediaCommand { from: DeviceInfo, player: String, command: MediaCommand },
    /// A device wants this one's `media_state` while connected (`on`), or no longer.
    MediaWatch { from: DeviceInfo, on: bool },
    /// A phone's call: ringing, answered, or over.
    CallState { from: DeviceInfo, state: CallState, contact: Option<String> },
    /// A device checked in ("still here").
    Heartbeat { from: DeviceInfo },
    /// A device wants this one to check in every `every_s` seconds (`on`), or no longer.
    HeartbeatWatch { from: DeviceInfo, on: bool, every_s: u32 },
    /// A camera this device uses (session `session`) says what it can do and how it's set.
    CameraState { from: DeviceInfo, session: Uuid, state: serde_json::Value },
    /// A device sent its picture (base64 JPEG or PNG) to show for it.
    Picture { from: DeviceInfo, image: String },
    /// A device asks where this one is (answer with `send_location` or `refuse_location`).
    LocateRequest { from: DeviceInfo },
    /// Where a device said it is.
    Location { from: DeviceInfo, lat: f64, lon: f64, accuracy_m: Option<f64>, at_ms: u64 },
    /// A device wouldn't or couldn't say where it is.
    LocationRefused { from: DeviceInfo, reason: String },
    /// A device asks which terminals run here for it (answer with `send_terminals`).
    TerminalListRequest { from: DeviceInfo },
    /// The terminals running on a device for this one.
    Terminals { from: DeviceInfo, list: Vec<crate::protocol::TerminalInfo> },
    /// A device wants this one's clipboard as it changes (`on`), or no longer.
    ClipboardWatch { from: DeviceInfo, on: bool },
    /// A paired device put a file on this one's clipboard (an image…): it's at `path`.
    ClipboardFile { from: DeviceInfo, path: PathBuf },
    /// Texting: what a computer asks of this phone…
    SmsListThreads { from: DeviceInfo, limit: u32 },
    SmsListMessages { from: DeviceInfo, thread: String, before_ms: Option<u64>, limit: u32 },
    SmsSend { from: DeviceInfo, client_id: String, to: Vec<String>, text: String },
    /// …and what a phone tells this computer.
    SmsThreads { from: DeviceInfo, threads: Vec<SmsThread> },
    SmsMessages { from: DeviceInfo, thread: String, messages: Vec<SmsMessage>, more: bool },
    SmsMessage { from: DeviceInfo, thread: String, message: SmsMessage },
    SmsSent { from: DeviceInfo, client_id: String, ok: bool, error: Option<String> },
}

/// A running media session: the stream pair, kept open (and the connection with it) until
/// dropped. Reports `SessionEnded` when dropped.
pub struct Session {
    pub id: Uuid,
    pub peer: DeviceInfo,
    pub kind: SessionKind,
    /// This device shares; the peer watches.
    pub sharing: bool,
    /// The size the other side's screen has, when it said (a second display's shape).
    pub size: Option<(u32, u32)>,
    pub send: quinn::SendStream,
    pub recv: quinn::RecvStream,
    _busy: Option<Busy>,
    events: broadcast::Sender<Event>,
}

/// A device wants a terminal here (docs/protocol.md §4.15). Whoever runs terminals (the daemon)
/// checks the code and its own rules and gives a [`Verdict`]; nothing is asked of the
/// user before that, so a wrong code never bothers them.
pub struct TerminalCall {
    pub request: Uuid,
    pub from: DeviceInfo,
    pub open: crate::protocol::TerminalOpen,
    verdict: tokio::sync::oneshot::Sender<Verdict>,
}

impl TerminalCall {
    pub fn answer(self, verdict: Verdict) {
        let _ = self.verdict.send(verdict);
    }
}

impl std::fmt::Debug for TerminalCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the code.
        f.debug_struct("TerminalCall").field("request", &self.request).field("from", &self.from.name).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Open it now, as terminal `terminal` (new, or the one to reattach).
    Accept { terminal: String },
    /// Right code, but the user says yes first (`Event::SessionRequest`, kind `terminal`).
    Confirm { terminal: String },
    /// `off`, `code` (with the tries left), `locked`, `unsupported`.
    Decline { reason: String, tries_left: Option<u32> },
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.events.send(Event::SessionEnded { session: self.id });
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("id", &self.id).field("peer", &self.peer.name).field("kind", &self.kind).field("sharing", &self.sharing).finish()
    }
}

/// A device as the UI sees it: paired, nearby, or both.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceView {
    pub device: DeviceInfo,
    pub trusted: bool,
    pub connected: bool,
    pub nearby: bool,
    /// Last battery level the device reported since this one started, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<Battery>,
    /// Connected through a USB cable (docs/protocol.md §4.11) rather than the network.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub usb: bool,
    /// Connected away from home, over the internet (docs/protocol.md §4.13).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub away: bool,
    /// Everything is allowed from away for it (screens, files, camera…), not only the
    /// small things.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub away_all: bool,
    /// Until when it may connect from away (seconds since the epoch): a week after it was
    /// last on the same network or cable. None for devices not paired, while away from
    /// home is off, or before the two have met on the same network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub away_until: Option<u64>,
}

/// How a connection reaches the other device, by the address it comes from: bridges
/// (docs/protocol.md §4.11, §4.13) deliver their packets from this computer itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    /// Over the internet (an away bridge on 127.0.0.2): the last resort.
    Away,
    /// The same network.
    Lan,
    /// A USB cable (a bridge on another loopback address): the best.
    Usb,
}

// TODO: change this function name. link_of is doing its best.
pub fn link_of(addr: SocketAddr) -> Link {
    match addr.ip().to_canonical() {
        std::net::IpAddr::V4(ip) if ip == crate::away::AWAY_IP => Link::Away,
        ip if ip.is_loopback() => Link::Usb,
        _ => Link::Lan,
    }
}

/// A connection that arrives through a USB bridge (docs/protocol.md §4.11).
pub fn over_usb(addr: SocketAddr) -> bool {
    link_of(addr) == Link::Usb
}

struct Peer {
    conn: quinn::Connection,
    device: DeviceInfo,
    /// Who opened this connection; used to break ties when both sides dial at once.
    dialer: Fingerprint,
    activity: Arc<Activity>,
    /// The other side keeps links open; don't close this one for being idle.
    peer_stays: bool,
}

/// What a connection is doing, so idle ones can be closed.
struct Activity {
    /// Transfers and prompts in progress.
    busy: AtomicUsize,
    last: Mutex<Instant>,
}

impl Activity {
    fn new() -> Arc<Self> {
        Arc::new(Self { busy: AtomicUsize::new(0), last: Mutex::new(Instant::now()) })
    }

    fn touch(&self) {
        *self.last.lock().unwrap() = Instant::now();
    }

    fn idle_for(&self) -> Option<Duration> {
        (self.busy.load(Ordering::SeqCst) == 0).then(|| self.last.lock().unwrap().elapsed())
    }

    fn hold(self: &Arc<Self>) -> Busy {
        self.busy.fetch_add(1, Ordering::SeqCst);
        Busy(self.clone())
    }
}

/// Keeps a connection open while alive.
struct Busy(Arc<Activity>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.touch();
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Inner {
    id: Fingerprint,
    info: Mutex<DeviceInfo>,
    config: Config,
    settings: Mutex<Settings>,
    endpoint: quinn::Endpoint,
    trust: Mutex<TrustStore>,
    peers: Mutex<HashMap<Fingerprint, Peer>>,
    nearby: Mutex<HashMap<Fingerprint, (DeviceInfo, Vec<SocketAddr>)>>,
    pending: Mutex<HashMap<Uuid, oneshot::Sender<bool>>>,
    transfers: Mutex<HashMap<Uuid, AbortHandle>>,
    events: broadcast::Sender<Event>,
    discovery: Mutex<Option<Discovery>>,
    /// One dial at a time per device, so concurrent sends share a connection.
    dialing: Mutex<HashMap<Fingerprint, Arc<tokio::sync::Mutex<()>>>>,
    /// Accepted incoming sessions, for whoever runs the media side (the daemon).
    sessions: Mutex<Option<mpsc::UnboundedReceiver<Session>>>,
    sessions_tx: mpsc::UnboundedSender<Session>,
    /// File requests from other devices, for whoever serves this device's files (phones).
    fs: Mutex<Option<mpsc::UnboundedReceiver<FsCall>>>,
    fs_tx: mpsc::UnboundedSender<FsCall>,
    /// Terminal requests, for whoever runs terminals (the daemon on a computer).
    terminals: Mutex<Option<mpsc::UnboundedReceiver<TerminalCall>>>,
    terminals_tx: mpsc::UnboundedSender<TerminalCall>,
    /// Session requests answered "this device can't do that", not "no".
    unsupported: Mutex<std::collections::HashSet<Uuid>>,
    /// Screen sizes to give with an accept (a second display shown here).
    session_sizes: Mutex<HashMap<Uuid, (u32, u32)>>,
    /// The token of the pairing QR code on screen, and when it expires.
    invite: Mutex<Option<(String, Instant)>>,
    /// Where imported photos go (a folder per device inside it); `None` refuses imports.
    import_dir: Mutex<Option<PathBuf>>,
    /// Each paired device's last reported battery.
    batteries: Mutex<HashMap<Fingerprint, Battery>>,
    /// The newest media snapshot `seq` seen from each device.
    media_seq: Mutex<HashMap<Fingerprint, u64>>,
    /// Away from home, when on (docs/protocol.md §4.13).
    away: Mutex<Option<Arc<crate::away::Away>>>,
}

#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

impl Node {
    pub async fn start(config: Config) -> Result<Self> {
        std::fs::create_dir_all(&config.data_dir)?;
        let identity = Identity::load_or_create(&config.data_dir)?;
        let trust = TrustStore::load(config.data_dir.join("trusted.json"))?;
        let settings = Settings::load(&config.data_dir.join("settings.json"), &config)?;
        let info = DeviceInfo {
            id: identity.fingerprint,
            name: settings.name.clone(),
            kind: config.kind,
            os: config.os.clone(),
            features: config.features.clone(),
            away: None,
        };

        let endpoint = bind_endpoint(&identity, SocketAddr::new(config.bind, config.port))?;
        let (events, _) = broadcast::channel(1024);
        let (sessions_tx, sessions) = mpsc::unbounded_channel();
        let (fs_tx, fs) = mpsc::unbounded_channel();
        let (terminals_tx, terminals) = mpsc::unbounded_channel();
        let node = Self {
            inner: Arc::new(Inner {
                id: info.id,
                info: Mutex::new(info),
                config,
                settings: Mutex::new(settings),
                endpoint,
                trust: Mutex::new(trust),
                peers: Mutex::new(HashMap::new()),
                nearby: Mutex::new(HashMap::new()),
                pending: Mutex::new(HashMap::new()),
                transfers: Mutex::new(HashMap::new()),
                events,
                discovery: Mutex::new(None),
                dialing: Mutex::new(HashMap::new()),
                invite: Mutex::new(None),
                import_dir: Mutex::default(),
                batteries: Mutex::default(),
                media_seq: Mutex::default(),
                sessions: Mutex::new(Some(sessions)),
                sessions_tx,
                fs: Mutex::new(Some(fs)),
                fs_tx,
                terminals: Mutex::new(Some(terminals)),
                terminals_tx,
                unsupported: Mutex::new(Default::default()),
                session_sizes: Mutex::default(),
                away: Mutex::new(None),
            }),
        };

        tokio::spawn(node.clone().accept_loop());
        if node.inner.config.discovery {
            let (tx, rx) = mpsc::unbounded_channel();
            let d = Discovery::start(&node.info(), node.local_addr()?.port(), tx)?;
            *node.inner.discovery.lock().unwrap() = Some(d);
            tokio::spawn(node.clone().discovery_loop(rx));
        }
        tokio::spawn(node.clone().reconnect_loop());
        tokio::spawn(node.clone().idle_loop());
        if node.inner.config.away && node.settings().away {
            node.start_away().await;
        }
        info!(id = %node.inner.id.short(), addr = %node.local_addr()?, "linksy node started");
        Ok(node)
    }

    pub fn info(&self) -> DeviceInfo {
        self.inner.info.lock().unwrap().clone()
    }

    pub fn settings(&self) -> Settings {
        self.inner.settings.lock().unwrap().clone()
    }

    /// Apply and save new settings. A new name is announced to the network right away.
    pub async fn update_settings(&self, new: Settings) -> Result<Settings> {
        let new = tokio::task::spawn_blocking(move || new.validated()).await.map_err(|e| protocol(e.to_string()))??;
        let old = self.settings();
        if new == old {
            return Ok(new);
        }
        new.save(&self.inner.config.data_dir.join("settings.json"), &self.inner.config)?;
        *self.inner.settings.lock().unwrap() = new.clone();
        if new.name != old.name {
            let info = {
                let mut info = self.inner.info.lock().unwrap();
                info.name = new.name.clone();
                info.clone()
            };
            if let Some(d) = self.inner.discovery.lock().unwrap().as_mut()
                && let Err(e) = d.update(&info)
            {
                warn!("could not announce the new name: {e}");
            }
            // Connected devices hear it directly; others at their next handshake.
            let live: Vec<quinn::Connection> = self.inner.peers.lock().unwrap().values().map(|p| p.conn.clone()).collect();
            for conn in live {
                let msg = Message::Renamed { name: info.name.clone() };
                tokio::spawn(async move {
                    if let Ok(mut s) = conn.open_uni().await {
                        let _ = write_msg(&mut s, &msg).await;
                        let _ = s.finish();
                    }
                });
            }
        }
        if new.stay_connected && !old.stay_connected {
            self.reconnect_all();
        }
        // Another relay: the away endpoint starts again on it (its away connections drop;
        // peers learn the new relay at their next handshake).
        if new.relay != old.relay && self.inner.config.away {
            self.stop_away().await;
            if new.away {
                self.start_away().await;
            }
        }
        // Turned on after starting without it: start it now. (Off is checked at each use.)
        if new.away && !old.away && self.inner.config.away && self.inner.away.lock().unwrap().is_none() {
            self.start_away().await;
        }
        self.emit(Event::SettingsChanged { settings: new.clone() });
        Ok(new)
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.inner.endpoint.local_addr()?)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    pub fn trusted(&self) -> Vec<TrustedDevice> {
        self.inner.trust.lock().unwrap().list()
    }

    /// Paired and nearby devices, merged.
    pub fn devices(&self) -> Vec<DeviceView> {
        let away_on = self.inner.away.lock().unwrap().is_some() && self.settings().away;
        let trust = self.inner.trust.lock().unwrap();
        let peers = self.inner.peers.lock().unwrap();
        let nearby = self.inner.nearby.lock().unwrap();
        let batteries = self.inner.batteries.lock().unwrap();
        let mut out: Vec<DeviceView> = trust
            .list()
            .into_iter()
            .map(|t| DeviceView {
                battery: batteries.get(&t.device.id).copied(),
                connected: peers.contains_key(&t.device.id),
                usb: peers.get(&t.device.id).is_some_and(|p| link_of(p.conn.remote_address()) == Link::Usb),
                away: peers.get(&t.device.id).is_some_and(|p| link_of(p.conn.remote_address()) == Link::Away),
                away_all: t.away_all,
                // Only once each knows how to reach the other away.
                away_until: (away_on && t.device.away.is_some()).then(|| t.lease_until()),
                nearby: nearby.contains_key(&t.device.id),
                // Newest name we know: the live connection, then the network announcement
                // (which doesn't carry features: those come from the last handshake).
                device: {
                    let mut d = peers
                        .get(&t.device.id)
                        .map(|p| p.device.clone())
                        .or_else(|| nearby.get(&t.device.id).map(|(d, _)| d.clone()))
                        .unwrap_or_else(|| t.device.clone());
                    if d.features.is_empty() {
                        d.features = t.device.features;
                    }
                    d
                },
                trusted: true,
            })
            .collect();
        for (id, (device, _)) in nearby.iter() {
            if !trust.contains(id) {
                out.push(DeviceView { device: device.clone(), trusted: false, connected: false, nearby: true, battery: None, usb: false, away: false, away_all: false, away_until: None });
            }
        }
        // Paired and reachable first, then devices to pair, then paired ones that are away.
        let rank = |d: &DeviceView| match (d.trusted, d.connected || d.nearby) {
            (true, true) => 0,
            (false, _) => 1,
            (true, false) => 2,
        };
        out.sort_by(|a, b| (rank(a), &a.device.name).cmp(&(rank(b), &b.device.name)));
        out
    }

    /// Turn down a session request because this device can't do that kind of session (a
    /// phone asked to be controlled, say). The requester learns it's unsupported rather
    /// than declined. Returns false if the request already expired.
    pub fn decline_unsupported(&self, request: Uuid) -> bool {
        self.inner.unsupported.lock().unwrap().insert(request);
        self.respond(request, false)
    }

    /// Answer a pairing or transfer request. Returns false if it already expired.
    /// Accept a `display` session giving this device's screen size (landscape), so the
    /// second display comes in its shape. Otherwise like [`Node::respond`].
    pub fn respond_session(&self, request: Uuid, accept: bool, size: Option<(u32, u32)>) -> bool {
        if let (true, Some(size)) = (accept, size) {
            self.inner.session_sizes.lock().unwrap().insert(request, size);
        }
        self.respond(request, accept)
    }

    pub fn respond(&self, request: Uuid, accept: bool) -> bool {
        match self.inner.pending.lock().unwrap().remove(&request) {
            Some(tx) => tx.send(accept).is_ok(),
            None => false,
        }
    }

    /// Whether a pairing, transfer or session request still waits for an answer.
    pub fn is_pending(&self, request: Uuid) -> bool {
        self.inner.pending.lock().unwrap().contains_key(&request)
    }

    /// Reconnect to an already paired device at a known address.
    pub async fn connect(&self, addr: SocketAddr) -> Result<DeviceInfo> {
        self.dial(addr, Intent::Connect).await
    }

    /// Pair with the device at `addr`. Resolves once both users confirmed the code.
    pub async fn pair(&self, addr: SocketAddr) -> Result<DeviceInfo> {
        self.dial(addr, Intent::Pair).await
    }

    /// Pair with a device found by discovery.
    pub async fn pair_device(&self, id: Fingerprint) -> Result<DeviceInfo> {
        let addrs = self.nearby_addrs(&id);
        let mut last = Error::Unreachable;
        for addr in addrs {
            match self.pair(addr).await {
                Ok(d) => return Ok(d),
                Err(e @ Error::Declined) => return Err(e),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// A pairing invite for a QR code: whoever scans it and connects within five minutes
    /// pairs without comparing codes. One at a time; a new one replaces the old.
    pub fn create_invite(&self) -> Result<Invite> {
        let port = self.local_addr()?.port();
        // Container and VM bridges only reach this machine; they'd just bloat the QR code.
        const VIRTUAL: [&str; 10] = ["docker", "br-", "veth", "virbr", "lxc", "lxd", "podman", "cni", "vmnet", "vboxnet"];
        let mut addrs: Vec<(u8, SocketAddr)> = if_addrs::get_if_addrs()?
            .into_iter()
            .filter(|i| !i.is_loopback() && !VIRTUAL.iter().any(|v| i.name.starts_with(v)))
            .map(|i| i.ip())
            // Link-local IPv6 needs an interface scope the other device can't know.
            .filter(|ip| !matches!(ip, IpAddr::V6(v6) if v6.segments()[0] & 0xffc0 == 0xfe80))
            .map(|ip| {
                // Home and office LANs first, then other IPv4 (VPNs), then IPv6.
                let rank = match ip {
                    IpAddr::V4(v4) if v4.is_private() => 0,
                    IpAddr::V4(_) => 1,
                    IpAddr::V6(_) => 2,
                };
                (rank, SocketAddr::new(ip, port))
            })
            .collect();
        addrs.sort();
        addrs.dedup();
        let addrs: Vec<SocketAddr> = addrs.into_iter().map(|(_, a)| a).take(4).collect();
        if addrs.is_empty() {
            return Err(invalid("this device isn't connected to a network"));
        }
        let token = Uuid::new_v4().simple().to_string();
        *self.inner.invite.lock().unwrap() = Some((token.clone(), Instant::now() + INVITE_TTL));
        Ok(Invite { id: self.inner.id, name: self.info().name, addrs, token })
    }

    /// Stop accepting the current invite (the QR code was closed).
    pub fn cancel_invite(&self) {
        self.inner.invite.lock().unwrap().take();
    }

    /// Pair with the device whose invite (QR code) was scanned.
    pub async fn pair_with_invite(&self, uri: &str) -> Result<DeviceInfo> {
        let invite = Invite::parse(uri)?;
        if invite.id == self.inner.id {
            return Err(invalid("that's this device's own code"));
        }
        let conn = self.race_connect(&invite.addrs, &invite.id).await?;
        tokio::time::timeout(PAIR_TIMEOUT, self.handshake(conn, Intent::Pair, Some(invite.token))).await.map_err(|_| Error::Timeout)?
    }

    /// Use up the current invite if `token` is its token.
    fn redeem_invite(&self, token: &str) -> bool {
        let mut slot = self.inner.invite.lock().unwrap();
        match slot.as_ref() {
            Some((t, expires)) if Instant::now() < *expires && invite::same_token(t, token) => {
                slot.take();
                true
            }
            _ => false,
        }
    }

    pub fn forget(&self, id: &Fingerprint) -> Result<bool> {
        if let Some(p) = self.inner.peers.lock().unwrap().remove(id) {
            p.conn.close(0u32.into(), b"forgotten");
        }
        self.inner.trust.lock().unwrap().remove(id)
    }

    pub async fn notify(&self, to: &Fingerprint, title: &str, body: &str) -> Result<()> {
        self.send_message(to, &Message::Notify { title: title.into(), body: body.into() }).await
    }

    /// Share clipboard text with a connected device.
    pub async fn send_clipboard(&self, to: &Fingerprint, text: &str) -> Result<()> {
        if text.is_empty() {
            return Err(invalid("the clipboard is empty"));
        }
        if text.len() > MAX_CLIPBOARD {
            return Err(invalid("clipboard text is too large, send it as a file instead"));
        }
        self.send_message(to, &Message::Clipboard { text: text.into() }).await
    }

    /// Ask a connected device to open a web link in its browser.
    pub async fn open_url(&self, to: &Fingerprint, url: &str) -> Result<()> {
        let url = url.trim();
        if !is_web_link(url) {
            return Err(invalid("only http:// and https:// links can be opened"));
        }
        self.send_message(to, &Message::OpenUrl { url: url.into() }).await
    }

    /// Tell a connected device this one's battery level (phones: on connecting, and when it
    /// changes by a few percent or starts or stops charging).
    pub async fn send_battery(&self, to: &Fingerprint, battery: Battery) -> Result<()> {
        if battery.level > 100 {
            return Err(invalid("battery level is a percentage"));
        }
        self.send_message(to, &Message::Battery(battery)).await
    }

    /// Make a device ring so it can be found, or stop it ringing.
    pub async fn ring(&self, to: &Fingerprint, on: bool) -> Result<()> {
        self.send_message(to, if on { &Message::Ring } else { &Message::RingStop }).await
    }

    /// Mirror one of this device's notifications to a connected device (phones).
    pub async fn post_notification(&self, to: &Fingerprint, notification: RemoteNotification) -> Result<()> {
        if notification.key.is_empty() || notification.key.len() > MAX_KEY {
            return Err(invalid("notification key must be 1 to 1024 bytes"));
        }
        if notification.icon.as_ref().is_some_and(|i| i.len() > MAX_ICON) {
            return Err(invalid("notification icon is too large"));
        }
        self.send_message(to, &Message::NotificationPosted(notification)).await
    }

    /// The mirrored notification `key` is gone here.
    pub async fn remove_notification(&self, to: &Fingerprint, key: &str) -> Result<()> {
        self.send_message(to, &Message::NotificationRemoved { key: key.into() }).await
    }

    /// Reply to a notification `from` mirrored here.
    pub async fn reply_notification(&self, from: &Fingerprint, key: &str, text: &str) -> Result<()> {
        if text.trim().is_empty() {
            return Err(invalid("the reply is empty"));
        }
        if text.len() > MAX_CLIPBOARD {
            return Err(invalid("the reply is too long"));
        }
        self.send_message(from, &Message::NotificationReply { key: key.into(), text: text.into() }).await
    }

    /// Press a button of a notification `from` mirrored here.
    pub async fn notification_action(&self, from: &Fingerprint, key: &str, action: &str) -> Result<()> {
        self.send_message(from, &Message::NotificationAction { key: key.into(), action: action.into() }).await
    }

    /// Dismiss, at the source, a notification `from` mirrored here.
    pub async fn dismiss_notification(&self, from: &Fingerprint, key: &str) -> Result<()> {
        self.send_message(from, &Message::NotificationDismiss { key: key.into() }).await
    }

    /// Send this device's media players (the whole set; bump `seq` each time).
    pub async fn send_media_state(&self, to: &Fingerprint, state: &MediaState) -> Result<()> {
        if state.players.iter().any(|p| p.art.as_ref().is_some_and(|a| a.len() > MAX_ICON)) {
            return Err(invalid("cover art is too large"));
        }
        self.send_message(to, &Message::MediaState(state.clone())).await
    }

    /// Control a player on another device.
    pub async fn media_command(&self, to: &Fingerprint, player: &str, command: MediaCommand) -> Result<()> {
        self.send_message(to, &Message::MediaCommand { player: player.into(), command }).await
    }

    /// Ask a device for its media players while connected (`on`), or stop.
    pub async fn watch_media(&self, to: &Fingerprint, on: bool) -> Result<()> {
        self.send_message(to, &Message::MediaWatch { on }).await
    }

    /// Tell a device about a call on this one (phones), so it can pause its media.
    pub async fn send_call_state(&self, to: &Fingerprint, state: CallState, contact: Option<&str>) -> Result<()> {
        self.send_message(to, &Message::CallState { state, contact: contact.map(Into::into) }).await
    }

    /// Ask a phone where it is; the answer comes as `Event::Location` (or
    /// `Event::LocationRefused`).
    pub async fn locate(&self, to: &Fingerprint) -> Result<()> {
        self.send_message(to, &Message::Locate).await
    }

    /// Answer `Event::LocateRequest`: where this device is now.
    pub async fn send_location(&self, to: &Fingerprint, lat: f64, lon: f64, accuracy_m: Option<f64>, at_ms: u64) -> Result<()> {
        self.send_message(to, &Message::Location { lat, lon, accuracy_m, at_ms }).await
    }

    /// Answer `Event::LocateRequest` with no: turned off, no permission, no fix.
    pub async fn refuse_location(&self, to: &Fingerprint, reason: &str) -> Result<()> {
        self.send_message(to, &Message::LocationRefused { reason: reason.into() }).await
    }

    /// Ask a computer which terminals run there for this device (`Event::Terminals`).
    pub async fn list_terminals(&self, to: &Fingerprint) -> Result<()> {
        self.send_message(to, &Message::TerminalList).await
    }

    /// Answer `Event::TerminalListRequest`.
    pub async fn send_terminals(&self, to: &Fingerprint, list: Vec<crate::protocol::TerminalInfo>) -> Result<()> {
        self.send_message(to, &Message::Terminals { list }).await
    }

    /// Terminal requests from other devices, for the code that runs terminals. Until
    /// something takes it, this device answers that it can't.
    pub fn incoming_terminals(&self) -> Option<mpsc::UnboundedReceiver<TerminalCall>> {
        self.inner.terminals.lock().unwrap().take()
    }

    /// Open (or with `open.resume`, reattach) a terminal on `to`. Resolves with the
    /// session (its output on `recv`; `ControlMsg::Term*` frames on `send`) and the
    /// terminal's id.
    pub async fn request_terminal(&self, to: &Fingerprint, open: crate::protocol::TerminalOpen) -> Result<(Session, String)> {
        let (session, terminal) = self.open_session(to, SessionKind::Terminal, Some(open)).await?;
        Ok((session, terminal.unwrap_or_default()))
    }

    /// Tell the device using this one's camera (session `session`) what it can do and how
    /// it's set (a phone, after every change).
    pub async fn send_camera_state(&self, to: &Fingerprint, session: Uuid, state: serde_json::Value) -> Result<()> {
        self.send_message(to, &Message::CameraState { session, state }).await
    }

    /// Send this device's picture (a JPEG or PNG, base64, ≤ 128 KiB) for the other to show.
    pub async fn send_picture(&self, to: &Fingerprint, image: &str) -> Result<()> {
        if image.len() > MAX_PICTURE {
            return Err(invalid("the picture is too big (128 KB at most)"));
        }
        self.send_message(to, &Message::Picture { image: image.into() }).await
    }

    /// Ask a device for its clipboard whenever it changes (`on`), or stop.
    pub async fn watch_clipboard(&self, to: &Fingerprint, on: bool) -> Result<()> {
        self.send_message(to, &Message::ClipboardWatch { on }).await
    }

    /// Put one file (≤ 10 MiB: an image, say) on another device's clipboard.
    pub async fn send_clipboard_file(&self, to: &Fingerprint, path: PathBuf) -> Result<Uuid> {
        let meta = std::fs::metadata(&path)?;
        if !meta.is_file() {
            return Err(invalid("only a single file can go on the clipboard"));
        }
        if meta.len() > MAX_CLIPBOARD_FILE {
            return Err(invalid("too big for the clipboard (10 MB at most); send it as a file instead"));
        }
        self.offer_with(to, vec![path], false, true).await
    }

    /// Check in with a device that asked for heartbeats.
    pub async fn heartbeat(&self, to: &Fingerprint) -> Result<()> {
        self.send_message(to, &Message::Heartbeat).await
    }

    /// Ask a device to check in every `every_s` seconds (`on`), or to stop.
    pub async fn watch_heartbeat(&self, to: &Fingerprint, on: bool, every_s: u32) -> Result<()> {
        self.send_message(to, &Message::HeartbeatWatch { on, every_s: every_s.max(5) }).await
    }

    /// Texting from a computer: ask a phone for its conversations (answer: `SmsThreads`).
    pub async fn sms_list_threads(&self, phone: &Fingerprint, limit: u32) -> Result<()> {
        self.send_message(phone, &Message::SmsListThreads { limit: limit.clamp(1, MAX_SMS) }).await
    }

    /// Ask for a conversation's messages, newest first, older than `before_ms` if given
    /// (answer: `SmsMessages`).
    pub async fn sms_list_messages(&self, phone: &Fingerprint, thread: &str, before_ms: Option<u64>, limit: u32) -> Result<()> {
        self.send_message(phone, &Message::SmsListMessages { thread: thread.into(), before_ms, limit: limit.clamp(1, MAX_SMS) }).await
    }

    /// Have a phone send a text; its outcome comes back as `SmsSent` with `client_id`.
    pub async fn sms_send(&self, phone: &Fingerprint, client_id: &str, to: &[String], text: &str) -> Result<()> {
        if to.is_empty() || text.trim().is_empty() {
            return Err(invalid("a text needs someone to send it to and something to say"));
        }
        self.send_message(phone, &Message::SmsSend { client_id: client_id.into(), to: to.to_vec(), text: text.into() }).await
    }

    /// Phones: answer `SmsListThreads` (at most 100 threads).
    pub async fn sms_threads(&self, to: &Fingerprint, threads: Vec<SmsThread>) -> Result<()> {
        self.send_message(to, &Message::SmsThreads { threads: threads.into_iter().take(MAX_SMS as usize).collect() }).await
    }

    /// Phones: answer `SmsListMessages` (at most 100 messages; `more` if older exist).
    pub async fn sms_messages(&self, to: &Fingerprint, thread: &str, messages: Vec<SmsMessage>, more: bool) -> Result<()> {
        let messages = messages.into_iter().take(MAX_SMS as usize).collect();
        self.send_message(to, &Message::SmsMessages { thread: thread.into(), messages, more }).await
    }

    /// Phones: a message came in or went out.
    pub async fn sms_message(&self, to: &Fingerprint, thread: &str, message: SmsMessage) -> Result<()> {
        self.send_message(to, &Message::SmsMessage { thread: thread.into(), message }).await
    }

    /// Phones: how sending `client_id` went.
    pub async fn sms_sent(&self, to: &Fingerprint, client_id: &str, ok: bool, error: Option<&str>) -> Result<()> {
        self.send_message(to, &Message::SmsSent { client_id: client_id.into(), ok, error: error.map(Into::into) }).await
    }

    /// Report a device found by other means than the built-in mDNS (Android's NsdManager,
    /// Bluetooth, a QR code…). Same effect as discovery finding it.
    pub fn device_found(&self, device: DeviceInfo, addrs: Vec<SocketAddr>) {
        if device.id == self.inner.id || addrs.is_empty() {
            return;
        }
        let known = self.inner.nearby.lock().unwrap().insert(device.id, (device.clone(), addrs.clone()));
        if known.as_ref().map(|(d, a)| (d, a)) != Some((&device, &addrs)) {
            self.emit(Event::DeviceFound { device: device.clone(), addrs });
        }
        // Back on the same network: replace an away connection with a direct one.
        if self.link(&device.id) == Some(Link::Away) {
            let (node, addrs, id) = (self.clone(), self.nearby_addrs(&device.id), device.id);
            tokio::spawn(async move {
                if let Ok(conn) = node.race_connect(&addrs, &id).await {
                    let _ = tokio::time::timeout(HANDSHAKE_TIMEOUT, node.handshake(conn, Intent::Connect, None)).await;
                }
            });
            return;
        }
        if self.stays() && self.inner.trust.lock().unwrap().contains(&device.id) && !self.is_connected(&device.id) {
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(e) = node.ensure_connection(&device.id).await {
                    debug!("connect to {} failed: {e}", device.name);
                }
            });
        }
    }

    /// The counterpart of [`Node::device_found`]: the device is no longer around.
    pub fn device_lost(&self, id: &Fingerprint) {
        if self.inner.nearby.lock().unwrap().remove(id).is_some() {
            self.emit(Event::DeviceLost { id: *id });
        }
    }

    /// Ask `to` to start a media session: for [`SessionKind::Screen`], to share its screen
    /// with us; for [`SessionKind::Display`], to show ours as its second display. Resolves
    /// once its user allowed it (or fails if they declined).
    pub async fn request_session(&self, to: &Fingerprint, kind: SessionKind) -> Result<Session> {
        Ok(self.open_session(to, kind, None).await?.0)
    }

    async fn open_session(&self, to: &Fingerprint, kind: SessionKind, terminal: Option<crate::protocol::TerminalOpen>) -> Result<(Session, Option<String>)> {
        let (conn, activity) = self.ensure_connection(to).await?;
        let busy = activity.hold();
        let peer = self.peer_info(to).ok_or(Error::Unreachable)?;
        let id = Uuid::new_v4();
        let (mut send, mut recv) = conn.open_bi().await?;
        write_msg(&mut send, &SessionMsg::SessionRequest { id, kind, terminal }).await?;
        let (size, terminal) = match tokio::time::timeout(OFFER_TIMEOUT, read_msg(&mut recv)).await.map_err(|_| Error::Timeout)?? {
            SessionMsg::SessionAccept { width: Some(w), height: Some(h), terminal } if w > 0 && h > 0 => (Some((w, h)), terminal),
            SessionMsg::SessionAccept { terminal, .. } => (None, terminal),
            SessionMsg::SessionDecline { reason: Some(r), .. } if r == "unsupported" => return Err(Error::Unsupported),
            SessionMsg::SessionDecline { reason: Some(r), .. } if r == "away" => return Err(Error::NotFromAway),
            SessionMsg::SessionDecline { reason: Some(reason), tries_left } => return Err(Error::Refused { reason, tries_left }),
            SessionMsg::SessionDecline { .. } => return Err(Error::Declined),
            other => return Err(protocol(format!("unexpected {other:?}"))),
        };
        let sharing = kind.requester_shares();
        self.emit(Event::SessionStarted { session: id, peer: peer.clone(), kind, sharing });
        Ok((Session { id, peer, kind, sharing, size, send, recv, _busy: Some(busy), events: self.inner.events.clone() }, terminal))
    }

    /// Sessions this device accepted. Can be taken once; the taker runs the media (capture
    /// and encoding when `sharing`, showing it otherwise) and drops each when done.
    /// File requests from other devices, for the code that serves this device's files.
    /// Until something takes it, this device answers that it doesn't share files.
    pub fn incoming_fs(&self) -> Option<mpsc::UnboundedReceiver<FsCall>> {
        self.inner.fs.lock().unwrap().take()
    }

    /// A new bidirectional stream to a paired device, connecting if needed.
    pub(crate) async fn open_stream(&self, to: &Fingerprint) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        let (conn, activity) = self.ensure_connection(to).await?;
        activity.touch();
        Ok(conn.open_bi().await?)
    }

    pub fn incoming_sessions(&self) -> Option<mpsc::UnboundedReceiver<Session>> {
        self.inner.sessions.lock().unwrap().take()
    }

    /// Make sure a paired device is reachable, connecting if needed.
    pub async fn reach(&self, to: &Fingerprint) -> Result<DeviceInfo> {
        self.ensure_connection(to).await?;
        self.peer_info(to).ok_or(Error::Unreachable)
    }

    /// Offer files and folders to a paired device, connecting if needed. Progress and the
    /// outcome arrive as events carrying the returned transfer id.
    pub async fn send_files(&self, to: &Fingerprint, paths: Vec<PathBuf>) -> Result<Uuid> {
        self.offer(to, paths, false).await
    }

    /// Send photos and videos into another device's picture library (phones: photo
    /// auto-import). Saved without asking where the receiver takes imports, else declined.
    pub async fn import_files(&self, to: &Fingerprint, paths: Vec<PathBuf>) -> Result<Uuid> {
        self.offer(to, paths, true).await
    }

    /// Take photo imports into `dir` (a folder per device inside it), or refuse them (`None`).
    pub fn set_import_dir(&self, dir: Option<PathBuf>) {
        *self.inner.import_dir.lock().unwrap() = dir;
    }

    async fn offer(&self, to: &Fingerprint, paths: Vec<PathBuf>, import: bool) -> Result<Uuid> {
        self.offer_with(to, paths, import, false).await
    }

    async fn offer_with(&self, to: &Fingerprint, paths: Vec<PathBuf>, import: bool, clipboard: bool) -> Result<Uuid> {
        let (conn, activity) = self.ensure_connection(to).await?;
        let busy = activity.hold();
        let peer = self.peer_info(to).ok_or(Error::Unreachable)?;
        let files = tokio::task::spawn_blocking(move || transfer::collect(&paths))
            .await
            .map_err(|e| protocol(e.to_string()))??;
        if files.is_empty() {
            return Err(invalid("nothing to send"));
        }
        let id = Uuid::new_v4();
        let node = self.clone();
        let task = tokio::spawn(async move {
            let entries: Vec<FileEntry> = files.iter().map(|f| f.entry.clone()).collect();
            let total = entries.iter().map(|f| f.size).sum();
            let result = async {
                let (mut send, mut recv) = conn.open_bi().await?;
                write_msg(&mut send, &TransferMsg::Offer { id, files: entries.clone(), import, clipboard }).await?;
                node.emit(Event::TransferStarted { transfer: id, peer, direction: Direction::Outgoing, files: entries, total, import });
                let offsets = match read_msg(&mut recv).await? {
                    TransferMsg::Accept { offsets } => offsets,
                    TransferMsg::Decline => return Err(Error::Declined),
                    other => return Err(protocol(format!("unexpected {other:?}"))),
                };
                let events = node.inner.events.clone();
                let mut report = move |bytes| {
                    let _ = events.send(Event::TransferProgress { transfer: id, bytes, total });
                };
                let start = offsets.iter().sum();
                let mut progress = Progress::new(start, &mut report);
                transfer::send_files(&mut send, &mut recv, &files, &offsets, &mut progress).await
            }
            .await;
            node.inner.transfers.lock().unwrap().remove(&id);
            drop(busy);
            node.emit(match result {
                Ok(()) => Event::TransferComplete { transfer: id, saved: Vec::new() },
                Err(e) => Event::TransferFailed { transfer: id, reason: e.to_string() },
            });
        });
        self.inner.transfers.lock().unwrap().insert(id, task.abort_handle());
        Ok(id)
    }

    /// Stop a transfer in either direction. The other side sees it fail.
    pub fn cancel(&self, transfer: Uuid) -> bool {
        self.respond(transfer, false);
        match self.inner.transfers.lock().unwrap().remove(&transfer) {
            Some(task) => {
                task.abort();
                self.emit(Event::TransferFailed { transfer, reason: "cancelled".into() });
                true
            }
            None => false,
        }
    }

    pub async fn shutdown(&self) {
        if let Some(d) = self.inner.discovery.lock().unwrap().take() {
            d.stop();
        }
        self.inner.endpoint.close(0u32.into(), b"bye");
        // Let peers hear the goodbye, but never hang on one that doesn't answer.
        let _ = tokio::time::timeout(Duration::from_secs(2), self.inner.endpoint.wait_idle()).await;
        self.stop_away().await;
    }

    // ---- internals ---- (here be dragons, the friendly kind)

    fn emit(&self, e: Event) {
        let _ = self.inner.events.send(e);
    }

    async fn send_message(&self, to: &Fingerprint, msg: &Message) -> Result<()> {
        let (conn, activity) = self.ensure_connection(to).await?;
        let mut s = conn.open_uni().await?;
        write_msg(&mut s, msg).await?;
        s.finish()?;
        activity.touch();
        Ok(())
    }

    fn live(&self, id: &Fingerprint) -> Option<(quinn::Connection, Arc<Activity>)> {
        let peers = self.inner.peers.lock().unwrap();
        let p = peers.get(id).filter(|p| p.conn.close_reason().is_none())?;
        Some((p.conn.clone(), p.activity.clone()))
    }

    /// The connection to a paired device, dialing it at every address we know (in
    /// parallel) when there is none.
    async fn ensure_connection(&self, id: &Fingerprint) -> Result<(quinn::Connection, Arc<Activity>)> {
        let last_addr = self.inner.trust.lock().unwrap().get(id).ok_or(Error::NotPaired)?.last_addr;
        if let Some(live) = self.live(id) {
            return Ok(live);
        }
        let gate = self.inner.dialing.lock().unwrap().entry(*id).or_default().clone();
        let _gate = gate.lock().await;
        if let Some(live) = self.live(id) {
            return Ok(live);
        }
        let mut addrs = self.nearby_addrs(id);
        addrs.extend(last_addr.filter(|a| !addrs.contains(a) && link_of(*a) != Link::Away));
        match self.race_connect(&addrs, id).await {
            Ok(conn) => {
                tokio::time::timeout(HANDSHAKE_TIMEOUT, self.handshake(conn, Intent::Connect, None)).await.map_err(|_| Error::Timeout)??;
            }
            // Not on this network: away from home, when it's on and allowed.
            Err(e) => {
                if self.connect_away(id).await.is_err() {
                    return Err(e);
                }
            }
        }
        self.live(id).ok_or(Error::Unreachable)
    }

    /// Open a QUIC connection to `id` at whichever address answers first. Only the QUIC
    /// handshakes race: running ours on several connections at once would register
    /// duplicates that close each other.
    async fn race_connect(&self, addrs: &[SocketAddr], id: &Fingerprint) -> Result<quinn::Connection> {
        let mut dials = JoinSet::new();
        for &addr in addrs {
            let endpoint = self.inner.endpoint.clone();
            dials.spawn(async move {
                let connecting = endpoint.connect(addr, "linksy")?;
                tokio::time::timeout(DIAL_TIMEOUT, connecting).await.map_err(|_| Error::Timeout)?.map_err(Error::from)
            });
        }
        let mut winner = None;
        while let Some(r) = dials.join_next().await {
            match r {
                Ok(Ok(conn)) => match tls::peer_fingerprint(&conn) {
                    Ok(fp) if fp == *id => {
                        winner = Some(conn);
                        break;
                    }
                    _ => conn.close(0u32.into(), b"wrong device"),
                },
                Ok(Err(e)) => debug!("dial failed: {e}"),
                Err(_) => {}
            }
        }
        // Dropping the set aborts slower attempts; any that already connected close.
        drop(dials);
        winner.ok_or(Error::Unreachable)
    }

    fn peer_info(&self, id: &Fingerprint) -> Option<DeviceInfo> {
        self.inner.peers.lock().unwrap().get(id).map(|p| p.device.clone())
    }

    fn nearby_addrs(&self, id: &Fingerprint) -> Vec<SocketAddr> {
        self.inner.nearby.lock().unwrap().get(id).map(|(_, a)| a.clone()).unwrap_or_default()
    }

    fn is_connected(&self, id: &Fingerprint) -> bool {
        self.inner.peers.lock().unwrap().get(id).is_some_and(|p| p.conn.close_reason().is_none())
    }

    /// Register a consent question. Must happen before the request event is emitted, so
    /// an immediate answer is never lost.
    fn expect_answer(&self, request: Uuid) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(request, tx);
        rx
    }

    async fn await_answer(&self, request: Uuid, rx: oneshot::Receiver<bool>, timeout: Duration) -> bool {
        let answer = matches!(tokio::time::timeout(timeout, rx).await, Ok(Ok(true)));
        self.inner.pending.lock().unwrap().remove(&request);
        answer
    }

    async fn accept_loop(self) {
        while let Some(incoming) = self.inner.endpoint.accept().await {
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(e) = node.handle_incoming(incoming).await {
                    debug!("incoming connection ended: {e}");
                }
            });
        }
    }

    async fn handle_incoming(&self, incoming: quinn::Incoming) -> Result<()> {
        let conn = incoming.await?;
        let fp = tls::peer_fingerprint(&conn)?;
        let (mut send, mut recv) = tokio::time::timeout(HANDSHAKE_TIMEOUT, conn.accept_bi()).await.map_err(|_| Error::Timeout)??;
        let (device, intent, peer_stays, invite) = match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut recv)).await.map_err(|_| Error::Timeout)?? {
            Handshake::Hello { version, device, intent, stay_connected, invite } => {
                if version != PROTOCOL_VERSION {
                    write_msg(&mut send, &Handshake::Reject { reason: RejectReason::IncompatibleVersion }).await?;
                    send.finish()?;
                    return Err(protocol(format!("peer speaks protocol v{version}")));
                }
                (device, intent, stay_connected, invite)
            }
            other => return Err(protocol(format!("expected hello, got {other:?}"))),
        };
        if device.id != fp {
            conn.close(1u32.into(), b"identity mismatch");
            return Err(protocol("device id does not match its certificate"));
        }
        let trusted = self.inner.trust.lock().unwrap().contains(&fp);
        match intent {
            Intent::Connect if trusted => {
                write_msg(&mut send, &self.welcome()).await?;
                self.register(conn, device, fp, peer_stays);
            }
            Intent::Connect => {
                write_msg(&mut send, &Handshake::Reject { reason: RejectReason::NotPaired }).await?;
                send.finish()?;
                // Let the reject arrive before the connection goes away.
                let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            }
            Intent::Pair => {
                // Scanned our QR code: the scanner checked our certificate, the token proves
                // it's them. Unknown or stale tokens fall back to comparing codes.
                let preapproved = invite.as_deref().is_some_and(|t| self.redeem_invite(t));
                write_msg(&mut send, &self.welcome()).await?;
                self.run_pairing(&conn, &mut send, &mut recv, &device, true, preapproved).await?;
                self.register(conn, device, fp, peer_stays);
            }
        }
        Ok(())
    }

    async fn dial(&self, addr: SocketAddr, intent: Intent) -> Result<DeviceInfo> {
        let conn = self.inner.endpoint.connect(addr, "linksy")?.await?;
        self.handshake(conn, intent, None).await
    }

    /// Our side of the handshake on a connection we opened.
    async fn handshake(&self, conn: quinn::Connection, intent: Intent, invite: Option<String>) -> Result<DeviceInfo> {
        let fp = tls::peer_fingerprint(&conn)?;
        if fp == self.inner.id {
            conn.close(0u32.into(), b"self");
            return Err(invalid("that address is this device"));
        }
        if intent == Intent::Connect && !self.inner.trust.lock().unwrap().contains(&fp) {
            conn.close(0u32.into(), b"not paired");
            return Err(Error::NotPaired);
        }
        let (mut send, mut recv) = conn.open_bi().await?;
        let scanned = invite.is_some();
        let hello = Handshake::Hello { version: PROTOCOL_VERSION, device: self.info(), intent, stay_connected: self.stays(), invite };
        write_msg(&mut send, &hello).await?;
        let (device, peer_stays) = match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut recv)).await.map_err(|_| Error::Timeout)?? {
            Handshake::Welcome { device, stay_connected } if device.id == fp => (device, stay_connected),
            Handshake::Welcome { .. } => return Err(protocol("device id does not match its certificate")),
            Handshake::Reject { reason: RejectReason::NotPaired } => return Err(Error::NotPaired),
            Handshake::Reject { reason } => return Err(protocol(format!("rejected: {reason:?}"))),
            other => return Err(protocol(format!("unexpected {other:?}"))),
        };
        if intent == Intent::Pair {
            self.run_pairing(&conn, &mut send, &mut recv, &device, false, scanned).await?;
        }
        let _ = send.finish();
        self.register(conn, device.clone(), self.inner.id, peer_stays);
        Ok(device)
    }

    /// Both sides show the same code and each user confirms; pairing succeeds only if
    /// both accept. A decline on either side ends it on both. `preapproved` (QR invite)
    /// skips the question on this side.
    async fn run_pairing(
        &self,
        conn: &quinn::Connection,
        send: &mut quinn::SendStream,
        recv: &mut quinn::RecvStream,
        device: &DeviceInfo,
        incoming: bool,
        preapproved: bool,
    ) -> Result<()> {
        if preapproved {
            write_msg(send, &Handshake::PairDecision { accept: true }).await?;
            let remote = match tokio::time::timeout(PAIR_TIMEOUT, read_msg(recv)).await.map_err(|_| Error::Timeout)?? {
                Handshake::PairDecision { accept } => accept,
                other => return Err(protocol(format!("unexpected {other:?}"))),
            };
            return self.finish_pairing(conn, send, device, true, remote).await;
        }
        let code = tls::pairing_code(conn)?;
        let request = Uuid::new_v4();
        let answer = self.expect_answer(request);
        self.emit(Event::PairingRequest { request, device: device.clone(), code, incoming });

        let read_remote = async {
            match read_msg(recv).await? {
                Handshake::PairDecision { accept } => Ok(accept),
                other => Err(protocol(format!("unexpected {other:?}"))),
            }
        };
        tokio::pin!(read_remote);
        let local_answer = self.await_answer(request, answer, PAIR_TIMEOUT);
        tokio::pin!(local_answer);
        // Watch for the other side's decision while waiting for ours, so a decline over
        // there closes the prompt here right away.
        let mut remote: Option<bool> = None;
        let local = tokio::select! {
            local = &mut local_answer => local,
            r = &mut read_remote => match r {
                Ok(true) => {
                    remote = Some(true);
                    local_answer.await
                }
                Ok(false) => {
                    remote = Some(false);
                    self.inner.pending.lock().unwrap().remove(&request);
                    false
                }
                Err(e) => {
                    self.inner.pending.lock().unwrap().remove(&request);
                    return Err(e);
                }
            },
        };
        write_msg(send, &Handshake::PairDecision { accept: local }).await?;
        let remote = match remote {
            Some(r) => r,
            None if !local => false,
            None => tokio::time::timeout(PAIR_TIMEOUT, read_remote).await.map_err(|_| Error::Timeout)??,
        };

        self.finish_pairing(conn, send, device, local, remote).await
    }

    async fn finish_pairing(
        &self,
        conn: &quinn::Connection,
        send: &mut quinn::SendStream,
        device: &DeviceInfo,
        local: bool,
        remote: bool,
    ) -> Result<()> {
        if local && remote {
            self.inner.trust.lock().unwrap().add(device.clone(), Some(conn.remote_address()))?;
            self.emit(Event::Paired { device: device.clone() });
            Ok(())
        } else {
            let reason = if !local { "declined on this device" } else { "declined on the other device" };
            self.emit(Event::PairingFailed { device: device.clone(), reason: reason.into() });
            // Give the decision time to reach the peer before the connection drops.
            let _ = send.finish();
            let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            Err(Error::Declined)
        }
    }

    fn welcome(&self) -> Handshake {
        Handshake::Welcome { device: self.info(), stay_connected: self.stays() }
    }

    fn stays(&self) -> bool {
        self.inner.settings.lock().unwrap().stay_connected
    }

    fn register(&self, conn: quinn::Connection, device: DeviceInfo, dialer: Fingerprint, peer_stays: bool) {
        let id = device.id;
        {
            let mut peers = self.inner.peers.lock().unwrap();
            if let Some(existing) = peers.get(&id).filter(|p| p.conn.stable_id() != conn.stable_id()) {
                // A cable beats the network, which beats away from home; both sides see which
                // link each connection is. Otherwise both sides dialed at once: keep the
                // connection opened by the smaller id, which both sides agree on.
                let (new_link, old_link) = (link_of(conn.remote_address()), link_of(existing.conn.remote_address()));
                let keep_old = if new_link != old_link { old_link > new_link } else { existing.dialer < dialer };
                if existing.conn.close_reason().is_none() && keep_old {
                    conn.close(0u32.into(), b"duplicate");
                    return;
                }
                existing.conn.close(0u32.into(), b"duplicate");
            }
            peers.insert(id, Peer { conn: conn.clone(), device: device.clone(), dialer, activity: Activity::new(), peer_stays });
        }
        if let Err(e) = self.inner.trust.lock().unwrap().touch(&device, conn.remote_address()) {
            warn!("could not save device address: {e}");
        }
        self.emit(Event::Connected { device: device.clone() });
        tokio::spawn(self.clone().serve_peer(conn, device));
    }

    async fn serve_peer(self, conn: quinn::Connection, device: DeviceInfo) {
        loop {
            tokio::select! {
                bi = conn.accept_bi() => match bi {
                    Ok((send, recv)) => { tokio::spawn(self.clone().handle_stream(device.clone(), send, recv)); }
                    Err(_) => break,
                },
                uni = conn.accept_uni() => match uni {
                    Ok(mut recv) => {
                        if let Some((_, a)) = self.live(&device.id) {
                            a.touch();
                        }
                        let node = self.clone();
                        let from = device.clone();
                        tokio::spawn(async move {
                            match read_msg::<Message>(&mut recv).await {
                                Ok(Message::Notify { title, body }) => node.emit(Event::Notification { from, title, body }),
                                Ok(Message::Clipboard { text }) if text.len() <= MAX_CLIPBOARD => {
                                    node.emit(Event::Clipboard { from, text })
                                }
                                Ok(Message::Clipboard { .. }) => debug!("oversized clipboard from {}", from.name),
                                Ok(Message::Renamed { name }) => node.peer_renamed(from.id, name),
                                Ok(Message::Ping) => {}
                                Ok(Message::OpenUrl { url }) if is_web_link(&url) => node.emit(Event::OpenUrl { from, url }),
                                Ok(Message::OpenUrl { .. }) => debug!("refused a link from {} that isn't http(s)", from.name),
                                Ok(Message::Battery(battery)) if battery.level <= 100 => {
                                    let changed = node.inner.batteries.lock().unwrap().insert(from.id, battery) != Some(battery);
                                    if changed {
                                        node.emit(Event::Battery { from, battery });
                                    }
                                }
                                Ok(Message::Battery(_)) => debug!("bad battery level from {}", from.name),
                                Ok(Message::NotificationPosted(n))
                                    if !n.key.is_empty() && n.key.len() <= MAX_KEY && n.icon.as_ref().is_none_or(|i| i.len() <= MAX_ICON) =>
                                {
                                    node.emit(Event::NotificationPosted { from, notification: n })
                                }
                                Ok(Message::NotificationPosted(_)) => debug!("bad notification from {}", from.name),
                                Ok(Message::NotificationRemoved { key }) => node.emit(Event::NotificationRemoved { from, key }),
                                Ok(Message::NotificationReply { key, text }) => node.emit(Event::NotificationReply { from, key, text }),
                                Ok(Message::NotificationAction { key, action }) => node.emit(Event::NotificationAction { from, key, action }),
                                Ok(Message::NotificationDismiss { key }) => node.emit(Event::NotificationDismissed { from, key }),
                                Ok(Message::MediaState(state)) => {
                                    let newer = {
                                        let mut seqs = node.inner.media_seq.lock().unwrap();
                                        let last = seqs.entry(from.id).or_insert(0);
                                        let newer = state.seq > *last;
                                        if newer {
                                            *last = state.seq;
                                        }
                                        newer
                                    };
                                    if newer {
                                        node.emit(Event::MediaState { from, state });
                                    }
                                }
                                Ok(Message::MediaCommand { player, command }) => node.emit(Event::MediaCommand { from, player, command }),
                                Ok(Message::MediaWatch { on }) => node.emit(Event::MediaWatch { from, on }),
                                Ok(Message::CallState { state, contact }) => node.emit(Event::CallState { from, state, contact }),
                                Ok(Message::Heartbeat) => node.emit(Event::Heartbeat { from }),
                                Ok(Message::ClipboardWatch { on }) => node.emit(Event::ClipboardWatch { from, on }),
                                Ok(Message::CameraState { session, state }) => node.emit(Event::CameraState { from, session, state }),
                                Ok(Message::Locate) => node.emit(Event::LocateRequest { from }),
                                Ok(Message::TerminalList) => node.emit(Event::TerminalListRequest { from }),
                                Ok(Message::Terminals { list }) => node.emit(Event::Terminals { from, list }),
                                Ok(Message::Location { lat, lon, accuracy_m, at_ms }) => {
                                    if lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0 {
                                        node.emit(Event::Location { from, lat, lon, accuracy_m: accuracy_m.filter(|a| a.is_finite() && *a >= 0.0), at_ms });
                                    }
                                }
                                Ok(Message::LocationRefused { reason }) => {
                                    node.emit(Event::LocationRefused { from, reason: reason.chars().take(200).collect() })
                                }
                                Ok(Message::Picture { image }) => {
                                    if image.len() <= MAX_PICTURE {
                                        node.emit(Event::Picture { from, image });
                                    }
                                }
                                Ok(Message::SmsListThreads { limit }) => node.emit(Event::SmsListThreads { from, limit }),
                                Ok(Message::SmsListMessages { thread, before_ms, limit }) => {
                                    node.emit(Event::SmsListMessages { from, thread, before_ms, limit })
                                }
                                Ok(Message::SmsSend { client_id, to, text }) => node.emit(Event::SmsSend { from, client_id, to, text }),
                                Ok(Message::SmsThreads { threads }) => node.emit(Event::SmsThreads { from, threads }),
                                Ok(Message::SmsMessages { thread, messages, more }) => node.emit(Event::SmsMessages { from, thread, messages, more }),
                                Ok(Message::SmsMessage { thread, message }) => node.emit(Event::SmsMessage { from, thread, message }),
                                Ok(Message::SmsSent { client_id, ok, error }) => node.emit(Event::SmsSent { from, client_id, ok, error }),
                                Ok(Message::HeartbeatWatch { on, every_s }) => node.emit(Event::HeartbeatWatch { from, on, every_s: every_s.max(5) }),
                                Ok(Message::Ring) => node.emit(Event::Ring { from, on: true }),
                                Ok(Message::RingStop) => node.emit(Event::Ring { from, on: false }),
                                Err(e) => debug!("bad message from {}: {e}", from.name),
                            }
                        });
                    }
                    Err(_) => break,
                },
            }
        }
        let mut peers = self.inner.peers.lock().unwrap();
        if peers.get(&device.id).is_some_and(|p| p.conn.stable_id() == conn.stable_id()) {
            peers.remove(&device.id);
            drop(peers);
            self.emit(Event::Disconnected { id: device.id });
        }
    }

    fn peer_renamed(&self, id: Fingerprint, name: String) {
        let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(crate::settings::MAX_NAME).collect();
        if name.is_empty() {
            return;
        }
        let device = {
            let mut peers = self.inner.peers.lock().unwrap();
            let Some(p) = peers.get_mut(&id) else { return };
            p.device.name = name;
            (p.device.clone(), p.conn.remote_address())
        };
        if let Err(e) = self.inner.trust.lock().unwrap().touch(&device.0, device.1) {
            warn!("could not save device name: {e}");
        }
        self.emit(Event::DeviceUpdated { device: device.0 });
    }

    /// A new bidirectional stream: a file offer or a media session request.
    async fn handle_stream(self, from: DeviceInfo, send: quinn::SendStream, mut recv: quinn::RecvStream) {
        let first = match read_value(&mut recv).await {
            Ok(v) => v,
            Err(e) => return debug!("bad stream from {}: {e}", from.name),
        };
        match first.get("type").and_then(|t| t.as_str()) {
            Some("offer") => match serde_json::from_value(first) {
                Ok(TransferMsg::Offer { id, files, import, clipboard }) => self.handle_offer(from, send, recv, id, files, import, clipboard).await,
                _ => debug!("bad offer from {}", from.name),
            },
            Some("session_request") => match serde_json::from_value(first) {
                Ok(SessionMsg::SessionRequest { id, kind, terminal }) => self.handle_session(from, send, recv, id, kind, terminal).await,
                _ => debug!("bad session request from {}", from.name),
            },
            Some("fs") => {
                let mut first = first;
                if let Some(o) = first.as_object_mut() {
                    o.remove("type");
                }
                let Ok(request) = serde_json::from_value::<FsRequest>(first) else {
                    return debug!("bad file request from {}", from.name);
                };
                let away_refused = !self.allowed_from_here(&from.id);
                let call = FsCall::new(from, request, send, recv);
                if away_refused {
                    let _ = call.error("this device shares its files only on the same network", false).await;
                    return;
                }
                let served = self.inner.fs.lock().unwrap().is_none();
                let unserved = if served { self.inner.fs_tx.send(call).err().map(|e| e.0) } else { Some(call) };
                if let Some(call) = unserved {
                    let _ = call.error("this device doesn't share its files", false).await;
                }
            }
            other => debug!("unknown stream {other:?} from {}", from.name),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_session(self, from: DeviceInfo, mut send: quinn::SendStream, recv: quinn::RecvStream, id: Uuid, kind: SessionKind, terminal: Option<crate::protocol::TerminalOpen>) {
        let decline = |reason: &str, tries_left: Option<u32>| SessionMsg::SessionDecline { reason: Some(reason.into()), tries_left };
        // From away, screens, control, camera, microphone and terminals need the user's yes
        // for this device beforehand; it isn't even asked otherwise.
        if !self.allowed_from_here(&from.id) {
            let _ = write_msg(&mut send, &decline("away", None)).await;
            let _ = send.finish();
            return;
        }
        let busy = self.live(&from.id).map(|(_, a)| a.hold());
        // A terminal: whoever runs them judges the code first.
        let mut terminal_id = None;
        let mut ask = true;
        if kind == SessionKind::Terminal {
            let verdict = match terminal {
                Some(open) => {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let call = TerminalCall { request: id, from: from.clone(), open, verdict: tx };
                    match self.inner.terminals_tx.send(call) {
                        Ok(()) => tokio::time::timeout(Duration::from_secs(20), rx).await.ok().and_then(|v| v.ok()),
                        Err(_) => None,
                    }
                }
                None => None,
            };
            match verdict.unwrap_or(Verdict::Decline { reason: "unsupported".into(), tries_left: None }) {
                Verdict::Accept { terminal } => {
                    terminal_id = Some(terminal);
                    ask = false;
                }
                Verdict::Confirm { terminal } => terminal_id = Some(terminal),
                Verdict::Decline { reason, tries_left } => {
                    let _ = write_msg(&mut send, &decline(&reason, tries_left)).await;
                    let _ = send.finish();
                    return;
                }
            }
        }
        // Otherwise always asks: nothing like auto-accept applies to someone seeing your
        // screen.
        let accepted = !ask || {
            let answer = self.expect_answer(id);
            self.emit(Event::SessionRequest { request: id, from: from.clone(), kind });
            self.await_answer(id, answer, OFFER_TIMEOUT).await
        };
        let unsupported = self.inner.unsupported.lock().unwrap().remove(&id);
        let reply = match (accepted, unsupported) {
            (true, _) => {
                let size = self.inner.session_sizes.lock().unwrap().remove(&id);
                SessionMsg::SessionAccept { width: size.map(|s| s.0), height: size.map(|s| s.1), terminal: terminal_id }
            }
            (false, true) => decline("unsupported", None),
            (false, false) => SessionMsg::SessionDecline { reason: None, tries_left: None },
        };
        if write_msg(&mut send, &reply).await.is_err() || !accepted {
            let _ = send.finish();
            return;
        }
        let sharing = !kind.requester_shares();
        self.emit(Event::SessionStarted { session: id, peer: from.clone(), kind, sharing });
        let session = Session { id, peer: from, kind, sharing, size: None, send, recv, _busy: busy, events: self.inner.events.clone() };
        // Nobody to run it (no daemon took the receiver): dropping ends it.
        let _ = self.inner.sessions_tx.send(session);
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_offer(self, from: DeviceInfo, mut send: quinn::SendStream, mut recv: quinn::RecvStream, id: Uuid, files: Vec<FileEntry>, import: bool, clipboard: bool) {
        let total: u64 = files.iter().map(|f| f.size).sum();
        let busy = self.live(&from.id).map(|(_, a)| a.hold());
        // Clipboard files: one small file, into a folder of their own, kept until the next.
        let clipboard_to = if clipboard {
            let total: u64 = files.iter().map(|f| f.size).sum();
            if files.len() != 1 || total > MAX_CLIPBOARD_FILE {
                let _ = write_msg(&mut send, &TransferMsg::Decline).await;
                let _ = send.finish();
                return;
            }
            let dir = self.inner.config.data_dir.join("clipboard");
            let _ = std::fs::remove_dir_all(&dir);
            Some(dir)
        } else {
            None
        };
        // Imports go to their own folder, never ask, and are refused where not taken.
        let import_to = if clipboard_to.is_some() {
            clipboard_to.clone()
        } else if import {
            let base = self.inner.import_dir.lock().unwrap().clone();
            match base {
                Some(base) => Some(base.join(folder_name(&from.name))),
                None => {
                    let _ = write_msg(&mut send, &TransferMsg::Decline).await;
                    let _ = send.finish();
                    return;
                }
            }
        } else {
            None
        };
        let auto = import_to.is_some() || self.inner.settings.lock().unwrap().auto_accept;
        let accepted = auto || {
            let answer = self.expect_answer(id);
            self.emit(Event::TransferRequest { transfer: id, from: from.clone(), files: files.clone(), total });
            self.await_answer(id, answer, OFFER_TIMEOUT).await
        };
        if !accepted {
            let _ = write_msg(&mut send, &TransferMsg::Decline).await;
            let _ = send.finish();
            self.emit(Event::TransferFailed { transfer: id, reason: "declined".into() });
            return;
        }

        let node = self.clone();
        let from_clip = from.clone();
        let to_clipboard = clipboard_to.is_some();
        let task = tokio::spawn(async move {
            let result = async {
                let dir = match import_to {
                    Some(dir) => dir,
                    None => node.inner.settings.lock().unwrap().download_dir.clone(),
                };
                let plan = transfer::plan(&dir, &files).await?;
                let offsets: Vec<u64> = plan.iter().map(|f| f.offset).collect();
                let start = offsets.iter().sum();
                write_msg(&mut send, &TransferMsg::Accept { offsets }).await?;
                // A clipboard file isn't a download: it shows up as `ClipboardFile` only.
                if !to_clipboard {
                    node.emit(Event::TransferStarted { transfer: id, peer: from, direction: Direction::Incoming, files, total, import });
                }
                let events = node.inner.events.clone();
                let mut report = move |bytes| {
                    let _ = events.send(Event::TransferProgress { transfer: id, bytes, total });
                };
                let mut progress = Progress::new(start, &mut report);
                transfer::receive_files(&mut send, &mut recv, &plan, &mut progress).await
            }
            .await;
            node.inner.transfers.lock().unwrap().remove(&id);
            drop(busy);
            if let (true, Ok(saved)) = (to_clipboard, &result)
                && let Some(path) = saved.first()
            {
                node.emit(Event::ClipboardFile { from: from_clip, path: path.clone() });
            }
            node.emit(match result {
                Ok(saved) => Event::TransferComplete { transfer: id, saved },
                Err(e) => Event::TransferFailed { transfer: id, reason: e.to_string() },
            });
        });
        self.inner.transfers.lock().unwrap().insert(id, task.abort_handle());
    }

    async fn discovery_loop(self, mut rx: mpsc::UnboundedReceiver<DiscoveryEvent>) {
        while let Some(ev) = rx.recv().await {
            match ev {
                DiscoveryEvent::Found { device, addrs } => self.device_found(device, addrs),
                DiscoveryEvent::Lost { id } => self.device_lost(&id),
            }
        }
    }

    /// Dial paired devices that are not connected, at their last known address and any
    /// address discovery found. Runs right away at startup, then periodically.
    async fn reconnect_loop(self) {
        let mut tick = tokio::time::interval(RECONNECT_EVERY);
        loop {
            tick.tick().await;
            if self.stays() {
                self.reconnect_all();
            }
        }
    }

    fn reconnect_all(&self) {
        let targets: Vec<Fingerprint> =
            self.inner.trust.lock().unwrap().list().into_iter().map(|t| t.device.id).filter(|id| !self.is_connected(id)).collect();
        for id in targets {
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(e) = node.ensure_connection(&id).await {
                    debug!("reconnect to {} failed: {e}", id.short());
                }
            });
        }
    }

    /// Close connections that have had nothing going on for `idle_after`. Both sides run
    /// this; whichever notices first closes, and the next send dials again.
    async fn idle_loop(self) {
        let idle = self.inner.config.idle_after;
        let period = (idle / 4).clamp(Duration::from_millis(100), Duration::from_secs(15));
        loop {
            tokio::time::sleep(period).await;
            if self.stays() {
                continue;
            }
            for p in self.inner.peers.lock().unwrap().values() {
                if !p.peer_stays && p.activity.idle_for().is_some_and(|d| d >= idle) {
                    debug!("closing idle connection to {}", p.device.name);
                    p.conn.close(0u32.into(), b"idle");
                }
            }
        }
    }
}

/// Away from home (docs/protocol.md §4.13).
impl Node {
    async fn start_away(&self) {
        // The user's own relay (settings) wins over the one the service lists (config).
        let relay = self.settings().relay.or_else(|| self.inner.config.relay.clone());
        let away = match crate::away::Away::start(&self.inner.config.data_dir, relay.as_deref()).await {
            Ok(a) => a,
            Err(e) => return warn!("away from home is off: {e}"),
        };
        self.inner.info.lock().unwrap().away = Some(away.addr());
        *self.inner.away.lock().unwrap() = Some(away.clone());
        let port = match self.local_addr() {
            Ok(a) => a.port(),
            Err(_) => return,
        };
        // Only paired devices met on the same network or cable within the week.
        let node = self.clone();
        tokio::spawn(away.clone().accept_loop(port, move |iroh_id| node.away_allowed(iroh_id)));
        // Its addresses change (networks, the relay's answer): what the next handshake says.
        let node = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                // Replaced (another relay) or stopped: this one has nothing to say.
                if !node.inner.away.lock().unwrap().as_ref().is_some_and(|a| Arc::ptr_eq(a, &away)) {
                    break;
                }
                let now = away.addr();
                node.inner.info.lock().unwrap().away = Some(now);
            }
        });
    }

    /// Close the away endpoint, if it runs; its address leaves the handshake.
    async fn stop_away(&self) {
        let away = self.inner.away.lock().unwrap().take();
        if let Some(away) = away {
            self.inner.info.lock().unwrap().away = None;
            let _ = tokio::time::timeout(Duration::from_secs(2), away.close()).await;
        }
    }

    fn away_allowed(&self, iroh_id: &str) -> bool {
        if !self.settings().away {
            return false;
        }
        let trust = self.inner.trust.lock().unwrap();
        trust.list().iter().any(|t| t.device.away.as_ref().is_some_and(|a| a.id == iroh_id) && t.lease_ok())
    }

    /// How a connected device is reached now.
    pub fn link(&self, id: &Fingerprint) -> Option<Link> {
        self.inner.peers.lock().unwrap().get(id).map(|p| link_of(p.conn.remote_address()))
    }

    /// Allow everything from away for a paired device (screens, files, camera…), or only
    /// the small things.
    pub fn set_away_all(&self, id: &Fingerprint, on: bool) -> Result<()> {
        if !self.inner.trust.lock().unwrap().set_away_all(id, on)? {
            return Err(Error::NotPaired);
        }
        self.emit(Event::DeviceUpdated { device: self.peer_info(id).or_else(|| self.inner.trust.lock().unwrap().get(id).map(|t| t.device.clone())).ok_or(Error::NotPaired)? });
        Ok(())
    }

    /// Whether `from` may have `what` now: on the same network or cable always; away only
    /// if its user allowed everything from away for it.
    fn allowed_from_here(&self, from: &Fingerprint) -> bool {
        self.link(from) != Some(Link::Away) || self.inner.trust.lock().unwrap().get(from).is_some_and(|t| t.away_all)
    }

    /// Connect to a paired device away from home: through iroh, then Linksy's own
    /// handshake with the pinned key. For [`Node::ensure_connection`] when the network
    /// doesn't answer; public for tests.
    pub async fn connect_away(&self, id: &Fingerprint) -> Result<DeviceInfo> {
        let away = self.inner.away.lock().unwrap().clone().ok_or(Error::Unreachable)?;
        if !self.settings().away {
            return Err(Error::Unreachable);
        }
        let to = {
            let trust = self.inner.trust.lock().unwrap();
            let t = trust.get(id).ok_or(Error::NotPaired)?;
            if !t.lease_ok() {
                return Err(Error::Unreachable);
            }
            t.device.away.clone().ok_or(Error::Unreachable)?
        };
        let local = away.dial(&to).await?;
        let conn = self.race_connect(&[local], id).await?;
        tokio::time::timeout(HANDSHAKE_TIMEOUT, self.handshake(conn, Intent::Connect, None)).await.map_err(|_| Error::Timeout)??;
        self.peer_info(id).ok_or(Error::Unreachable)
    }
}

/// An `http(s)` link with something after the scheme, no longer than [`MAX_URL`]: the only
/// kind a device may ask another to open.
fn is_web_link(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &url[8..]
    } else if lower.starts_with("http://") {
        &url[7..]
    } else {
        return false;
    };
    !rest.is_empty() && url.len() <= MAX_URL && !url.chars().any(char::is_control)
}

/// A device name as a folder name: no separators, nothing hidden or empty.
fn folder_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() { "Phone".into() } else { clean }
}

fn bind_endpoint(identity: &Identity, addr: SocketAddr) -> Result<quinn::Endpoint> {
    // Prefer a dual-stack socket; fall back to IPv4 where IPv6 is unavailable.
    let mut endpoint = match quinn::Endpoint::server(tls::server_config(identity)?, addr) {
        Ok(e) => e,
        Err(e) if addr.ip() == IpAddr::V6(Ipv6Addr::UNSPECIFIED) => {
            warn!("IPv6 bind failed ({e}), using IPv4");
            let v4 = SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), addr.port());
            quinn::Endpoint::server(tls::server_config(identity)?, v4)?
        }
        Err(e) => return Err(e.into()),
    };
    endpoint.set_default_client_config(tls::client_config(identity)?);
    Ok(endpoint)
}
