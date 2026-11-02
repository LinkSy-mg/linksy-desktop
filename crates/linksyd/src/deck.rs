//! Phones' decks (docs/protocol.md §4.19): this computer says what each phone may do and
//! does it. Every action is on or off per phone (media and sound on until the user says
//! otherwise, locking and the user's own commands off), kept in `deck.json` with the
//! commands and each phone's layouts (§4.20). Whatever else a phone runs is ignored.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use linksy_core::deck_layout::{self, MAX_BYTES};
use linksy_core::{DeckAction, DeckFace, DeckKind, DeckLayout, DeckLayouts, DeckState, DeckTheme, DeckTile, Event, Fingerprint, MediaCommand, Node, Uuid};
use linksy_i18n::t;
use linksy_ipc::{DeckCommand, DeckOption, DeckPrefs};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub static DECK: OnceLock<Deck> = OnceLock::new();

/// So a deck's list stays well within what a phone takes in one message.
const MAX_COMMANDS: usize = 100;
const MAX_NAME: usize = 100;
/// Longest one action may hold the next ones up (a player that doesn't answer).
const ACT_WAIT: Duration = Duration::from_secs(5);

const MEDIA: &str = "Media";
const SOUND: &str = "Sound";
const SYSTEM: &str = "System";
const COMMANDS: &str = "Commands";

/// What an action does here.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Media(MediaCommand),
    Volume(f64),
    Mute,
    MicMute,
    /// The next sound output becomes the default.
    NextOutput,
    Lock,
    Restart,
    Shutdown,
    Command { name: String, line: String },
}

/// What this computer can do for a deck besides media and locking, found out once.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Caps {
    /// A volume to set (wpctl or pactl).
    pub sound: bool,
    /// Restart and turn off (logind).
    pub power_off: bool,
}

/// This computer's state now, as far as the decks show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub level: Option<f64>,
    pub muted: Option<bool>,
    pub mic_muted: Option<bool>,
    /// The default output's name.
    pub output: Option<String>,
}

/// What runs the actions: this desktop, or a stand-in in tests.
pub trait Desktop: Send + Sync + 'static {
    fn probe(&self) -> impl Future<Output = Caps> + Send;
    fn run(&self, act: Act) -> impl Future<Output = ()> + Send;
    /// The state of what the actions `ids` show; the rest stays unknown.
    fn status(&self, ids: &HashSet<&str>) -> impl Future<Output = Status> + Send;
}

#[derive(Default)]
pub struct System {
    controls: crate::controls::Controls,
}

impl Desktop for System {
    async fn probe(&self) -> Caps {
        let found = self.controls.probe().await;
        Caps { sound: crate::volume::available(), power_off: found.power_off }
    }

    async fn run(&self, act: Act) {
        match act {
            Act::Media(command) => crate::media::press(command).await,
            Act::Volume(level) => crate::volume::set_level(level).await,
            Act::Mute => crate::volume::toggle_mute(false).await,
            Act::MicMute => crate::volume::toggle_mute(true).await,
            Act::NextOutput => crate::volume::next_output().await,
            Act::Lock => {
                crate::lock::lock_screen().await;
            }
            Act::Restart => self.controls.power_off(true).await,
            Act::Shutdown => self.controls.power_off(false).await,
            Act::Command { name, line } => spawn(&name, &line),
        }
    }

    async fn status(&self, ids: &HashSet<&str>) -> Status {
        let (output, mic) = (ids.contains("sound.volume") || ids.contains("sound.mute"), ids.contains("sound.mic_mute"));
        let (out, mic_muted, name) = tokio::join!(
            async { if output { crate::volume::output_state().await } else { None } },
            async { if mic { crate::volume::mic_muted().await } else { None } },
            async { if ids.contains("sound.output") { crate::volume::output_name().await } else { None } },
        );
        Status { level: out.map(|(level, _)| level.clamp(0.0, 1.0)), muted: out.map(|(_, muted)| muted), mic_muted, output: name }
    }
}

/// Start one of the user's commands on its own (from the Flatpak, on the computer): it reads
/// nothing, its output goes nowhere, and nothing waits for it.
fn spawn(name: &str, line: &str) {
    let mut command = tokio::process::Command::from(linksy_host::detached("sh", &[]));
    command.args(["-c", line]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    if let Some(home) = std::env::var_os("HOME") {
        command.current_dir(home);
    }
    match command.spawn() {
        Ok(mut child) => {
            let name = name.to_string();
            tokio::spawn(async move {
                if let Ok(status) = child.wait().await
                    && !status.success()
                {
                    debug!("deck command {name}: {status}");
                }
            });
        }
        Err(e) => warn!("could not run the deck command {name}: {e}"),
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    /// What the user turned on or off for each phone; the rest is as by default.
    phones: HashMap<Fingerprint, BTreeMap<String, bool>>,
    commands: Vec<DeckCommand>,
    /// Each phone's layouts as the user saved them.
    layouts: HashMap<Fingerprint, DeckLayouts>,
}

impl Saved {
    fn on(&self, phone: &Fingerprint, id: &str) -> bool {
        match self.phones.get(phone).and_then(|choices| choices.get(id)) {
            Some(on) => *on,
            None => id.starts_with("media.") || id.starts_with("sound."),
        }
    }
}

enum Job {
    /// A phone's deck shows: its list, then the state.
    Open(Fingerprint),
    Run(Fingerprint, Act),
    /// What a phone may do changed (`None`: any phone): its list again, if its deck shows.
    Actions(Option<Fingerprint>),
    /// A phone's layouts were saved: its list and layouts again, if its deck shows.
    Layouts(Fingerprint),
    /// The volume changed.
    Sound,
}

pub struct Deck<D: Desktop = System> {
    inner: Arc<Inner<D>>,
}

impl<D: Desktop> Clone for Deck<D> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

struct Inner<D> {
    path: PathBuf,
    saved: Mutex<Saved>,
    /// Phones whose deck shows, and the state each was sent last.
    open: Mutex<HashMap<Fingerprint, Option<Vec<DeckState>>>>,
    jobs: mpsc::UnboundedSender<Job>,
    queued: Mutex<Option<mpsc::UnboundedReceiver<Job>>>,
    desktop: D,
    /// Found out the first time a deck or the editor needs it.
    caps: tokio::sync::OnceCell<Caps>,
    /// The state as last read, for what a batch doesn't read again.
    status: Mutex<Status>,
    /// For the names in the questions actions ask.
    node: OnceLock<Node>,
}

impl<D: Desktop> Deck<D> {
    pub fn load(path: PathBuf, desktop: D) -> Self {
        let saved = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let (jobs, queued) = mpsc::unbounded_channel();
        let inner = Inner {
            path,
            saved: Mutex::new(saved),
            open: Mutex::default(),
            jobs,
            queued: Mutex::new(Some(queued)),
            desktop,
            caps: tokio::sync::OnceCell::new(),
            status: Mutex::default(),
            node: OnceLock::new(),
        };
        Self { inner: Arc::new(inner) }
    }

    /// Find out what this computer can do, once; until then only what needs nothing.
    pub async fn ready(&self) {
        self.inner.caps.get_or_init(|| self.inner.desktop.probe()).await;
    }

    fn caps(&self) -> Caps {
        self.inner.caps.get().copied().unwrap_or_default()
    }

    /// Everything this computer offers a deck. The questions keep `{computer}` and
    /// `{phone}`, for [`Deck::named`].
    fn offered(&self, saved: &Saved) -> Vec<DeckAction> {
        let caps = self.caps();
        let action = |id: &str, name: &str, kind, group: &str| DeckAction { id: id.into(), name: name.into(), kind, group: group.into(), confirm: None };
        let mut all = vec![
            action("media.play_pause", t!("Play or Pause"), DeckKind::Button, MEDIA),
            action("media.next", t!("Next Track"), DeckKind::Button, MEDIA),
            action("media.previous", t!("Previous Track"), DeckKind::Button, MEDIA),
        ];
        if caps.sound {
            all.extend([
                action("sound.volume", t!("Volume"), DeckKind::Slider, SOUND),
                action("sound.mute", t!("Mute"), DeckKind::Toggle, SOUND),
                action("sound.mic_mute", t!("Mute Microphone"), DeckKind::Toggle, SOUND),
                action("sound.output", t!("Sound Output"), DeckKind::Button, SOUND),
            ]);
        }
        all.push(action("system.lock", t!("Lock Screen"), DeckKind::Button, SYSTEM));
        let asking = |mut action: DeckAction, question: &str| {
            action.confirm = Some(question.into());
            action
        };
        if caps.power_off {
            all.push(asking(action("system.restart", t!("Restart"), DeckKind::Button, SYSTEM), t!("Restart {computer}?")));
            all.push(asking(action("system.shutdown", t!("Shut Down"), DeckKind::Button, SYSTEM), t!("Turn off {computer}?")));
        }
        all.extend(saved.commands.iter().map(|c| action(&format!("command.{}", c.id), &c.name, DeckKind::Button, COMMANDS)));
        all
    }

    /// What `phone`'s deck may do: its list.
    fn enabled(&self, phone: &Fingerprint) -> Vec<DeckAction> {
        let saved = self.inner.saved.lock().unwrap();
        let list = self.offered(&saved).into_iter().filter(|a| saved.on(phone, &a.id)).collect();
        drop(saved);
        self.named(phone, list)
    }

    pub fn prefs(&self, phone: &Fingerprint) -> DeckPrefs {
        let saved = self.inner.saved.lock().unwrap();
        let offered = self.offered(&saved);
        let on: Vec<bool> = offered.iter().map(|a| saved.on(phone, &a.id)).collect();
        let commands = saved.commands.clone();
        drop(saved);
        let actions = self.named(phone, offered).into_iter().zip(on).map(|(action, on)| DeckOption { action, on }).collect();
        DeckPrefs { actions, commands }
    }

    /// The questions with this computer's name and `phone`'s.
    fn named(&self, phone: &Fingerprint, actions: Vec<DeckAction>) -> Vec<DeckAction> {
        let Some(node) = self.inner.node.get() else { return actions };
        let phone = node.trusted().into_iter().find(|t| t.device.id == *phone).map(|t| t.device.name).unwrap_or_default();
        fill_in(actions, &node.info().name, &phone)
    }

    pub fn set_action(&self, phone: &Fingerprint, action: &str, on: bool) -> Result<(), String> {
        let mut saved = self.inner.saved.lock().unwrap();
        if !self.offered(&saved).iter().any(|a| a.id == action) {
            return Err(format!("this computer doesn't offer \"{action}\""));
        }
        let mut next = saved.clone();
        next.phones.entry(*phone).or_default().insert(action.into(), on);
        self.save(&next)?;
        *saved = next;
        drop(saved);
        let _ = self.inner.jobs.send(Job::Actions(Some(*phone)));
        Ok(())
    }

    /// Replace the user's commands; a command that isn't kept is off for every phone too.
    pub fn set_commands(&self, commands: Vec<DeckCommand>) -> Result<Vec<DeckCommand>, String> {
        if commands.len() > MAX_COMMANDS {
            return Err(format!("at most {MAX_COMMANDS} commands"));
        }
        let mut saved = self.inner.saved.lock().unwrap();
        let mut kept = HashSet::new();
        let mut list = Vec::with_capacity(commands.len());
        for c in commands {
            let (name, command) = (c.name.trim(), c.command.trim());
            if name.is_empty() || command.is_empty() {
                return Err("each command needs a name and a command line".into());
            }
            if name.chars().count() > MAX_NAME {
                return Err(format!("a command's name can be at most {MAX_NAME} characters long"));
            }
            let known = saved.commands.iter().any(|s| s.id == c.id) && kept.insert(c.id.clone());
            let id = if known { c.id } else { Uuid::new_v4().simple().to_string() };
            list.push(DeckCommand { id, name: name.into(), command: command.into() });
        }
        let ids: HashSet<String> = list.iter().map(|c| format!("command.{}", c.id)).collect();
        let mut next = Saved { phones: saved.phones.clone(), commands: list, layouts: saved.layouts.clone() };
        for choices in next.phones.values_mut() {
            choices.retain(|id, _| !id.starts_with("command.") || ids.contains(id));
        }
        self.save(&next)?;
        let list = next.commands.clone();
        *saved = next;
        drop(saved);
        let _ = self.inner.jobs.send(Job::Actions(None));
        Ok(list)
    }

    /// `phone`'s layouts as saved; until then a first one, from what this computer offers.
    pub fn layouts(&self, phone: &Fingerprint) -> DeckLayouts {
        let saved = self.inner.saved.lock().unwrap();
        match saved.layouts.get(phone) {
            Some(layouts) => layouts.clone(),
            None => self.first_layouts(&saved),
        }
    }

    /// Save `phone`'s layouts, one version on from the last. Placing a tile allows its
    /// action: each one a tile runs is turned on for that phone.
    pub fn set_layouts(&self, phone: &Fingerprint, mut layouts: DeckLayouts) -> Result<DeckLayouts, String> {
        if !layouts.layouts.iter().any(|l| l.id == layouts.active) {
            layouts.active = layouts.layouts.first().map(|l| l.id.clone()).unwrap_or_default();
        }
        let mut saved = self.inner.saved.lock().unwrap();
        let offered: HashSet<String> = self.offered(&saved).into_iter().map(|a| a.id).collect();
        layouts.check(|a| offered.contains(a))?;
        layouts.version = saved.layouts.get(phone).map_or(0, |l| l.version) + 1;
        let size = serde_json::to_vec(&layouts).map_err(|e| e.to_string())?.len();
        if size > MAX_BYTES {
            return Err(format!("these layouts are too big to send to the phone ({} KiB, at most {} KiB)", size.div_ceil(1024), MAX_BYTES / 1024));
        }
        let mut next = saved.clone();
        let choices = next.phones.entry(*phone).or_default();
        for action in runs(&layouts) {
            choices.insert(action.into(), true);
        }
        next.layouts.insert(*phone, layouts.clone());
        self.save(&next)?;
        *saved = next;
        drop(saved);
        let _ = self.inner.jobs.send(Job::Layouts(*phone));
        Ok(layouts)
    }

    /// The layout a deck has until the user saves one: the clock, the player, the volume,
    /// and buttons for the rest, wherever there's room.
    fn first_layouts(&self, saved: &Saved) -> DeckLayouts {
        let offered: HashSet<String> = self.offered(saved).into_iter().map(|a| a.id).collect();
        let buttons: Vec<&str> = ["media.play_pause", "media.next", "sound.mute", "sound.mic_mute"].into_iter().filter(|a| offered.contains(*a)).collect();
        let volume = offered.contains("sound.volume");
        let tile = |kind: &str, (x, y): (u8, u8), (w, h): (u8, u8), action: Option<&str>| DeckTile {
            id: action.unwrap_or(kind).into(),
            kind: kind.into(),
            x,
            y,
            w,
            h,
            action: action.map(Into::into),
            ..Default::default()
        };
        let face = |mut face: DeckFace, clock: (u8, u8), music: (u8, u8), sound: (u8, u8)| {
            let page = &mut face.pages[0];
            page.push(tile("clock", clock, (2, 1), None));
            page.push(tile("music", music, (4, 2), None));
            if volume {
                page.push(tile("volume", sound, (4, 1), Some("sound.volume")));
            }
            for action in &buttons {
                let (page, (x, y)) = match (0..face.pages.len()).find_map(|p| Some((p, face.free_spot(p, 1, 1)?))) {
                    Some(spot) => spot,
                    None => {
                        face.pages.push(Vec::new());
                        (face.pages.len() - 1, (0, 0))
                    }
                };
                face.pages[page].push(tile("button", (x, y), (1, 1), Some(action)));
            }
            face
        };
        let layout = DeckLayout {
            id: "standard".into(),
            name: t!("Standard").into(),
            theme: DeckTheme::default(),
            portrait: face(DeckFace::portrait(), (0, 0), (0, 1), (0, 3)),
            landscape: face(DeckFace::landscape(), (4, 0), (0, 0), (0, 2)),
        };
        DeckLayouts { version: 0, active: layout.id.clone(), layouts: vec![layout] }
    }

    fn save(&self, saved: &Saved) -> Result<(), String> {
        let tmp = self.inner.path.with_extension("json.tmp");
        serde_json::to_vec_pretty(saved)
            .map_err(std::io::Error::other)
            .and_then(|b| linksy_core::identity::write_private(&tmp, &b))
            .and_then(|_| std::fs::rename(&tmp, &self.inner.path))
            .map_err(|e| format!("could not save the deck's settings: {e}"))
    }

    /// What `id` does when `phone` runs it, if that phone may.
    fn act(&self, phone: &Fingerprint, id: &str, value: Option<f64>) -> Option<Act> {
        let saved = self.inner.saved.lock().unwrap();
        if !saved.on(phone, id) {
            return None;
        }
        let caps = self.caps();
        Some(match id {
            "media.play_pause" => Act::Media(MediaCommand::PlayPause),
            "media.next" => Act::Media(MediaCommand::Next),
            "media.previous" => Act::Media(MediaCommand::Previous),
            "sound.volume" if caps.sound => Act::Volume(value.filter(|v| v.is_finite())?.clamp(0.0, 1.0)),
            "sound.mute" if caps.sound => Act::Mute,
            "sound.mic_mute" if caps.sound => Act::MicMute,
            "sound.output" if caps.sound => Act::NextOutput,
            "system.lock" => Act::Lock,
            "system.restart" if caps.power_off => Act::Restart,
            "system.shutdown" if caps.power_off => Act::Shutdown,
            _ => {
                let id = id.strip_prefix("command.")?;
                let c = saved.commands.iter().find(|c| c.id == id)?;
                Act::Command { name: c.name.clone(), line: c.command.clone() }
            }
        })
    }

    pub async fn run(self, node: Node) {
        let Some(jobs) = self.inner.queued.lock().unwrap().take() else { return };
        let _ = self.inner.node.set(node.clone());
        tokio::spawn(self.clone().work(node.clone(), jobs));
        let mut events = node.subscribe();
        let paired = |id: &Fingerprint| node.trusted().iter().any(|t| t.device.id == *id);
        loop {
            tokio::select! {
                event = events.recv() => match event {
                    Ok(Event::DeckOpen { from }) if paired(&from.id) => {
                        self.inner.open.lock().unwrap().insert(from.id, None);
                        let _ = self.inner.jobs.send(Job::Open(from.id));
                    }
                    Ok(Event::DeckRun { from, action, value }) if paired(&from.id) => {
                        self.ready().await;
                        match self.act(&from.id, &action, value) {
                            Some(act) => {
                                let _ = self.inner.jobs.send(Job::Run(from.id, act));
                            }
                            None => debug!("{} may not run {action} here", from.name),
                        }
                    }
                    Ok(Event::DeckOpen { from } | Event::DeckRun { from, .. }) => debug!("deck of {}, which isn't paired", from.name),
                    Ok(Event::DeckClose { from }) => {
                        self.inner.open.lock().unwrap().remove(&from.id);
                    }
                    Ok(Event::Disconnected { id }) => {
                        self.inner.open.lock().unwrap().remove(&id);
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
                () = crate::volume::CHANGED.notified() => {
                    if !self.inner.open.lock().unwrap().is_empty() {
                        let _ = self.inner.jobs.send(Job::Sound);
                    }
                }
            }
        }
    }

    /// Jobs one batch at a time (whatever came in meanwhile), so a slider dragged on the
    /// phone sets the volume once per batch, not once per step.
    async fn work(self, node: Node, mut jobs: mpsc::UnboundedReceiver<Job>) {
        while let Some(first) = jobs.recv().await {
            let mut batch = vec![first];
            while let Ok(job) = jobs.try_recv() {
                batch.push(job);
            }
            self.ready().await;
            // Only the volume moved (a slider, the keyboard): read the sound alone again
            let sound_only = batch.iter().all(|j| matches!(j, Job::Sound | Job::Run(_, Act::Volume(_))));
            let mut told = HashSet::new();
            let mut acts = Vec::new();
            for job in batch {
                match job {
                    Job::Run(phone, act) => {
                        told.insert(phone);
                        acts.push(act);
                    }
                    Job::Open(phone) | Job::Layouts(phone) => {
                        told.insert(phone);
                        self.send_actions(&node, phone).await;
                        self.send_layouts(&node, phone).await;
                    }
                    Job::Actions(only) => {
                        let open: Vec<Fingerprint> = self.inner.open.lock().unwrap().keys().copied().filter(|p| only.is_none_or(|o| o == *p)).collect();
                        for phone in open {
                            told.insert(phone);
                            self.send_actions(&node, phone).await;
                        }
                    }
                    Job::Sound => {}
                }
            }
            self.perform(acts).await;
            self.send_states(&node, &told, sound_only).await;
        }
    }

    /// Run actions in order, a slider's newest value only.
    async fn perform(&self, acts: Vec<Act>) {
        let last_volume = acts.iter().rposition(|a| matches!(a, Act::Volume(_)));
        for (i, act) in acts.into_iter().enumerate() {
            if matches!(act, Act::Volume(_)) && Some(i) != last_volume {
                continue;
            }
            let what = format!("{act:?}");
            if tokio::time::timeout(ACT_WAIT, self.inner.desktop.run(act)).await.is_err() {
                warn!("deck: {what} took too long");
            }
        }
    }

    /// Whether `phone`'s deck still shows and it's connected; forgotten otherwise, so a
    /// message to it never dials it.
    fn reachable(&self, node: &Node, phone: &Fingerprint) -> bool {
        let mut open = self.inner.open.lock().unwrap();
        if open.contains_key(phone) && node.link(phone).is_none() {
            open.remove(phone);
        }
        open.contains_key(phone)
    }

    async fn send_actions(&self, node: &Node, phone: Fingerprint) {
        if self.reachable(node, &phone)
            && let Err(e) = node.deck_actions(&phone, self.enabled(&phone)).await
        {
            debug!("deck actions: {e}");
            self.inner.open.lock().unwrap().remove(&phone);
        }
    }

    /// Its layouts, to a phone that takes them.
    async fn send_layouts(&self, node: &Node, phone: Fingerprint) {
        if !self.reachable(node, &phone) {
            return;
        }
        match node.deck_layout(&phone, &self.layouts(&phone)).await {
            Ok(()) | Err(linksy_core::Error::Unsupported) => {}
            Err(e) => {
                debug!("deck layouts: {e}");
                self.inner.open.lock().unwrap().remove(&phone);
            }
        }
    }

    /// The state to the decks `told` something, and to the other open ones if it changed.
    async fn send_states(&self, node: &Node, told: &HashSet<Fingerprint>, sound_only: bool) {
        let phones: Vec<Fingerprint> = self.inner.open.lock().unwrap().keys().copied().collect();
        let lists: Vec<(Fingerprint, Vec<DeckAction>)> = phones.into_iter().map(|p| (p, self.enabled(&p))).collect();
        let mut shown: HashSet<&str> = lists.iter().flat_map(|(_, actions)| actions.iter().map(|a| a.id.as_str())).collect();
        if sound_only {
            shown.retain(|id| ["sound.volume", "sound.mute"].contains(id));
        }
        let fresh = if shown.is_empty() { Status::default() } else { self.inner.desktop.status(&shown).await };
        let status = {
            let mut last = self.inner.status.lock().unwrap();
            if sound_only {
                (last.level, last.muted) = (fresh.level, fresh.muted);
            } else {
                *last = fresh;
            }
            last.clone()
        };
        for (phone, actions) in lists {
            let now = states(&actions, &status);
            {
                let mut open = self.inner.open.lock().unwrap();
                let Some(last) = open.get_mut(&phone) else { continue };
                if !told.contains(&phone) && last.as_ref() == Some(&now) {
                    continue;
                }
                *last = Some(now.clone());
            }
            if !self.reachable(node, &phone) {
                continue;
            }
            if let Err(e) = node.deck_state(&phone, now).await {
                debug!("deck state: {e}");
                self.inner.open.lock().unwrap().remove(&phone);
            }
        }
    }
}

fn fill_in(mut actions: Vec<DeckAction>, computer: &str, phone: &str) -> Vec<DeckAction> {
    for question in actions.iter_mut().filter_map(|a| a.confirm.as_mut()) {
        *question = question.replace("{computer}", computer).replace("{phone}", phone);
    }
    actions
}

/// The actions `layouts`' tiles run: only kinds that run one (a clock's action is ignored).
fn runs(layouts: &DeckLayouts) -> impl Iterator<Item = &str> {
    layouts
        .layouts
        .iter()
        .flat_map(|l| [&l.portrait, &l.landscape])
        .flat_map(|face| face.pages.iter().flatten())
        .filter(|t| deck_layout::kind(&t.kind).is_some_and(|k| k.runs))
        .filter_map(|t| t.action.as_deref())
}

/// The tiles' state for these actions: the ones that have one.
fn states(actions: &[DeckAction], now: &Status) -> Vec<DeckState> {
    actions
        .iter()
        .filter_map(|a| {
            let state = |on, value| Some(DeckState { id: a.id.clone(), on, value, text: None });
            match a.id.as_str() {
                "sound.volume" => state(None, Some(now.level?)),
                "sound.mute" => state(Some(now.muted?), None),
                "sound.mic_mute" => state(Some(now.mic_muted?), None),
                "sound.output" => Some(DeckState { id: a.id.clone(), on: None, value: None, text: Some(now.output.clone()?) }),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A computer that can do everything but what it's `without`, and only notes what it
    /// would have done.
    #[derive(Default)]
    struct Fake {
        without: &'static [&'static str],
        ran: Mutex<Vec<Act>>,
    }

    impl Fake {
        fn caps(&self) -> Caps {
            let has = |what| !self.without.contains(&what);
            Caps { sound: has("sound"), power_off: has("power_off") }
        }
    }

    impl Desktop for Fake {
        async fn probe(&self) -> Caps {
            self.caps()
        }

        async fn run(&self, act: Act) {
            self.ran.lock().unwrap().push(act);
        }

        async fn status(&self, ids: &HashSet<&str>) -> Status {
            let output = ids.contains("sound.volume") || ids.contains("sound.mute");
            let has = |id: &str| ids.contains(id);
            Status {
                level: output.then_some(0.42),
                muted: output.then_some(false),
                mic_muted: has("sound.mic_mute").then_some(true),
                output: has("sound.output").then(|| "Speakers".into()),
            }
        }
    }

    fn phone(n: u8) -> Fingerprint {
        Fingerprint([n; 32])
    }

    /// A deck that already found out what `fake` can do.
    fn ready(path: PathBuf, fake: Fake) -> Deck<Fake> {
        let caps = fake.caps();
        let d = Deck::load(path, fake);
        d.inner.caps.set(caps).unwrap();
        d
    }

    fn deck(dir: &tempfile::TempDir) -> Deck<Fake> {
        ready(dir.path().join("deck.json"), Fake::default())
    }

    fn command(id: &str, name: &str, line: &str) -> DeckCommand {
        DeckCommand { id: id.into(), name: name.into(), command: line.into() }
    }

    fn on(deck: &Deck<Fake>, p: &Fingerprint) -> Vec<String> {
        deck.prefs(p).actions.into_iter().filter(|a| a.on).map(|a| a.action.id).collect()
    }

    /// Everything the fake computer offers, and what's on for a phone until changed.
    const OFFERED: &[&str] = &["media.play_pause", "media.next", "media.previous", "sound.volume", "sound.mute", "sound.mic_mute", "sound.output", "system.lock", "system.restart", "system.shutdown"];
    const ON: &[&str] = &["media.play_pause", "media.next", "media.previous", "sound.volume", "sound.mute", "sound.mic_mute", "sound.output"];

    #[test]
    fn media_and_sound_on_until_changed_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        let all = d.prefs(&phone(1)).actions;
        assert_eq!(all.iter().map(|a| a.action.id.as_str()).collect::<Vec<_>>(), OFFERED);
        assert_eq!(on(&d, &phone(1)), ON);
        assert_eq!(all.iter().find(|a| a.action.id == "sound.volume").unwrap().action.kind, DeckKind::Slider);

        d.set_action(&phone(1), "system.lock", true).unwrap();
        d.set_action(&phone(1), "media.next", false).unwrap();
        let again = deck(&dir);
        let want: Vec<&str> = OFFERED.iter().copied().filter(|id| (ON.contains(id) && *id != "media.next") || *id == "system.lock").collect();
        assert_eq!(on(&again, &phone(1)), want);
        // Another phone still has the defaults
        assert_eq!(on(&again, &phone(2)), ON);
        assert_eq!(again.enabled(&phone(2)).len(), ON.len());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.path().join("deck.json")).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn only_what_is_on_runs() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        let p = phone(1);
        assert_eq!(d.act(&p, "media.play_pause", None), Some(Act::Media(MediaCommand::PlayPause)));
        assert_eq!(d.act(&p, "sound.mute", Some(0.3)), Some(Act::Mute));
        assert_eq!(d.act(&p, "system.lock", None), None);
        for unknown in ["media.stop", "sound.bass", "command.", "command.nope", "", "system.screen_off"] {
            assert_eq!(d.act(&p, unknown, None), None, "{unknown}");
        }
        assert!(d.set_action(&p, "media.stop", true).is_err());
        d.set_action(&p, "media.play_pause", false).unwrap();
        assert_eq!(d.act(&p, "media.play_pause", None), None);
        d.set_action(&p, "system.lock", true).unwrap();
        assert_eq!(d.act(&p, "system.lock", None), Some(Act::Lock));
        assert_eq!(d.act(&phone(2), "system.lock", None), None);

        // Without wpctl or pactl, no sound at all
        let quiet = ready(dir.path().join("quiet.json"), Fake { without: &["sound"], ..Default::default() });
        assert_eq!(quiet.act(&p, "sound.volume", Some(0.5)), None);
        assert!(quiet.prefs(&p).actions.iter().all(|a| a.action.group != SOUND));
        assert!(quiet.set_action(&p, "sound.mute", true).is_err());
    }

    #[test]
    fn commands_get_ids_and_leave_every_phone_when_removed() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        let list = d.set_commands(vec![command("", " Backup ", "rsync -a ~/Documents /backup"), command("made-up", "Backup", "echo two")]).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Backup");
        assert!(!list[0].id.is_empty() && list[0].id != list[1].id && list[1].id != "made-up");
        let (first, second) = (format!("command.{}", list[0].id), format!("command.{}", list[1].id));
        // Off until turned on
        assert_eq!(d.act(&phone(1), &first, None), None);
        for p in [phone(1), phone(2)] {
            d.set_action(&p, &first, true).unwrap();
            d.set_action(&p, &second, true).unwrap();
        }
        assert_eq!(d.act(&phone(1), &first, None), Some(Act::Command { name: "Backup".into(), line: "rsync -a ~/Documents /backup".into() }));
        let action = d.enabled(&phone(1)).into_iter().find(|a| a.id == first).unwrap();
        assert_eq!((action.kind, action.group.as_str()), (DeckKind::Button, COMMANDS));

        // Refused: nothing changes
        for bad in [command("", "  ", "true"), command("", "Empty", " \n ")] {
            assert!(d.set_commands(vec![list[0].clone(), bad]).is_err());
        }
        assert!(d.set_commands(vec![command("", &"x".repeat(MAX_NAME + 1), "true")]).is_err());
        assert_eq!(d.prefs(&phone(1)).commands, list);

        // The second one goes, the first is kept with its id (even listed twice), and renamed
        let kept = d.set_commands(vec![command(&list[0].id, "Back Up", "rsync -a ~/Documents /backup"), list[0].clone()]).unwrap();
        assert_eq!(kept[0].id, list[0].id);
        assert_ne!(kept[1].id, list[0].id);
        let again = deck(&dir);
        for p in [phone(1), phone(2)] {
            assert_eq!(again.act(&p, &second, None), None);
            assert!(again.inner.saved.lock().unwrap().phones[&p].keys().all(|id| *id != second));
            assert_eq!(again.act(&p, &first, None), Some(Act::Command { name: "Back Up".into(), line: "rsync -a ~/Documents /backup".into() }));
        }
        assert!(d.set_commands(vec![command("", "Many", "true"); MAX_COMMANDS + 1]).is_err());
        assert!(d.set_commands(Vec::new()).unwrap().is_empty());
        assert_eq!(d.act(&phone(1), &first, None), None);
    }

    #[tokio::test]
    async fn sliders_need_a_value_and_only_the_last_one_counts() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        let p = phone(1);
        assert_eq!(d.act(&p, "sound.volume", None), None);
        assert_eq!(d.act(&p, "sound.volume", Some(f64::NAN)), None);
        assert_eq!(d.act(&p, "sound.volume", Some(1.7)), Some(Act::Volume(1.0)));
        assert_eq!(d.act(&p, "sound.volume", Some(0.6)), Some(Act::Volume(0.6)));

        d.perform(vec![Act::Volume(0.2), Act::Mute, Act::Volume(0.5), Act::Media(MediaCommand::Next), Act::Volume(0.7)]).await;
        assert_eq!(*d.inner.desktop.ran.lock().unwrap(), [Act::Mute, Act::Media(MediaCommand::Next), Act::Volume(0.7)]);
    }

    #[test]
    fn state_for_the_tiles_that_have_one() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        let sound = Status { level: Some(0.42), muted: Some(false), ..Default::default() };
        let now = states(&d.enabled(&phone(1)), &sound);
        assert_eq!(now, [DeckState { id: "sound.volume".into(), on: None, value: Some(0.42), text: None }, DeckState { id: "sound.mute".into(), on: Some(false), value: None, text: None }]);
        d.set_action(&phone(1), "sound.volume", false).unwrap();
        assert_eq!(states(&d.enabled(&phone(1)), &sound).len(), 1);
    }

    #[test]
    fn restarting_and_turning_off_ask_first_and_are_off_until_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        let p = phone(1);
        assert_eq!((d.act(&p, "system.restart", None), d.act(&p, "system.shutdown", None)), (None, None));
        d.set_action(&p, "system.restart", true).unwrap();
        d.set_action(&p, "system.shutdown", true).unwrap();
        assert_eq!((d.act(&p, "system.restart", None), d.act(&p, "system.shutdown", None)), (Some(Act::Restart), Some(Act::Shutdown)));
        let asked = fill_in(d.enabled(&p), "Office PC", "Pixel");
        let question = |id: &str| asked.iter().find(|a| a.id == id).and_then(|a| a.confirm.clone());
        assert_eq!((question("system.restart").unwrap().as_str(), question("system.shutdown").unwrap().as_str()), ("Restart Office PC?", "Turn off Office PC?"));
        assert_eq!(question("system.lock"), None);

        let elsewhere = ready(dir.path().join("other.json"), Fake { without: &["power_off"], ..Default::default() });
        assert!(elsewhere.set_action(&p, "system.restart", true).is_err(), "no logind: not offered");
    }

    #[tokio::test]
    async fn the_output_switches_and_shows_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        assert_eq!(d.act(&phone(1), "sound.output", None), Some(Act::NextOutput));
        let now = d.inner.desktop.status(&HashSet::from(["sound.output"])).await;
        assert_eq!(states(&d.enabled(&phone(1)), &now), [DeckState { id: "sound.output".into(), on: None, value: None, text: Some("Speakers".into()) }]);
    }

    #[tokio::test]
    async fn found_out_once_and_only_what_is_there_is_offered() {
        let dir = tempfile::tempdir().unwrap();
        let d = Deck::load(dir.path().join("deck.json"), Fake { without: &["sound"], ..Default::default() });
        assert_eq!(d.prefs(&phone(1)).actions.len(), 4, "media and locking need nothing");
        d.ready().await;
        d.ready().await;
        assert_eq!(d.caps(), Caps { sound: false, ..Fake::default().caps() });
        assert!(d.prefs(&phone(1)).actions.iter().all(|a| !a.action.id.starts_with("sound.")));
    }

    fn tile(id: &str, kind: &str, (x, y): (u8, u8), (w, h): (u8, u8), action: Option<&str>) -> DeckTile {
        DeckTile { id: id.into(), kind: kind.into(), x, y, w, h, action: action.map(Into::into), ..Default::default() }
    }

    fn layouts(active: &str, tiles: Vec<DeckTile>) -> DeckLayouts {
        let mut portrait = DeckFace::portrait();
        portrait.pages[0] = tiles;
        let work = DeckLayout { id: "work".into(), name: "Work".into(), theme: DeckTheme::default(), portrait, landscape: DeckFace::landscape() };
        let games = DeckLayout { id: "games".into(), name: "Games".into(), ..work.clone() };
        DeckLayouts { version: 99, active: active.into(), layouts: vec![work, games] }
    }

    fn kinds(face: &DeckFace) -> Vec<(&str, Option<&str>)> {
        face.pages.iter().flatten().map(|t| (t.kind.as_str(), t.action.as_deref())).collect()
    }

    #[test]
    fn a_first_layout_of_what_is_offered_until_one_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        let first = d.layouts(&phone(1));
        assert_eq!((first.version, first.active.as_str(), first.layouts.len()), (0, "standard", 1));
        let all = HashSet::from(["media.play_pause", "media.next", "sound.volume", "sound.mute", "sound.mic_mute"]);
        assert_eq!(first.check(|a| all.contains(a)), Ok(()));
        let buttons = [("button", Some("media.play_pause")), ("button", Some("media.next")), ("button", Some("sound.mute")), ("button", Some("sound.mic_mute"))];
        for face in [&first.layouts[0].portrait, &first.layouts[0].landscape] {
            let mut want = vec![("clock", None), ("music", None), ("volume", Some("sound.volume"))];
            want.extend(buttons);
            assert_eq!(kinds(face), want);
        }
        assert_eq!(first.layouts[0].theme.id, "stock");
        assert!(!dir.path().join("deck.json").exists(), "not saved");

        let quiet = ready(dir.path().join("quiet.json"), Fake { without: &["sound"], ..Default::default() });
        assert_eq!(kinds(&quiet.layouts(&phone(1)).layouts[0].portrait), [("clock", None), ("music", None), ("button", Some("media.play_pause")), ("button", Some("media.next"))]);
    }

    #[test]
    fn saving_counts_versions_and_allows_what_the_tiles_run() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        let p = phone(1);
        d.set_action(&p, "media.next", false).unwrap();
        assert_eq!(d.act(&p, "system.lock", None), None);
        let mine = layouts("games", vec![tile("a", "button", (0, 0), (1, 1), Some("system.lock")), tile("b", "music", (0, 1), (4, 2), None)]);
        let saved = d.set_layouts(&p, mine.clone()).unwrap();
        assert_eq!((saved.version, saved.active.as_str()), (1, "games"), "the version is counted here");
        assert_eq!(saved.layouts, mine.layouts);
        assert_eq!(d.act(&p, "system.lock", None), Some(Act::Lock), "placing the tile allowed it");
        assert_eq!(d.act(&p, "media.next", None), None, "the rest is as it was");
        assert_eq!(d.act(&phone(2), "system.lock", None), None);
        assert_eq!(d.set_layouts(&p, mine).unwrap().version, 2);

        let again = deck(&dir);
        assert_eq!(again.layouts(&p).version, 2);
        assert_eq!(again.act(&p, "system.lock", None), Some(Act::Lock));
        assert_eq!(again.layouts(&phone(2)).version, 0, "another phone still has the first layout");

        // Only a kind that runs its action allows it
        d.set_action(&p, "system.lock", false).unwrap();
        d.set_layouts(&p, layouts("work", vec![tile("c", "clock", (0, 0), (2, 1), Some("system.lock"))])).unwrap();
        assert_eq!(d.act(&p, "system.lock", None), None);
    }

    #[test]
    fn layouts_that_fail_the_check_change_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        let p = phone(1);
        let good = d.set_layouts(&p, layouts("work", vec![tile("a", "button", (0, 0), (1, 1), Some("media.next"))])).unwrap();
        let file = std::fs::read(dir.path().join("deck.json")).unwrap();
        let mut big = layouts("work", vec![tile("a", "button", (0, 0), (1, 1), Some("media.next"))]);
        big.layouts[0].portrait.pages[0][0].props.insert("note".into(), "x".repeat(MAX_BYTES));
        for bad in [
            layouts("work", vec![tile("a", "button", (0, 0), (1, 1), Some("system.screen_off"))]),
            layouts("work", vec![tile("a", "button", (0, 0), (1, 1), Some("command.nope"))]),
            layouts("work", vec![tile("a", "music", (0, 0), (4, 2), None), tile("b", "clock", (1, 1), (2, 1), None)]),
            layouts("work", vec![tile("a", "volume", (0, 0), (3, 1), Some("sound.volume"))]),
            layouts("work", vec![tile("a", "weather", (0, 0), (1, 1), None)]),
        ] {
            assert!(d.set_layouts(&p, bad).is_err());
        }
        let refused = d.set_layouts(&p, big).unwrap_err();
        assert!(refused.contains("too big"), "{refused}");
        assert_eq!(d.layouts(&p), good);
        assert_eq!(std::fs::read(dir.path().join("deck.json")).unwrap(), file);
        assert_eq!(d.act(&p, "system.screen_off", None), None);
    }

    #[test]
    fn an_empty_or_unknown_active_layout_becomes_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let d = deck(&dir);
        for active in ["", "gone"] {
            assert_eq!(d.set_layouts(&phone(1), layouts(active, vec![])).unwrap().active, "work");
        }
        let none = d.set_layouts(&phone(1), DeckLayouts::default()).unwrap();
        assert_eq!((none.active.as_str(), none.layouts.len()), ("", 0));
    }

    #[test]
    fn an_older_deck_json_without_layouts_loads() {
        let dir = tempfile::tempdir().unwrap();
        let old = serde_json::json!({
            "phones": { phone(1).to_string(): { "system.lock": true, "media.next": false } },
            "commands": [{ "id": "c1", "name": "Backup", "command": "true" }],
        });
        std::fs::write(dir.path().join("deck.json"), old.to_string()).unwrap();
        let d = deck(&dir);
        assert_eq!(d.act(&phone(1), "system.lock", None), Some(Act::Lock));
        assert_eq!(d.act(&phone(1), "media.next", None), None);
        assert_eq!(d.prefs(&phone(1)).commands.len(), 1);
        assert_eq!(d.layouts(&phone(1)).version, 0);
        assert_eq!(d.set_layouts(&phone(1), layouts("work", vec![])).unwrap().version, 1);
        assert_eq!(deck(&dir).prefs(&phone(1)).commands.len(), 1, "kept beside the layouts");
    }
}
