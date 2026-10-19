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
use crate::error::{Error, Result, protocol};
use crate::identity::{DeviceInfo, DeviceKind, Fingerprint, Identity};
use crate::protocol::{
    FileEntry, Handshake, Intent, Message, PROTOCOL_VERSION, RejectReason, TransferMsg, read_msg, write_msg,
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
    Notification { from: DeviceInfo, title: String, body: String },
    /// A paired device shared its clipboard; front ends decide where it goes.
    Clipboard { from: DeviceInfo, text: String },
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
    info: DeviceInfo,
    config: Config,
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
        let info = DeviceInfo {
            id: identity.fingerprint,
            name: config.name.clone(),
            kind: config.kind,
            os: std::env::consts::OS.to_string(),
        };

        let endpoint = bind_endpoint(&identity, SocketAddr::new(config.bind, config.port))?;
        let (events, _) = broadcast::channel(1024);
        let node = Self {
            inner: Arc::new(Inner {
                info,
                config,
                endpoint,
                trust: Mutex::new(trust),
                peers: Mutex::new(HashMap::new()),
                nearby: Mutex::new(HashMap::new()),
                pending: Mutex::new(HashMap::new()),
                transfers: Mutex::new(HashMap::new()),
                events,
                discovery: Mutex::new(None),
                dialing: Mutex::new(HashMap::new()),
            }),
        };

        tokio::spawn(node.clone().accept_loop());
        if node.inner.config.discovery {
            let (tx, rx) = mpsc::unbounded_channel();
            let d = Discovery::start(&node.inner.info, node.local_addr()?.port(), tx)?;
            *node.inner.discovery.lock().unwrap() = Some(d);
            tokio::spawn(node.clone().discovery_loop(rx));
        }
        if node.inner.config.stay_connected {
            tokio::spawn(node.clone().reconnect_loop());
        } else {
            tokio::spawn(node.clone().idle_loop());
        }
        info!(id = %node.inner.info.id.short(), addr = %node.local_addr()?, "linksy node started");
        Ok(node)
    }

    pub fn info(&self) -> &DeviceInfo {
        &self.inner.info
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
                device: peers.get(&t.device.id).map(|p| p.device.clone()).unwrap_or(t.device),
                trusted: true,
            })
            .collect();
        for (id, (device, _)) in nearby.iter() {
            if !trust.contains(id) {
                out.push(DeviceView { device: device.clone(), trusted: false, connected: false, nearby: true });
            }
        }
        out.sort_by(|a, b| (!a.connected, !a.trusted, &a.device.name).cmp(&(!b.connected, !b.trusted, &b.device.name)));
        out
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
            return Err(protocol("the clipboard is empty"));
        }
        if text.len() > MAX_CLIPBOARD {
            return Err(protocol("clipboard text is too large, send it as a file instead"));
        }
        self.send_message(to, &Message::Clipboard { text: text.into() }).await
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
            return Err(protocol("nothing to send"));
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
        self.inner.endpoint.wait_idle().await;
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
        let mut dials = JoinSet::new();
        for addr in addrs {
            let node = self.clone();
            dials.spawn(async move { tokio::time::timeout(DIAL_TIMEOUT, node.connect(addr)).await });
        }
        while let Some(r) = dials.join_next().await {
            match r {
                Ok(Ok(Ok(d))) if d.id == *id => break,
                Ok(Ok(Ok(d))) => debug!("expected {}, reached {} instead", id.short(), d.name),
                Ok(Ok(Err(e))) => debug!("dial failed: {e}"),
                _ => {}
            }
        }
        dials.abort_all();
        self.live(id).ok_or(Error::Unreachable)
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
        let (device, intent) = match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut recv)).await.map_err(|_| Error::Timeout)?? {
            Handshake::Hello { version, device, intent } => {
                if version != PROTOCOL_VERSION {
                    write_msg(&mut send, &Handshake::Reject { reason: RejectReason::IncompatibleVersion }).await?;
                    send.finish()?;
                    return Err(protocol(format!("peer speaks protocol v{version}")));
                }
                (device, intent)
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
                write_msg(&mut send, &Handshake::Welcome { device: self.inner.info.clone() }).await?;
                self.register(conn, device, fp);
            }
            Intent::Connect => {
                write_msg(&mut send, &Handshake::Reject { reason: RejectReason::NotPaired }).await?;
                send.finish()?;
                // Let the reject arrive before the connection goes away.
                let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            }
            Intent::Pair => {
                write_msg(&mut send, &Handshake::Welcome { device: self.inner.info.clone() }).await?;
                self.run_pairing(&conn, &mut send, &mut recv, &device, true).await?;
                self.register(conn, device, fp);
            }
        }
        Ok(())
    }

    async fn dial(&self, addr: SocketAddr, intent: Intent) -> Result<DeviceInfo> {
        let conn = self.inner.endpoint.connect(addr, "linksy")?.await?;
        let fp = tls::peer_fingerprint(&conn)?;
        if fp == self.inner.info.id {
            conn.close(0u32.into(), b"self");
            return Err(protocol("that address is this device"));
        }
        if intent == Intent::Connect && !self.inner.trust.lock().unwrap().contains(&fp) {
            conn.close(0u32.into(), b"not paired");
            return Err(Error::NotPaired);
        }
        let (mut send, mut recv) = conn.open_bi().await?;
        write_msg(&mut send, &Handshake::Hello { version: PROTOCOL_VERSION, device: self.inner.info.clone(), intent }).await?;
        let device = match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut recv)).await.map_err(|_| Error::Timeout)?? {
            Handshake::Welcome { device } if device.id == fp => device,
            Handshake::Welcome { .. } => return Err(protocol("device id does not match its certificate")),
            Handshake::Reject { reason: RejectReason::NotPaired } => return Err(Error::NotPaired),
            Handshake::Reject { reason } => return Err(protocol(format!("rejected: {reason:?}"))),
            other => return Err(protocol(format!("unexpected {other:?}"))),
        };
        if intent == Intent::Pair {
            self.run_pairing(&conn, &mut send, &mut recv, &device, false).await?;
        }
        let _ = send.finish();
        self.register(conn, device.clone(), self.inner.info.id);
        Ok(device)
    }

    /// Both sides show the same code and each user confirms; pairing succeeds only if
    /// both accept. A decline on either side ends it on both.
    async fn run_pairing(
        &self,
        conn: &quinn::Connection,
        send: &mut quinn::SendStream,
        recv: &mut quinn::RecvStream,
        device: &DeviceInfo,
        incoming: bool,
    ) -> Result<()> {
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

    fn register(&self, conn: quinn::Connection, device: DeviceInfo, dialer: Fingerprint) {
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
            peers.insert(id, Peer { conn: conn.clone(), device: device.clone(), dialer, activity: Activity::new() });
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
                    Ok((send, recv)) => { tokio::spawn(self.clone().handle_offer(device.clone(), send, recv)); }
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

    async fn handle_offer(self, from: DeviceInfo, mut send: quinn::SendStream, mut recv: quinn::RecvStream) {
        let (id, files) = match read_msg(&mut recv).await {
            Ok(TransferMsg::Offer { id, files }) => (id, files),
            Ok(other) => return debug!("expected offer, got {other:?}"),
            Err(e) => return debug!("bad offer: {e}"),
        };
        let total: u64 = files.iter().map(|f| f.size).sum();
        let busy = self.live(&from.id).map(|(_, a)| a.hold());
        let answer = self.expect_answer(id);
        self.emit(Event::TransferRequest { transfer: id, from: from.clone(), files: files.clone(), total });
        if !self.await_answer(id, answer, OFFER_TIMEOUT).await {
            let _ = write_msg(&mut send, &TransferMsg::Decline).await;
            let _ = send.finish();
            self.emit(Event::TransferFailed { transfer: id, reason: "declined".into() });
            return;
        }

        let node = self.clone();
        let task = tokio::spawn(async move {
            let result = async {
                let plan = transfer::plan(&node.inner.config.download_dir, &files).await?;
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
                DiscoveryEvent::Found { device, addrs } => {
                    if device.id == self.inner.info.id {
                        continue;
                    }
                    let known = self.inner.nearby.lock().unwrap().insert(device.id, (device.clone(), addrs.clone()));
                    if known.as_ref().map(|(d, a)| (d, a)) != Some((&device, &addrs)) {
                        self.emit(Event::DeviceFound { device: device.clone(), addrs: addrs.clone() });
                    }
                    if self.inner.config.stay_connected
                        && self.inner.trust.lock().unwrap().contains(&device.id)
                        && !self.is_connected(&device.id)
                    {
                        let node = self.clone();
                        tokio::spawn(async move { node.try_connect(&addrs).await });
                    }
                }
                DiscoveryEvent::Lost { id } => {
                    if self.inner.nearby.lock().unwrap().remove(&id).is_some() {
                        self.emit(Event::DeviceLost { id });
                    }
                }
            }
        }
    }

    /// Dial paired devices that are not connected, at their last known address and any
    /// address discovery found. Runs right away at startup, then periodically.
    async fn reconnect_loop(self) {
        let mut tick = tokio::time::interval(RECONNECT_EVERY);
        loop {
            tick.tick().await;
            let targets: Vec<(Fingerprint, Vec<SocketAddr>)> = {
                let trust = self.inner.trust.lock().unwrap();
                trust
                    .list()
                    .into_iter()
                    .filter(|t| !self.is_connected(&t.device.id))
                    .map(|t| {
                        let mut addrs = self.nearby_addrs(&t.device.id);
                        addrs.extend(t.last_addr.filter(|a| !addrs.contains(a)));
                        (t.device.id, addrs)
                    })
                    .collect()
            };
            for (_, addrs) in targets {
                let node = self.clone();
                tokio::spawn(async move { node.try_connect(&addrs).await });
            }
        }
    }

    /// Close connections that have had nothing going on for `idle_after`. Both sides run
    /// this; whichever notices first closes, and the next send dials again.
    async fn idle_loop(self) {
        let idle = self.inner.config.idle_after;
        let period = (idle / 4).clamp(Duration::from_millis(100), Duration::from_secs(15));
        loop {
            tokio::time::sleep(period).await;
            for p in self.inner.peers.lock().unwrap().values() {
                if p.activity.idle_for().is_some_and(|d| d >= idle) {
                    debug!("closing idle connection to {}", p.device.name);
                    p.conn.close(0u32.into(), b"idle");
                }
            }
        }
    }

    async fn try_connect(&self, addrs: &[SocketAddr]) {
        for &addr in addrs {
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, self.connect(addr)).await {
                Ok(Ok(d)) => return debug!("reconnected to {} at {addr}", d.name),
                Ok(Err(e)) => debug!("reconnect to {addr} failed: {e}"),
                Err(_) => debug!("reconnect to {addr} timed out"),
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
