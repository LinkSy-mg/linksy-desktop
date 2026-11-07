//! Media controls, both ways, over MPRIS (the D-Bus interface every Linux player speaks):
//! - this computer's players go to devices that ask (`media_watch`), and their commands
//!   (play, pause, next…) come back to the player;
//! - a phone's players appear here as MPRIS players of their own, so GNOME's media
//!   controls and the keyboard's media keys work on them.
//!
//! And when a phone has a call, what plays here pauses, and resumes when the call ends.
//!
//! Nothing runs on D-Bus while no device is watching and no phone is playing.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use base64::Engine;
use futures_util::StreamExt;
use linksy_core::{CallState, DeviceInfo, Event, Fingerprint, MediaCommand, MediaPlayer, MediaState, Node};
use tokio::sync::{broadcast::error::RecvError, mpsc};
use tracing::{debug, warn};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, proxy::CacheProperties};

mod clock;
use clock::{Clock, Track};

const MPRIS: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// The names Linksy publishes phones' players under; never sent back to a phone.
const OURS: &str = "org.mpris.MediaPlayer2.Linksy.";
/// Largest cover art passed along (the protocol allows 64 KiB).
const MAX_ART: usize = 48 * 1024;
/// A file bigger than this is not read as a cover at all.
const MAX_ART_FILE: usize = 16 * 1024 * 1024;
/// A bigger picture is scaled down to fit this many pixels a side, as JPEG.
const ART_SIDES: [u32; 3] = [400, 300, 200];

pub async fn run(node: Node, art_dir: std::path::PathBuf) {
    let mut events = node.subscribe();
    let mut watchers: HashSet<Fingerprint> = HashSet::new();
    let mut local: Option<Local> = None;
    let mut phones = Phones { exported: HashMap::new(), art_dir };
    // Players paused for each phone's call, to resume (only those) when it ends.
    let mut paused_for_call: HashMap<Fingerprint, Vec<String>> = HashMap::new();
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
                Ok(Event::CallState { from, state: CallState::Ringing | CallState::Active, .. }) if !paused_for_call.contains_key(&from.id) => {
                    let paused = match Local::start().await {
                        Ok(l) => l.pause_all().await,
                        Err(e) => {
                            warn!("can't pause this computer's players: {e}");
                            Vec::new()
                        }
                    };
                    paused_for_call.insert(from.id, paused);
                }
                Ok(Event::CallState { from, state: CallState::Idle, .. }) => {
                    if let Some(paused) = paused_for_call.remove(&from.id)
                        && !paused.is_empty()
                        && let Ok(l) = Local::start().await
                    {
                        for name in paused {
                            l.command(&name[MPRIS.len()..], MediaCommand::Play).await;
                        }
                    }
                }
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

/// A deck's media button: for the player that's playing, else the first paused one, else
/// the first there is (playerctl's choice).
pub async fn press(command: MediaCommand) {
    let local = match Local::start().await {
        Ok(l) => l,
        Err(e) => return warn!("can't reach this computer's players: {e}"),
    };
    let mut chosen: Option<(u8, String)> = None;
    for name in local.names().await {
        let Ok(p) = local.proxy(&name, PLAYER).await else { continue };
        let rank = match p.get_property::<String>("PlaybackStatus").await.as_deref() {
            Ok("Playing") => 0,
            Ok("Paused") => 1,
            _ => 2,
        };
        if chosen.as_ref().is_none_or(|(best, _)| rank < *best) {
            chosen = Some((rank, name));
        }
        if rank == 0 {
            break;
        }
    }
    match chosen {
        Some((_, name)) => local.command(&name[MPRIS.len()..], command).await,
        None => debug!("no player here for {command:?}"),
    }
}

/// This computer's players, read over D-Bus while some device watches them.
struct Local {
    bus: Connection,
    changed: mpsc::Receiver<()>,
    _listen: tokio::task::JoinHandle<()>,
    /// Cover art as sent, by file and when it last changed: players send their state
    /// again and again with the same picture.
    art: std::sync::Mutex<HashMap<String, (std::time::SystemTime, Option<String>)>>,
    /// Where each player is by our own count, for those whose Position can't be believed
    /// (by player name)
    clocks: std::sync::Mutex<HashMap<String, Clock>>,
    /// Where players said they went (`Seeked`) and when, by their unique bus name, until the
    /// next look
    seeks: std::sync::Arc<std::sync::Mutex<HashMap<String, (u64, Instant)>>>,
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
        let seeks: std::sync::Arc<std::sync::Mutex<HashMap<String, (u64, Instant)>>> = Default::default();
        let heard = seeks.clone();
        let listen = tokio::spawn(async move {
            loop {
                let next = tokio::select! { m = props.next() => m, m = owners.next() => m };
                if let Some(Ok(message)) = &next {
                    note_seek(&heard, message);
                }
                if next.is_none() || tx.try_send(()).is_err() && tx.is_closed() {
                    break;
                }
            }
        });
        Ok(Self { bus, changed, _listen: listen, art: Default::default(), clocks: Default::default(), seeks })
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
        let names = self.names().await;
        for name in &names {
            match self.player(name).await {
                Ok(p) => players.push(p),
                Err(e) => debug!("{name}: {e}"),
            }
        }
        self.clocks.lock().unwrap().retain(|name, _| names.contains(name));
        let seq = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        MediaState { seq, players: drop_shadowed(players) }
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
        let playing = status == "Playing";
        let length_ms = length.filter(|l| *l > 0).map(|l| l as u64 / 1000);
        let position_ms = self.position(name, Track { title: text("xesam:title"), length_ms }, playing, position.filter(|p| *p >= 0).map(|p| p as u64 / 1000)).await;
        let flag = |v: zbus::Result<bool>| v.unwrap_or(false);
        Ok(MediaPlayer {
            id: name[MPRIS.len()..].to_string(),
            app,
            title: text("xesam:title"),
            artist,
            album: text("xesam:album"),
            playing,
            position_ms,
            length_ms,
            can_next: flag(player.get_property("CanGoNext").await),
            can_previous: flag(player.get_property("CanGoPrevious").await),
            can_seek: flag(player.get_property("CanSeek").await),
            art: self.art(&text("mpris:artUrl")),
        })
    }

    /// Pause every player that's playing; their names, to resume.
    async fn pause_all(&self) -> Vec<String> {
        let mut paused = Vec::new();
        for name in self.names().await {
            let Ok(p) = self.proxy(&name, PLAYER).await else { continue };
            if p.get_property::<String>("PlaybackStatus").await.is_ok_and(|s| s == "Playing") && p.call_method("Pause", &()).await.is_ok() {
                paused.push(name);
            }
        }
        paused
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
            MediaCommand::Seek { position_ms } => self.seek(&name, &proxy, position_ms).await,
        };
        if let Err(e) = result {
            debug!("{player}: {e}");
        }
    }
}

impl Local {
    /// Where the player is: what it says, unless that has stayed at 0 while it plays.
    async fn position(&self, name: &str, track: Track, playing: bool, raw_ms: Option<u64>) -> Option<u64> {
        let seeked = if self.seeks.lock().unwrap().is_empty() {
            None
        } else {
            let owner = async {
                let dbus = zbus::fdo::DBusProxy::new(&self.bus).await.ok()?;
                dbus.get_name_owner(zbus::names::BusName::try_from(name).ok()?).await.ok().map(|o| o.to_string())
            };
            let owner = owner.await;
            owner.and_then(|o| self.seeks.lock().unwrap().remove(&o))
        };
        let now = Instant::now();
        let mut clocks = self.clocks.lock().unwrap();
        clocks.entry(name.to_string()).or_insert_with(|| Clock::new(track.clone(), playing, now)).look(track, playing, raw_ms, seeked, now)
    }

    /// Go to a place in the track. Exactly there (`SetPosition`) when the player gives the
    /// track's id; else by the difference from where it is, which is only as right as that is.
    async fn seek(&self, name: &str, proxy: &zbus::Proxy<'_>, ms: u64) -> zbus::Result<()> {
        let target = ms as i64 * 1000;
        let meta: HashMap<String, OwnedValue> = proxy.get_property("Metadata").await.unwrap_or_default();
        let mut done = false;
        if let Some(track) = meta.get("mpris:trackid").and_then(track_path) {
            match proxy.call_method("SetPosition", &(track, target)).await {
                Ok(_) => done = true,
                Err(e) => debug!("{name}: SetPosition: {e}"),
            }
        }
        if !done {
            let raw: i64 = proxy.get_property("Position").await.unwrap_or(0);
            let counted = self.clocks.lock().unwrap().get(name).filter(|c| c.stuck()).map(|c| c.at(Instant::now()) as i64 * 1000);
            proxy.call_method("Seek", &(target - counted.unwrap_or(raw))).await?;
        }
        if let Some(clock) = self.clocks.lock().unwrap().get_mut(name) {
            clock.seeked(ms, Instant::now());
        }
        Ok(())
    }

    /// Cover art from a local file, small enough to pass along.
    fn art(&self, url: &str) -> Option<String> {
        let path = percent_decode(url.strip_prefix("file://")?);
        // The Flatpak can't see the player's folder, so the computer reads it, one process a
        // time: no asking after the file's time on every call, it is read again every 10 s.
        let modified = if linksy_host::flatpak() {
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() / 10 * 10)
        } else {
            std::fs::metadata(&path).and_then(|m| m.modified()).ok()?
        };
        let mut known = self.art.lock().unwrap();
        if let Some((when, art)) = known.get(&path)
            && *when == modified
        {
            return art.clone();
        }
        let art = linksy_host::read_bytes(std::path::Path::new(&path), MAX_ART_FILE).and_then(small_art).map(|a| base64::engine::general_purpose::STANDARD.encode(a));
        // Players come and go: only the pictures of late
        if known.len() > 16 {
            known.clear();
        }
        known.insert(path, (modified, art.clone()));
        art
    }
}

/// A player said it moved (`Seeked`, in microseconds): where, by who.
fn note_seek(heard: &std::sync::Mutex<HashMap<String, (u64, Instant)>>, message: &zbus::Message) {
    let header = message.header();
    if header.member().is_none_or(|m| m.as_str() != "Seeked") {
        return;
    }
    let Some(sender) = header.sender() else { return };
    let Ok(us) = message.body().deserialize::<i64>() else { return };
    let mut heard = heard.lock().unwrap();
    if heard.len() > 16 {
        heard.clear();
    }
    heard.insert(sender.to_string(), (us.max(0) as u64 / 1000, Instant::now()));
}

/// A track's id, which `SetPosition` wants: an object path (some players send a string),
/// and not the one that means there is no track.
fn track_path(value: &OwnedValue) -> Option<OwnedObjectPath> {
    let path = OwnedObjectPath::try_from(value.clone()).ok().or_else(|| OwnedObjectPath::try_from(String::try_from(value.clone()).ok()?).ok())?;
    (path.as_str() != "/org/mpris/MediaPlayer2/TrackList/NoTrack").then_some(path)
}

/// The picture as it is when it's small enough, else scaled down and saved as JPEG (a
/// browser's video thumbnail is often twice the size allowed).
fn small_art(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.len() <= MAX_ART {
        return Some(bytes);
    }
    let picture = image::load_from_memory(&bytes).ok()?;
    for side in ART_SIDES {
        let mut out = Vec::new();
        let rgb = picture.thumbnail(side, side).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80).encode_image(&rgb).ok()?;
        if out.len() <= MAX_ART {
            return Some(out);
        }
    }
    None
}

pub(crate) fn percent_decode(s: &str) -> String {
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

/// Plasma's browser integration reports a tab's media beside the browser's own MPRIS player,
/// so one track shows up twice. Where the integration has a track, the others playing the
/// same one are dropped; its better metadata stays.
fn drop_shadowed(players: Vec<MediaPlayer>) -> Vec<MediaPlayer> {
    const PLASMA: &str = "plasma-browser-integration";
    let plasma: Vec<(String, String)> = players.iter().filter(|p| p.id.starts_with(PLASMA) && !p.title.is_empty()).map(|p| (p.title.clone(), p.artist.clone())).collect();
    if plasma.is_empty() {
        return players;
    }
    players
        .into_iter()
        .filter(|p| p.id.starts_with(PLASMA) || !plasma.iter().any(|(title, artist)| *title == p.title && (artist.is_empty() || p.artist.is_empty() || *artist == p.artist)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: &str, title: &str, artist: &str) -> MediaPlayer {
        MediaPlayer {
            id: id.into(),
            app: id.into(),
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            playing: true,
            position_ms: None,
            length_ms: None,
            can_next: false,
            can_previous: false,
            can_seek: false,
            art: None,
        }
    }

    fn ids(players: &[MediaPlayer]) -> Vec<&str> {
        players.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn the_browsers_own_copy_of_a_track_gives_way_to_plasmas() {
        let kept = drop_shadowed(vec![player("firefox.instance_1_234", "Song", "Band"), player("plasma-browser-integration", "Song", "Band")]);
        assert_eq!(ids(&kept), ["plasma-browser-integration"]);
        // Plasma often has no artist where the browser does, or the other way round
        let kept = drop_shadowed(vec![player("firefox.instance_1_234", "Song", ""), player("plasma-browser-integration", "Song", "Band")]);
        assert_eq!(ids(&kept), ["plasma-browser-integration"]);
    }

    #[test]
    fn other_players_stay() {
        let kept = drop_shadowed(vec![player("spotify", "Other", "Band"), player("plasma-browser-integration", "Song", "Band"), player("vlc", "Song", "Someone Else")]);
        assert_eq!(ids(&kept), ["spotify", "plasma-browser-integration", "vlc"]);
    }

    #[test]
    fn nothing_changes_without_plasma_or_a_track() {
        let two = vec![player("mpv.instance1", "Song", "Band"), player("mpv.instance2", "Song", "Band")];
        assert_eq!(drop_shadowed(two.clone()), two);
        let idle = vec![player("firefox.instance_1_234", "Song", "Band"), player("plasma-browser-integration", "", "")];
        assert_eq!(drop_shadowed(idle.clone()), idle);
    }


    #[test]
    fn makes_a_big_cover_small() {
        // Noise doesn't compress: a PNG well over the limit
        let mut seed = 1u32;
        let picture = image::RgbImage::from_fn(640, 360, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            image::Rgb([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8])
        });
        let mut png = Vec::new();
        picture.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert!(png.len() > MAX_ART);
        let small = small_art(png).unwrap();
        assert!(small.len() <= MAX_ART);
        assert_eq!(image::guess_format(&small).unwrap(), image::ImageFormat::Jpeg);
        // Small ones go as they are
        assert_eq!(small_art(vec![1, 2, 3]), Some(vec![1, 2, 3]));
    }
}
