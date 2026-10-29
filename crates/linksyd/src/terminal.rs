//! Terminals for phones (docs/protocol.md §4.15): a shell on this computer, shown on a paired
//! phone. The phone opens one; this side decides. It's allowed per phone, behind a code
//! set here (only its Argon2id hash is kept); 5 wrong codes in a row lock it for that
//! phone until it's unlocked here; the user says yes the first time each day; from away
//! only when everything is allowed from away (the core checks that before asking here).
//! Shells outlive the connection: a phone that drops or closes the app reattaches later,
//! for up to a day.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use linksy_core::{ControlMsg, Event, Fingerprint, Node, Session, TerminalCall, TerminalInfo, Uuid, Verdict};
use linksy_i18n::t;
use linksy_ipc::{RunningTerminal, TerminalStatus};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub static TERMINALS: OnceLock<Terminals> = OnceLock::new();

/// Wrong codes in a row before the terminal locks for that phone.
const MAX_TRIES: u32 = 5;
/// How long a shell nobody's watching keeps running.
const DETACHED_FOR: Duration = Duration::from_secs(24 * 60 * 60);
/// Output kept to redraw a reattached terminal.
const SCROLLBACK: usize = 256 * 1024;
/// Shells one phone may have at once.
const PER_DEVICE: usize = 8;
/// Shortest terminal code.
pub const MIN_CODE: usize = 6;

#[derive(Default, Serialize, Deserialize)]
struct Prefs {
    /// The code's Argon2id hash (PHC string); none: terminals are off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(default)]
    allowed: Vec<Fingerprint>,
    #[serde(default)]
    locked: Vec<Fingerprint>,
    /// Wrong codes in a row, per phone.
    #[serde(default)]
    failures: HashMap<Fingerprint, u32>,
    /// The day each phone's terminal was last allowed by the user (`today()`).
    #[serde(default)]
    confirmed: HashMap<Fingerprint, i64>,
}

impl Prefs {
    /// Count a code attempt before it's checked, so requests in flight together can't
    /// outnumber the tries left. False: locked, or all the tries are already in flight.
    fn reserve_attempt(&mut self, device: &Fingerprint) -> bool {
        if self.locked.contains(device) {
            return false;
        }
        let tries = self.failures.entry(*device).or_default();
        if *tries >= MAX_TRIES {
            return false;
        }
        *tries += 1;
        true
    }

    /// A reserved attempt was wrong.
    fn attempt_failed(&mut self, device: &Fingerprint) -> Failure {
        if self.locked.contains(device) {
            return Failure::Locked;
        }
        let tries = self.failures.get(device).copied().unwrap_or(0);
        if tries >= MAX_TRIES {
            self.failures.remove(device);
            self.locked.push(*device);
            return Failure::Locks;
        }
        Failure::Left(MAX_TRIES - tries)
    }

    /// A reserved attempt was right: the count starts over. False: it was locked meanwhile.
    fn attempt_succeeded(&mut self, device: &Fingerprint) -> bool {
        if self.locked.contains(device) {
            return false;
        }
        self.failures.remove(device);
        true
    }
}

/// What a wrong code did.
#[derive(Debug, PartialEq)]
enum Failure {
    /// Tries left.
    Left(u32),
    /// This one locked the phone.
    Locks,
    /// Another attempt had already locked it.
    Locked,
}

#[derive(Clone)]
pub struct Terminals {
    inner: Arc<Mutex<State>>,
    path: PathBuf,
}

#[derive(Default)]
struct State {
    prefs: Prefs,
    running: HashMap<String, Term>,
    /// Accepted (or waiting for the user's yes) requests: which terminal they open.
    pending: HashMap<Uuid, Pending>,
    /// Bumped on every attach, so an old view detaching doesn't unhook a newer one.
    attaches: u64,
}

struct Pending {
    terminal: String,
    owner: Fingerprint,
    owner_name: String,
    size: (u16, u16),
    at: Instant,
}

struct Term {
    owner: Fingerprint,
    title: String,
    started_ms: u64,
    master: Arc<OwnedFd>,
    input: std::sync::mpsc::Sender<Vec<u8>>,
    /// The shell's process group (it leads its own session).
    pid: i32,
    scrollback: VecDeque<u8>,
    attached: Option<(u64, mpsc::UnboundedSender<Vec<u8>>)>,
    detached: Option<Instant>,
}

impl Terminals {
    pub fn load(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join("terminal.json");
        let prefs = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { inner: Arc::new(Mutex::new(State { prefs, ..Default::default() })), path }
    }

    fn save(&self, state: &State) {
        let Ok(bytes) = serde_json::to_vec_pretty(&state.prefs) else { return };
        let tmp = self.path.with_extension("tmp");
        let written = (|| {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            f.write_all(&bytes)?;
            std::fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = written {
            warn!("could not save terminal settings: {e}");
        }
    }

    pub fn status(&self) -> TerminalStatus {
        let st = self.inner.lock().unwrap();
        let mut running: Vec<RunningTerminal> = st
            .running
            .iter()
            .map(|(id, t)| RunningTerminal { id: id.clone(), device: t.owner, title: t.title.clone(), started_ms: t.started_ms, attached: t.attached.is_some() })
            .collect();
        running.sort_by_key(|t| t.started_ms);
        TerminalStatus { code_set: st.prefs.code.is_some(), allowed: st.prefs.allowed.clone(), locked: st.prefs.locked.clone(), running }
    }

    /// Set the terminal code (or with `None`, forget it, which turns terminals off).
    pub async fn set_code(&self, code: Option<String>) -> Result<(), String> {
        let hash = match code {
            Some(code) if code.chars().count() < MIN_CODE => return Err(format!("The code needs at least {MIN_CODE} characters.")),
            Some(code) => Some(tokio::task::spawn_blocking(move || hash(&code)).await.map_err(|e| e.to_string())??),
            None => None,
        };
        let mut st = self.inner.lock().unwrap();
        if hash.is_none() {
            end_where(&mut st, |_, _| true);
        }
        st.prefs.code = hash;
        st.prefs.failures.clear();
        self.save(&st);
        Ok(())
    }

    /// Allow terminals for a phone, or stop allowing them (which ends its terminals).
    pub fn allow(&self, device: Fingerprint, on: bool) {
        let mut st = self.inner.lock().unwrap();
        st.prefs.allowed.retain(|d| *d != device);
        if on {
            st.prefs.allowed.push(device);
        } else {
            end_where(&mut st, |_, t| t.owner == device);
        }
        self.save(&st);
    }

    /// End every terminal (the service is stopping).
    pub fn end_all(&self) {
        end_where(&mut self.inner.lock().unwrap(), |_, _| true);
    }

    pub fn unlock(&self, device: Fingerprint) {
        let mut st = self.inner.lock().unwrap();
        st.prefs.locked.retain(|d| *d != device);
        st.prefs.failures.remove(&device);
        self.save(&st);
    }

    pub fn end(&self, id: &str) -> bool {
        let mut st = self.inner.lock().unwrap();
        let found = st.running.contains_key(id);
        end_where(&mut st, |tid, _| tid == id);
        found
    }

    pub async fn run(self, node: Node) {
        let Some(mut calls) = node.incoming_terminals() else { return };
        let mut events = node.subscribe();
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                call = calls.recv() => match call {
                    Some(call) => {
                        let this = self.clone();
                        tokio::spawn(async move { this.judge(call).await });
                    }
                    None => return,
                },
                event = events.recv() => match event {
                    Ok(Event::TerminalListRequest { from }) => {
                        let list = self.list_for(&from.id);
                        let node = node.clone();
                        tokio::spawn(async move {
                            if let Err(e) = node.send_terminals(&from.id, list).await {
                                debug!("terminal list to {}: {e}", from.name);
                            }
                        });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    _ => {}
                },
                _ = tick.tick() => self.tidy(&node),
            }
        }
    }

    /// Detached too long, unpaired, or no longer allowed: end it. Forgotten requests go.
    fn tidy(&self, node: &Node) {
        let paired: Vec<Fingerprint> = node.devices().into_iter().filter(|d| d.trusted).map(|d| d.device.id).collect();
        let mut st = self.inner.lock().unwrap();
        let allowed = st.prefs.allowed.clone();
        end_where(&mut st, |_, t| t.detached.is_some_and(|d| d.elapsed() > DETACHED_FOR) || !paired.contains(&t.owner) || !allowed.contains(&t.owner));
        st.pending.retain(|_, p| p.at.elapsed() < Duration::from_secs(300));
    }

    fn list_for(&self, owner: &Fingerprint) -> Vec<TerminalInfo> {
        let st = self.inner.lock().unwrap();
        let mut list: Vec<TerminalInfo> = st.running.iter().filter(|(_, t)| t.owner == *owner).map(|(id, t)| TerminalInfo { id: id.clone(), title: t.title.clone(), started_ms: t.started_ms }).collect();
        list.sort_by_key(|t| t.started_ms);
        list
    }

    async fn judge(&self, call: TerminalCall) {
        let from = call.from.clone();
        let decline = |reason: &str, tries_left: Option<u32>| Verdict::Decline { reason: reason.into(), tries_left };
        let (hash, allowed, reserved) = {
            let mut st = self.inner.lock().unwrap();
            let (hash, allowed) = (st.prefs.code.clone(), st.prefs.allowed.contains(&from.id));
            // Taken before the slow check below, under this lock: a burst of requests can't all pass it.
            let reserved = hash.is_some() && allowed && st.prefs.reserve_attempt(&from.id);
            (hash, allowed, reserved)
        };
        let Some(hash) = hash.filter(|_| allowed) else { return call.answer(decline("off", None)) };
        if !reserved {
            return call.answer(decline("locked", None));
        }
        let code = call.open.code.clone();
        let right = tokio::task::spawn_blocking(move || verify(&hash, &code)).await.unwrap_or(false);
        let mut st = self.inner.lock().unwrap();
        if !right {
            let verdict = match st.prefs.attempt_failed(&from.id) {
                Failure::Left(left) => decline("code", Some(left)),
                Failure::Locked => decline("locked", None),
                Failure::Locks => {
                    warn!("terminal locked for {} after {MAX_TRIES} wrong codes", from.name);
                    crate::notify::show(t!("Terminal locked for {device}", device = from.name), t!("{tries} wrong codes in a row. Unlock it in Linksy on this computer if it was you.", tries = MAX_TRIES));
                    decline("locked", None)
                }
            };
            self.save(&st);
            return call.answer(verdict);
        }
        if !st.prefs.attempt_succeeded(&from.id) {
            return call.answer(decline("locked", None));
        }
        let resume = call.open.resume.clone().filter(|id| st.running.get(id).is_some_and(|t| t.owner == from.id));
        let terminal = match resume {
            Some(id) => id,
            None if st.running.values().filter(|t| t.owner == from.id).count() >= PER_DEVICE => return call.answer(decline("limit", None)),
            None => Uuid::new_v4().to_string(),
        };
        let size = (call.open.cols.clamp(10, 1000), call.open.rows.clamp(4, 500));
        st.pending.insert(call.request, Pending { terminal: terminal.clone(), owner: from.id, owner_name: from.name.clone(), size, at: Instant::now() });
        let today_ok = st.prefs.confirmed.get(&from.id) == Some(&today());
        self.save(&st);
        call.answer(if today_ok { Verdict::Accept { terminal } } else { Verdict::Confirm { terminal } });
    }

    /// Run an accepted terminal session: start or reattach its shell and relay it.
    pub fn attach(&self, session: Session) {
        let pending = self.inner.lock().unwrap().pending.remove(&session.id);
        let Some(pending) = pending else { return };
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.serve(session, pending).await {
                debug!("terminal session: {e}");
            }
        });
    }

    async fn serve(&self, mut session: Session, p: Pending) -> std::io::Result<()> {
        let (tx, mut output) = mpsc::unbounded_channel();
        let (attach, replay, master) = {
            let mut st = self.inner.lock().unwrap();
            if !st.running.contains_key(&p.terminal) {
                let term = self.start(&p)?;
                info!("terminal started for {}", p.owner_name);
                st.running.insert(p.terminal.clone(), term);
            }
            st.attaches += 1;
            let attach = st.attaches;
            st.prefs.confirmed.insert(p.owner, today());
            let term = st.running.get_mut(&p.terminal).ok_or_else(|| std::io::Error::other("ended"))?;
            term.attached = Some((attach, tx));
            term.detached = None;
            let replay: Vec<u8> = term.scrollback.iter().copied().collect();
            let master = term.master.clone();
            self.save(&st);
            (attach, replay, master)
        };
        crate::notify::show(t!("Terminal open from {device}", device = p.owner_name), t!("It runs as you until it's ended on the phone, or here in Linksy.").into());
        // The phone's size; a resize also makes full-screen programs redraw.
        resize(&master, p.size);
        session.send.write_all(&replay).await?;
        let mut frames = Frames::default();
        let mut buf = vec![0u8; 16 * 1024];
        let closed = loop {
            tokio::select! {
                out = output.recv() => match out {
                    Some(bytes) => session.send.write_all(&bytes).await?,
                    // The shell exited.
                    None => break true,
                },
                read = session.recv.read(&mut buf) => match read {
                    // quinn's read: cancel-safe, so select! may drop it.
                    Ok(None) | Err(_) => break false,
                    Ok(Some(n)) => {
                        for msg in frames.push(&buf[..n]) {
                            match msg {
                                ControlMsg::TermInput { data } => self.input(&p.terminal, data.into_bytes()),
                                ControlMsg::TermResize { cols, rows } => resize(&master, (cols.clamp(10, 1000), rows.clamp(4, 500))),
                                ControlMsg::TermClose => {
                                    let mut st = self.inner.lock().unwrap();
                                    end_where(&mut st, |id, _| id == p.terminal);
                                }
                                _ => {}
                            }
                        }
                    }
                },
            }
        };
        // Detached, not ended: the shell keeps running for a while.
        if !closed {
            let mut st = self.inner.lock().unwrap();
            if let Some(t) = st.running.get_mut(&p.terminal).filter(|t| t.attached.as_ref().is_some_and(|(a, _)| *a == attach)) {
                t.attached = None;
                t.detached = Some(Instant::now());
            }
        }
        let _ = session.send.finish();
        Ok(())
    }

    fn input(&self, id: &str, bytes: Vec<u8>) {
        if let Some(t) = self.inner.lock().unwrap().running.get(id) {
            let _ = t.input.send(bytes);
        }
    }

    /// Start a login shell on a new pseudo-terminal, with threads to read and write it.
    fn start(&self, p: &Pending) -> std::io::Result<Term> {
        let (master, child) = spawn_shell(p.size)?;
        let master = Arc::new(master);
        let pid = child.id() as i32;
        let (input, inputs) = std::sync::mpsc::channel::<Vec<u8>>();
        let mut writer = std::fs::File::from(master.try_clone()?);
        std::thread::spawn(move || {
            while let Ok(bytes) = inputs.recv() {
                if writer.write_all(&bytes).is_err() {
                    break;
                }
            }
        });
        let mut reader = std::fs::File::from(master.try_clone()?);
        let (this, id) = (self.clone(), p.terminal.clone());
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            // EIO once the shell is gone.
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                this.output(&id, &buf[..n]);
            }
            let mut st = this.inner.lock().unwrap();
            // Dropping the term drops its sender: the attached session sees the end.
            if st.running.remove(&id).is_some() {
                info!("terminal ended");
            }
        });
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        let shell = std::env::var("SHELL").ok().and_then(|s| s.rsplit('/').next().map(str::to_string)).unwrap_or_else(|| "sh".into());
        Ok(Term { owner: p.owner, title: shell, started_ms: now_ms(), master, input, pid, scrollback: VecDeque::new(), attached: None, detached: None })
    }

    fn output(&self, id: &str, bytes: &[u8]) {
        let mut st = self.inner.lock().unwrap();
        let Some(t) = st.running.get_mut(id) else { return };
        t.scrollback.extend(bytes);
        let over = t.scrollback.len().saturating_sub(SCROLLBACK);
        t.scrollback.drain(..over);
        if let Some(title) = window_title(bytes) {
            t.title = title;
        }
        if let Some((_, tx)) = &t.attached {
            let _ = tx.send(bytes.to_vec());
        }
    }
}

/// End (hang up) every terminal `which` picks, by id and terminal.
fn end_where(st: &mut State, which: impl Fn(&str, &Term) -> bool) {
    let ids: Vec<String> = st.running.iter().filter(|(id, t)| which(id, t)).map(|(id, _)| id.clone()).collect();
    for id in ids {
        if let Some(t) = st.running.remove(&id) {
            hang_up(t.pid);
        }
    }
}

/// End a terminal's shell and everything started in it: every process in its session
/// (the shell leads it), whatever process group a job put itself in. Hung up first, so
/// editors can save; killed two seconds later if still there.
fn hang_up(session: i32) {
    let signal_all = move |signal: i32| {
        let Ok(procs) = std::fs::read_dir("/proc") else { return };
        for p in procs.flatten() {
            let Some(pid) = p.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
            if unsafe { libc::getsid(pid) } == session {
                unsafe {
                    libc::kill(pid, signal);
                }
            }
        }
    };
    signal_all(libc::SIGHUP);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        signal_all(libc::SIGKILL);
    });
}

fn spawn_shell((cols, rows): (u16, u16)) -> std::io::Result<(OwnedFd, std::process::Child)> {
    let (mut master, mut slave) = (-1, -1);
    let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &size) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
    }
    let env = [("TERM", "xterm-256color"), ("COLORTERM", "truecolor"), ("LINKSY_TERMINAL", "1")];
    let flatpak = linksy_host::flatpak();
    let mut cmd = if flatpak {
        // The computer's own login shell, started outside the sandbox on this terminal
        let mut c = linksy_host::command("sh", &env);
        c.args(["-c", "cd; shell=$(getent passwd \"$(id -un)\" | cut -d: -f7); exec \"${shell:-/bin/sh}\" -l"]);
        c
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let name = shell.rsplit('/').next().unwrap_or("sh").to_string();
        let mut c = std::process::Command::new(&shell);
        // A login shell: "-bash" reads the profile, as a terminal window's would.
        c.arg0(format!("-{name}")).current_dir(std::env::var("HOME").unwrap_or_else(|_| "/".into())).envs(env);
        c
    };
    cmd.stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
    unsafe {
        cmd.pre_exec(move || {
            // Its own session, with the terminal as its controlling one (job control,
            // Ctrl+C, window size changes). From the Flatpak, the computer's side makes it
            // the shell's instead.
            if libc::setsid() < 0 || (!flatpak && libc::ioctl(0, libc::TIOCSCTTY, 0) < 0) {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok((master, cmd.spawn()?))
}

fn resize(master: &OwnedFd, (cols, rows): (u16, u16)) {
    let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    unsafe {
        libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &size);
    }
}

/// The last window title (OSC 0 or 2) in `bytes`, if any.
fn window_title(bytes: &[u8]) -> Option<String> {
    let mut found = None;
    let mut rest = bytes;
    while let Some(start) = rest.windows(2).position(|w| w == b"\x1b]") {
        let after = &rest[start + 2..];
        let Some(semi) = after.iter().position(|&b| b == b';') else { break };
        let kind = &after[..semi];
        let text = &after[semi + 1..];
        let end = text.iter().position(|&b| b == 0x07 || b == 0x1b).unwrap_or(text.len());
        if kind == b"0" || kind == b"2" {
            found = Some(String::from_utf8_lossy(&text[..end]).chars().take(120).collect());
        }
        rest = &text[end..];
    }
    found
}

/// Framed control messages (u32 BE length + JSON), from a stream read in pieces.
#[derive(Default)]
struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    fn push(&mut self, bytes: &[u8]) -> Vec<ControlMsg> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while self.buf.len() >= 4 {
            let len = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
            if len > 1 << 20 {
                // Nonsense: drop what's buffered rather than wait forever.
                self.buf.clear();
                break;
            }
            if self.buf.len() < 4 + len {
                break;
            }
            if let Ok(msg) = serde_json::from_slice(&self.buf[4..4 + len]) {
                out.push(msg);
            }
            self.buf.drain(..4 + len);
        }
        out
    }
}

fn hash(code: &str) -> Result<String, String> {
    let mut salt = [0u8; 16];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut salt)).map_err(|e| e.to_string())?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| e.to_string())?;
    Argon2::default().hash_password(code.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| e.to_string())
}

fn verify(hash: &str, code: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(code.as_bytes(), &h).is_ok())
}

/// Today, locally, as a number that changes at midnight.
fn today() -> i64 {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
    unsafe {
        libc::localtime_r(&now, tm.as_mut_ptr());
        let tm = tm.assume_init();
        tm.tm_year as i64 * 1000 + tm.tm_yday as i64
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_hash_and_check() {
        let h = hash("482913").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify(&h, "482913"));
        assert!(!verify(&h, "482914"));
    }

    #[test]
    fn frames_come_whole_whatever_the_pieces() {
        let mut bytes = linksy_core::protocol::control_frame(&ControlMsg::TermInput { data: "ls\r".into() });
        bytes.extend(linksy_core::protocol::control_frame(&ControlMsg::TermResize { cols: 80, rows: 24 }));
        let mut frames = Frames::default();
        let mut got = Vec::new();
        for piece in bytes.chunks(3) {
            got.extend(frames.push(piece));
        }
        assert_eq!(got, vec![ControlMsg::TermInput { data: "ls\r".into() }, ControlMsg::TermResize { cols: 80, rows: 24 }]);
    }

    #[test]
    fn titles_from_osc() {
        assert_eq!(window_title(b"x\x1b]0;claude \xe2\x80\x94 ~/Project\x07y").as_deref(), Some("claude — ~/Project"));
        assert_eq!(window_title(b"\x1b]2;vim\x1b\\"), Some("vim".into()));
        assert_eq!(window_title(b"plain"), None);
    }

    fn phone() -> Fingerprint {
        Fingerprint([7; 32])
    }

    #[test]
    fn five_wrong_codes_in_a_row_lock() {
        let (mut prefs, id) = (Prefs::default(), phone());
        for left in (1..MAX_TRIES).rev() {
            assert!(prefs.reserve_attempt(&id));
            assert_eq!(prefs.attempt_failed(&id), Failure::Left(left));
        }
        assert!(prefs.reserve_attempt(&id));
        assert_eq!(prefs.attempt_failed(&id), Failure::Locks);
        assert!(prefs.locked.contains(&id));
        assert!(!prefs.reserve_attempt(&id));
    }

    #[test]
    fn a_right_code_resets_the_count_and_leaves_it_unlocked() {
        let (mut prefs, id) = (Prefs::default(), phone());
        for _ in 0..MAX_TRIES - 1 {
            assert!(prefs.reserve_attempt(&id));
            prefs.attempt_failed(&id);
        }
        assert!(prefs.reserve_attempt(&id));
        prefs.attempt_succeeded(&id);
        assert!(!prefs.locked.contains(&id));
        assert!(!prefs.failures.contains_key(&id));
        for _ in 0..MAX_TRIES - 1 {
            assert!(prefs.reserve_attempt(&id));
            prefs.attempt_failed(&id);
        }
        assert!(!prefs.locked.contains(&id));
    }

    #[test]
    fn attempts_in_flight_cannot_outrun_the_budget() {
        let (mut prefs, id) = (Prefs::default(), phone());
        // 100 requests arrive before any verdict is known.
        let admitted = (0..100).filter(|_| prefs.reserve_attempt(&id)).count();
        assert_eq!(admitted, MAX_TRIES as usize);
        for _ in 0..admitted {
            prefs.attempt_failed(&id);
        }
        assert!(prefs.locked.contains(&id));
        assert!(!prefs.reserve_attempt(&id));
    }

    #[test]
    fn a_right_code_among_attempts_in_flight_leaves_it_unlocked() {
        let (mut prefs, id) = (Prefs::default(), phone());
        for _ in 0..MAX_TRIES {
            assert!(prefs.reserve_attempt(&id));
        }
        assert!(prefs.attempt_succeeded(&id));
        for _ in 0..MAX_TRIES - 1 {
            prefs.attempt_failed(&id);
        }
        assert!(!prefs.locked.contains(&id));
        assert!(prefs.reserve_attempt(&id));
    }

    #[test]
    fn failures_while_others_are_pending_lock_once() {
        let (mut prefs, id) = (Prefs::default(), phone());
        for _ in 0..MAX_TRIES {
            assert!(prefs.reserve_attempt(&id));
        }
        assert_eq!(prefs.attempt_failed(&id), Failure::Locks);
        for _ in 1..MAX_TRIES {
            assert_eq!(prefs.attempt_failed(&id), Failure::Locked);
        }
        assert_eq!(prefs.locked.iter().filter(|d| **d == id).count(), 1);
    }

    #[test]
    fn a_right_code_resolving_after_a_lock_is_rejected() {
        let (mut prefs, id) = (Prefs::default(), phone());
        for _ in 0..MAX_TRIES {
            assert!(prefs.reserve_attempt(&id));
        }
        assert_eq!(prefs.attempt_failed(&id), Failure::Locks);
        assert!(!prefs.attempt_succeeded(&id));
        assert!(prefs.locked.contains(&id));
    }
}
