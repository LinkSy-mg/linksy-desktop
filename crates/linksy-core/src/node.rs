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
use crate::invite::{self, Invite};
use crate::settings::Settings;
use crate::protocol::{
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
    TransferStarted { transfer: Uuid, peer: DeviceInfo, direction: Direction, files: Vec<FileEntry>, total: u64 },
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
}

/// A running media session: the stream pair, kept open (and the connection with it) until
/// dropped. Reports `SessionEnded` when dropped.
pub struct Session {
    pub id: Uuid,
    pub peer: DeviceInfo,
    pub kind: SessionKind,
    /// This device shares; the peer watches.
    pub sharing: bool,
    pub send: quinn::SendStream,
    pub recv: quinn::RecvStream,
    _busy: Option<Busy>,
    events: broadcast::Sender<Event>,
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
    /// Session requests answered "this device can't do that", not "no".
    unsupported: Mutex<std::collections::HashSet<Uuid>>,
    /// The token of the pairing QR code on screen, and when it expires.
    invite: Mutex<Option<(String, Instant)>>,
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
            os: std::env::consts::OS.to_string(),
        };

        let endpoint = bind_endpoint(&identity, SocketAddr::new(config.bind, config.port))?;
        let (events, _) = broadcast::channel(1024);
        let (sessions_tx, sessions) = mpsc::unbounded_channel();
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
                sessions: Mutex::new(Some(sessions)),
                sessions_tx,
                unsupported: Mutex::new(Default::default()),
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
        let trust = self.inner.trust.lock().unwrap();
        let peers = self.inner.peers.lock().unwrap();
        let nearby = self.inner.nearby.lock().unwrap();
        let mut out: Vec<DeviceView> = trust
            .list()
            .into_iter()
            .map(|t| DeviceView {
                connected: peers.contains_key(&t.device.id),
                nearby: nearby.contains_key(&t.device.id),
                // Newest name we know: the live connection, then the network announcement.
                device: peers
                    .get(&t.device.id)
                    .map(|p| p.device.clone())
                    .or_else(|| nearby.get(&t.device.id).map(|(d, _)| d.clone()))
                    .unwrap_or(t.device),
                trusted: true,
            })
            .collect();
        for (id, (device, _)) in nearby.iter() {
            if !trust.contains(id) {
                out.push(DeviceView { device: device.clone(), trusted: false, connected: false, nearby: true });
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
    pub fn respond(&self, request: Uuid, accept: bool) -> bool {
        match self.inner.pending.lock().unwrap().remove(&request) {
            Some(tx) => tx.send(accept).is_ok(),
            None => false,
        }
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
        let (conn, activity) = self.ensure_connection(to).await?;
        let busy = activity.hold();
        let peer = self.peer_info(to).ok_or(Error::Unreachable)?;
        let id = Uuid::new_v4();
        let (mut send, mut recv) = conn.open_bi().await?;
        write_msg(&mut send, &SessionMsg::SessionRequest { id, kind }).await?;
        match tokio::time::timeout(OFFER_TIMEOUT, read_msg(&mut recv)).await.map_err(|_| Error::Timeout)?? {
            SessionMsg::SessionAccept => {}
            SessionMsg::SessionDecline { reason: Some(r) } if r == "unsupported" => return Err(Error::Unsupported),
            SessionMsg::SessionDecline { .. } => return Err(Error::Declined),
            other => return Err(protocol(format!("unexpected {other:?}"))),
        }
        let sharing = kind.requester_shares();
        self.emit(Event::SessionStarted { session: id, peer: peer.clone(), kind, sharing });
        Ok(Session { id, peer, kind, sharing, send, recv, _busy: Some(busy), events: self.inner.events.clone() })
    }

    /// Sessions this device accepted. Can be taken once; the taker runs the media (capture
    /// and encoding when `sharing`, showing it otherwise) and drops each when done.
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
                write_msg(&mut send, &TransferMsg::Offer { id, files: entries.clone() }).await?;
                node.emit(Event::TransferStarted { transfer: id, peer, direction: Direction::Outgoing, files: entries, total });
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
    }

    // ---- internals ----

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
        addrs.extend(last_addr.filter(|a| !addrs.contains(a)));
        let conn = self.race_connect(&addrs, id).await?;
        tokio::time::timeout(HANDSHAKE_TIMEOUT, self.handshake(conn, Intent::Connect, None)).await.map_err(|_| Error::Timeout)??;
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
                // Both sides dialed at once: keep the connection opened by the smaller id,
                // which both sides agree on without talking.
                if existing.conn.close_reason().is_none() && existing.dialer < dialer {
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
                Ok(TransferMsg::Offer { id, files }) => self.handle_offer(from, send, recv, id, files).await,
                _ => debug!("bad offer from {}", from.name),
            },
            Some("session_request") => match serde_json::from_value(first) {
                Ok(SessionMsg::SessionRequest { id, kind }) => self.handle_session(from, send, recv, id, kind).await,
                _ => debug!("bad session request from {}", from.name),
            },
            other => debug!("unknown stream {other:?} from {}", from.name),
        }
    }

    async fn handle_session(self, from: DeviceInfo, mut send: quinn::SendStream, recv: quinn::RecvStream, id: Uuid, kind: SessionKind) {
        let busy = self.live(&from.id).map(|(_, a)| a.hold());
        // Always asks: nothing like auto-accept applies to someone seeing your screen.
        let answer = self.expect_answer(id);
        self.emit(Event::SessionRequest { request: id, from: from.clone(), kind });
        let accepted = self.await_answer(id, answer, OFFER_TIMEOUT).await;
        let unsupported = self.inner.unsupported.lock().unwrap().remove(&id);
        let reply = match (accepted, unsupported) {
            (true, _) => SessionMsg::SessionAccept,
            (false, true) => SessionMsg::SessionDecline { reason: Some("unsupported".into()) },
            (false, false) => SessionMsg::SessionDecline { reason: None },
        };
        if write_msg(&mut send, &reply).await.is_err() || !accepted {
            let _ = send.finish();
            return;
        }
        let sharing = !kind.requester_shares();
        self.emit(Event::SessionStarted { session: id, peer: from.clone(), kind, sharing });
        let session = Session { id, peer: from, kind, sharing, send, recv, _busy: busy, events: self.inner.events.clone() };
        // Nobody to run it (no daemon took the receiver): dropping ends it.
        let _ = self.inner.sessions_tx.send(session);
    }

    async fn handle_offer(self, from: DeviceInfo, mut send: quinn::SendStream, mut recv: quinn::RecvStream, id: Uuid, files: Vec<FileEntry>) {
        let total: u64 = files.iter().map(|f| f.size).sum();
        let busy = self.live(&from.id).map(|(_, a)| a.hold());
        let auto = self.inner.settings.lock().unwrap().auto_accept;
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
        let task = tokio::spawn(async move {
            let result = async {
                let download_dir = node.inner.settings.lock().unwrap().download_dir.clone();
                let plan = transfer::plan(&download_dir, &files).await?;
                let offsets: Vec<u64> = plan.iter().map(|f| f.offset).collect();
                let start = offsets.iter().sum();
                write_msg(&mut send, &TransferMsg::Accept { offsets }).await?;
                node.emit(Event::TransferStarted { transfer: id, peer: from, direction: Direction::Incoming, files, total });
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
