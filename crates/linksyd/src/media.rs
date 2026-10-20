//! Media controls, both ways, over MPRIS (the D-Bus interface every Linux player speaks):
//! - this computer's players go to devices that ask (`media_watch`), and their commands
//!   (play, pause, next…) come back to the player;
//! - a phone's players appear here as MPRIS players of their own, so GNOME's media
//!   controls and the keyboard's media keys work on them.
//!
//! Nothing runs on D-Bus while no device is watching and no phone is playing.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use base64::Engine;
use futures_util::StreamExt;
use linksy_core::{DeviceInfo, Event, Fingerprint, MediaCommand, MediaPlayer, MediaState, Node};
use tokio::sync::{broadcast::error::RecvError, mpsc};
use tracing::{debug, warn};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{Connection, proxy::CacheProperties};

const MPRIS: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// The names Linksy publishes phones' players under; never sent back to a phone.
const OURS: &str = "org.mpris.MediaPlayer2.Linksy.";
/// Largest cover art passed along (as the protocol allows).
const MAX_ART: u64 = 48 * 1024;

pub async fn run(node: Node, art_dir: std::path::PathBuf) {
    let mut events = node.subscribe();
    let mut watchers: HashSet<Fingerprint> = HashSet::new();
    let mut local: Option<Local> = None;
    let mut phones = Phones { exported: HashMap::new(), art_dir };
    // Commands from GNOME's controls for a phone's player, run on this runtime.
    let (commands, mut queued) = mpsc::unbounded_channel::<(Fingerprint, String, MediaCommand)>();
    loop {
        tokio::select! {
            Some((to, player, command)) = queued.recv() => {
                if let Err(e) = node.media_command(&to, &player, command).await {
                    debug!("media command to the phone failed: {e}");
                }
            }
            Some(()) = async { match local.as_mut() { Some(l) => l.changed.recv().await, None => std::future::pending().await } } => {
                // Several signals tend to come together; send once they settle.
                // 150: measured carefully, then rounded to something that looks deliberate.
                tokio::time::sleep(Duration::from_millis(150)).await;
                if let Some(l) = local.as_mut() {
                    while l.changed.try_recv().is_ok() {}
                    let state = l.snapshot().await;
                    for id in watchers.clone() {
                        if node.send_media_state(&id, &state).await.is_err() {
                            watchers.remove(&id);
                        }
                    }
                }
            }
            event = events.recv() => match event {
                Ok(Event::MediaWatch { from, on: true }) => {
                    watchers.insert(from.id);
                    if local.is_none() {
                        local = Local::start().await.map_err(|e| warn!("can't read this computer's players: {e}")).ok();
                    }
                    if let Some(l) = local.as_mut() {
                        let state = l.snapshot().await;
                        let _ = node.send_media_state(&from.id, &state).await;
                    }
                }
                Ok(Event::MediaWatch { from, on: false }) => {
                    watchers.remove(&from.id);
                }
                Ok(Event::Disconnected { id }) => {
                    watchers.remove(&id);
                }
                Ok(Event::MediaCommand { player, command, .. }) => {
                    if let Some(l) = local.as_ref() {
                        l.command(&player, command).await;
                    }
                }
                Ok(Event::MediaState { from, state }) => phones.update(&from, state, &commands).await,
                Ok(Event::DeviceLost { id }) => phones.remove_device(&id),
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
        }
        if watchers.is_empty() {
            // Nobody's looking: stop listening to D-Bus.
            local = None;
        }
    }
}

/// This computer's players, read over D-Bus while some device watches them.
struct Local {
    bus: Connection,
    changed: mpsc::Receiver<()>,
    _listen: tokio::task::JoinHandle<()>,
}

impl Local {
    async fn start() -> zbus::Result<Self> {
        let bus = Connection::session().await?;
        let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).path(PATH)?.build();
        let mut props = zbus::MessageStream::for_match_rule(rule, &bus, None).await?;
        let owners = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg0ns(MPRIS.trim_end_matches('.'))?
            .build();
        let mut owners = zbus::MessageStream::for_match_rule(owners, &bus, None).await?;
        let (tx, changed) = mpsc::channel(8);
        let listen = tokio::spawn(async move {
            loop {
                let next = tokio::select! { m = props.next() => m, m = owners.next() => m };
                if next.is_none() || tx.try_send(()).is_err() && tx.is_closed() {
                    break;
                }
            }
        });
        Ok(Self { bus, changed, _listen: listen })
    }

    async fn names(&self) -> Vec<String> {
        let dbus = zbus::fdo::DBusProxy::new(&self.bus).await;
        let names = match dbus {
            Ok(d) => d.list_names().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let mut out: Vec<String> = names.into_iter().map(|n| n.to_string()).filter(|n| n.starts_with(MPRIS) && !n.starts_with(OURS)).collect();
        out.sort();
        out
    }

    async fn proxy(&self, name: &str, interface: &'static str) -> zbus::Result<zbus::Proxy<'static>> {
        zbus::proxy::Builder::new(&self.bus).destination(name.to_string())?.path(PATH)?.interface(interface)?.cache_properties(CacheProperties::No).build().await
    }

    async fn snapshot(&mut self) -> MediaState {
        let mut players = Vec::new();
        for name in self.names().await {
            match self.player(&name).await {
                Ok(p) => players.push(p),
                Err(e) => debug!("{name}: {e}"),
            }
        }
        let seq = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        MediaState { seq, players }
    }

    async fn player(&self, name: &str) -> zbus::Result<MediaPlayer> {
        let root = self.proxy(name, "org.mpris.MediaPlayer2").await?;
        let player = self.proxy(name, PLAYER).await?;
        let app: String = root.get_property("Identity").await.unwrap_or_else(|_| name[MPRIS.len()..].to_string());
        let status: String = player.get_property("PlaybackStatus").await?;
        let meta: HashMap<String, OwnedValue> = player.get_property("Metadata").await.unwrap_or_default();
        let text = |key: &str| meta.get(key).and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
        let artist = meta.get("xesam:artist").and_then(|v| Vec::<String>::try_from(v.clone()).ok()).map(|a| a.join(", ")).unwrap_or_default();
        let length = meta.get("mpris:length").and_then(|v| i64::try_from(v.clone()).ok().or_else(|| u64::try_from(v.clone()).ok().map(|u| u as i64)));
        let position: Option<i64> = player.get_property("Position").await.ok();
        let flag = |v: zbus::Result<bool>| v.unwrap_or(false);
        Ok(MediaPlayer {
            id: name[MPRIS.len()..].to_string(),
            app,
            title: text("xesam:title"),
            artist,
            album: text("xesam:album"),
            playing: status == "Playing",
            position_ms: position.filter(|p| *p >= 0).map(|p| p as u64 / 1000),
            length_ms: length.filter(|l| *l > 0).map(|l| l as u64 / 1000),
            can_next: flag(player.get_property("CanGoNext").await),
            can_previous: flag(player.get_property("CanGoPrevious").await),
            can_seek: flag(player.get_property("CanSeek").await),
            art: art(&text("mpris:artUrl")),
        })
    }

    async fn command(&self, player: &str, command: MediaCommand) {
        let name = format!("{MPRIS}{player}");
        // Only players that are here, never Linksy's own copies of a phone's.
        if name.starts_with(OURS) || !self.names().await.contains(&name) {
            return;
        }
        let Ok(proxy) = self.proxy(&name, PLAYER).await else { return };
        let result = match command {
            MediaCommand::Play => proxy.call_method("Play", &()).await.map(|_| ()),
            MediaCommand::Pause => proxy.call_method("Pause", &()).await.map(|_| ()),
            MediaCommand::PlayPause => proxy.call_method("PlayPause", &()).await.map(|_| ()),
            MediaCommand::Next => proxy.call_method("Next", &()).await.map(|_| ()),
            MediaCommand::Previous => proxy.call_method("Previous", &()).await.map(|_| ()),
            MediaCommand::Seek { position_ms } => {
                // Relative, so it works without the track id SetPosition wants.
                let now: i64 = proxy.get_property("Position").await.unwrap_or(0);
                proxy.call_method("Seek", &(position_ms as i64 * 1000 - now)).await.map(|_| ())
            }
        };
        if let Err(e) = result {
            debug!("{player}: {e}");
        }
    }
}

/// Cover art from a local file, small enough to pass along.
fn art(url: &str) -> Option<String> {
    let path = url.strip_prefix("file://")?;
    let path = percent_decode(path);
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > MAX_ART {
        return None;
    }
    Some(base64::engine::general_purpose::STANDARD.encode(std::fs::read(&path).ok()?))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Phones' players, each published as an MPRIS player of its own.
struct Phones {
    exported: HashMap<(Fingerprint, String), Exported>,
    art_dir: std::path::PathBuf,
}

struct Exported {
    /// Holds the bus name; dropping it takes the player away.
    bus: Connection,
}

impl Phones {
    async fn update(&mut self, from: &DeviceInfo, state: MediaState, commands: &mpsc::UnboundedSender<(Fingerprint, String, MediaCommand)>) {
        let present: HashSet<&str> = state.players.iter().map(|p| p.id.as_str()).collect();
        self.exported.retain(|(dev, id), _| *dev != from.id || present.contains(id.as_str()));
        for p in state.players {
            let key = (from.id, p.id.clone());
            let art = p.art.as_deref().and_then(|a| self.save_art(from, &p.id, a));
            let fresh = PhonePlayer { device: from.clone(), player: p, art, commands: commands.clone() };
            match self.exported.get(&key) {
                Some(e) => {
                    if let Err(err) = refresh(&e.bus, fresh).await {
                        debug!("updating a phone's player: {err}");
                    }
                }
                None => {
                    let n = self.exported.len();
                    let name = format!("{OURS}d{}p{n}", from.id.short().replace('-', ""));
                    match export(&name, fresh).await {
                        Ok(bus) => {
                            self.exported.insert(key, Exported { bus });
                        }
                        Err(err) => warn!("can't show {}'s player: {err}", from.name),
                    }
                }
            }
        }
    }

    fn remove_device(&mut self, id: &Fingerprint) {
        self.exported.retain(|(dev, _), _| dev != id);
    }

    fn save_art(&self, from: &DeviceInfo, player: &str, b64: &str) -> Option<String> {
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
        let safe: String = format!("{}-{player}", from.id.short()).chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        let path = self.art_dir.join(format!("{safe}.img"));
        if std::fs::read(&path).ok().as_deref() != Some(&bytes[..]) {
            std::fs::create_dir_all(&self.art_dir).ok()?;
            std::fs::write(&path, &bytes).ok()?;
        }
        Some(format!("file://{}", path.display()))
    }
}

async fn export(name: &str, player: PhonePlayer) -> zbus::Result<Connection> {
    let root = Root { identity: format!("{} on {}", player.player.app, player.device.name) };
    zbus::connection::Builder::session()?.name(name.to_string())?.serve_at(PATH, root)?.serve_at(PATH, player)?.build().await
}

async fn refresh(bus: &Connection, fresh: PhonePlayer) -> zbus::Result<()> {
    let iface = bus.object_server().interface::<_, PhonePlayer>(PATH).await?;
    let mut p = iface.get_mut().await;
    *p = fresh;
    let emitter = iface.signal_emitter();
    p.playback_status_changed(emitter).await?;
    p.metadata_changed(emitter).await?;
    p.can_go_next_changed(emitter).await?;
    p.can_go_previous_changed(emitter).await?;
    p.can_seek_changed(emitter).await?;
    Ok(())
}

struct Root {
    identity: String,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}
    fn quit(&self) {}
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        crate::notify::DESKTOP_ENTRY.into()
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

/// One of a phone's players, as GNOME sees it. Its buttons send commands to the phone.
struct PhonePlayer {
    device: DeviceInfo,
    player: MediaPlayer,
    art: Option<String>,
    commands: mpsc::UnboundedSender<(Fingerprint, String, MediaCommand)>,
}

impl PhonePlayer {
    fn send(&self, command: MediaCommand) {
        let _ = self.commands.send((self.device.id, self.player.id.clone(), command));
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl PhonePlayer {
    fn play(&self) {
        self.send(MediaCommand::Play);
    }
    fn pause(&self) {
        self.send(MediaCommand::Pause);
    }
    fn play_pause(&self) {
        self.send(MediaCommand::PlayPause);
    }
    fn stop(&self) {
        self.send(MediaCommand::Pause);
    }
    fn next(&self) {
        self.send(MediaCommand::Next);
    }
    fn previous(&self) {
        self.send(MediaCommand::Previous);
    }
    fn seek(&self, offset: i64) {
        let now = self.player.position_ms.unwrap_or(0) as i64 * 1000;
        self.send(MediaCommand::Seek { position_ms: ((now + offset).max(0) / 1000) as u64 });
    }
    fn set_position(&self, _track: ObjectPath<'_>, position: i64) {
        self.send(MediaCommand::Seek { position_ms: (position.max(0) / 1000) as u64 });
    }
    fn open_uri(&self, _uri: String) {}

    #[zbus(property)]
    fn playback_status(&self) -> String {
        if self.player.playing { "Playing" } else { "Paused" }.into()
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let p = &self.player;
        let mut m: HashMap<String, OwnedValue> = HashMap::new();
        let track = format!("/app/linksy/track/{}", p.id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>());
        if let Ok(path) = ObjectPath::try_from(track) {
            m.insert("mpris:trackid".into(), owned(Value::from(path)));
        }
        m.insert("xesam:title".into(), owned(Value::from(p.title.clone())));
        m.insert("xesam:artist".into(), owned(Value::from(vec![p.artist.clone()])));
        m.insert("xesam:album".into(), owned(Value::from(p.album.clone())));
        if let Some(l) = p.length_ms {
            m.insert("mpris:length".into(), owned(Value::from(l as i64 * 1000)));
        }
        if let Some(a) = &self.art {
            m.insert("mpris:artUrl".into(), owned(Value::from(a.clone())));
        }
        m
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        self.player.position_ms.unwrap_or(0) as i64 * 1000
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.player.can_next
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.player.can_previous
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.player.can_seek
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

fn owned(v: Value<'_>) -> OwnedValue {
    v.try_to_owned().expect("plain values convert")
}
