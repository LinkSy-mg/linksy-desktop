//! pkexec with Linksy's own password prompt. Where no polkit agent runs (Openbox, Hyprland,
//! Sway…), pkexec asks on its terminal: it gets a private one, and the app asks the person
//! in its own window, rather than starting another program's prompt in the session
//! (desktop#67). With an agent running, that agent asks, as before.

use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

/// pkexec wants the password.
#[derive(Clone, Debug)]
pub struct Prompt {
    /// Whose password, as polkit names them ("Ada Lovelace (ada)").
    pub who: String,
    /// The last one didn't work.
    pub again: bool,
}

/// Asks the person for the password (None: they cancelled).
pub type Ask = Box<dyn FnMut(&Prompt) -> Option<String> + Send>;

/// How a run ended.
#[derive(Debug)]
pub struct Outcome {
    /// pkexec's exit status, the program's once it ran; 126 when the password wasn't given.
    pub code: i32,
    /// Nothing happened for two minutes (a prompt elsewhere that never showed), so it was
    /// stopped.
    pub timed_out: bool,
    /// The password was wrong, every try.
    pub wrong: bool,
}

/// How long pkexec may wait in silence: a prompt that never shows up mustn't leave Linksy,
/// or the desktop, waiting.
const WAIT: Duration = Duration::from_secs(120);
const TRIES: usize = 3;

/// Run `argv` (a program and its arguments) as root through pkexec. `ask` gets the password
/// when pkexec asks Linksy for it (None cancels); `line`, each line the program prints.
pub fn run(argv: &[OsString], mut ask: impl FnMut(&Prompt) -> Option<String>, mut line: impl FnMut(&str)) -> std::io::Result<Outcome> {
    let mut tries = 1;
    loop {
        let outcome = attempt(argv, &mut ask, &mut line, tries > 1)?;
        // Nothing ran as root yet: ask again
        if !outcome.wrong || tries == TRIES {
            return Ok(outcome);
        }
        tries += 1;
    }
}

fn attempt(argv: &[OsString], ask: &mut impl FnMut(&Prompt) -> Option<String>, line: &mut impl FnMut(&str), again: bool) -> std::io::Result<Outcome> {
    let (master, mut child) = spawn(argv)?;
    let mut master = File::from(master);
    let mut talk = Talk::new(std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).ok());
    let started = Instant::now();
    let (mut heard, mut timed_out, mut cancelled) = (false, false, false);
    let mut chunk = [0u8; 4096];
    loop {
        let mut poll = libc::pollfd { fd: master.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut poll, 1, 100) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if ready > 0 {
            // An error (EIO) once every program has closed the terminal
            let Ok(n @ 1..) = master.read(&mut chunk) else { break };
            heard = true;
            for l in talk.feed(&chunk[..n]) {
                line(&l);
            }
            continue;
        }
        if !heard && !timed_out && started.elapsed() > WAIT {
            stop(&child);
            timed_out = true;
        }
        match talk.waiting(echo_off(&master)) {
            Some(Wants::Identity(n)) => master.write_all(format!("{n}\n").as_bytes())?,
            Some(Wants::Password) => match ask(&Prompt { who: talk.who.clone(), again }) {
                Some(password) => {
                    let mut bytes = password.into_bytes();
                    bytes.push(b'\n');
                    let wrote = master.write_all(&bytes);
                    bytes.fill(0);
                    wrote?;
                }
                None => {
                    cancelled = true;
                    stop(&child);
                }
            },
            None => {}
        }
    }
    let status = child.wait()?;
    let code = if cancelled { 126 } else { status.code().unwrap_or(1) };
    Ok(Outcome { code, timed_out, wrong: talk.failed && !cancelled })
}

/// pkexec on a terminal of its own.
fn spawn(argv: &[OsString]) -> std::io::Result<(OwnedFd, Child)> {
    let (mut master, mut slave) = (-1, -1);
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
    }
    let flatpak = crate::flatpak();
    let mut cmd = crate::command("pkexec", &[]);
    cmd.args(argv).stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
    unsafe {
        cmd.pre_exec(move || {
            // Its own session, with the terminal as its controlling one, where pkexec asks.
            // From the Flatpak, the computer's side makes it pkexec's instead.
            if libc::setsid() < 0 || (!flatpak && libc::ioctl(0, libc::TIOCSCTTY, 0) < 0) {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok((master, cmd.spawn()?))
}

/// SIGTERM, which flatpak-spawn passes on. Not reaped yet, so still pkexec's pid; it may
/// already run the program as root, which this can't stop.
fn stop(child: &Child) {
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
}

/// Whether the terminal stopped echoing, as pkexec does to read a password.
fn echo_off(master: &File) -> bool {
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    unsafe { libc::tcgetattr(master.as_raw_fd(), &mut t) == 0 && t.c_lflag & libc::ECHO == 0 }
}

#[derive(Debug, PartialEq)]
enum Wants {
    /// Which of the people polkit offers (1, 2…).
    Identity(usize),
    Password,
}

/// pkexec's own prompt, as it comes out of the terminal (polkit's text listener, in
/// English), and the program's lines around it.
struct Talk {
    user: Option<String>,
    partial: Vec<u8>,
    asking: bool,
    identities: Vec<String>,
    who: String,
    failed: bool,
}

impl Talk {
    fn new(user: Option<String>) -> Self {
        Self { user, partial: Vec::new(), asking: false, identities: Vec::new(), who: String::new(), failed: false }
    }

    /// The program's lines in `bytes`, pkexec's conversation left out.
    fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &b in bytes {
            if b != b'\n' {
                self.partial.push(b);
                continue;
            }
            let text = clean(&std::mem::take(&mut self.partial));
            if let Some(l) = self.hear(text) {
                lines.push(l);
            }
        }
        lines
    }

    fn hear(&mut self, text: String) -> Option<String> {
        if text.starts_with("==== AUTHENTICATING FOR ") {
            self.asking = true;
            self.identities.clear();
            return None;
        }
        if !self.asking {
            return Some(text);
        }
        match text.as_str() {
            "==== AUTHENTICATION COMPLETE ====" | "==== AUTHENTICATION CANCELED ====" => self.asking = false,
            "==== AUTHENTICATION FAILED ====" => {
                self.asking = false;
                self.failed = true;
            }
            _ => {
                if let Some(who) = text.strip_prefix("Authenticating as: ") {
                    self.who = who.to_string();
                } else if let Some(identity) = identity(&text) {
                    self.identities.push(identity);
                }
            }
        }
        None
    }

    /// What pkexec waits for, from the line it left open; `echo_off`: the terminal's state.
    fn waiting(&mut self, echo_off: bool) -> Option<Wants> {
        if !self.asking || self.partial.is_empty() {
            return None;
        }
        let wants = if clean(&self.partial).starts_with("Choose identity to authenticate as") {
            // The person using Linksy, when polkit lets them; else the first one offered
            let mine = |i: &String| self.user.as_deref().is_some_and(|u| i.ends_with(&format!("({u})")) || i.ends_with(&format!("unix-user:{u}")));
            let n = self.identities.iter().position(mine).unwrap_or(0);
            self.who = self.identities.get(n).cloned().unwrap_or_default();
            Wants::Identity(n + 1)
        } else if echo_off {
            Wants::Password
        } else {
            return None;
        };
        self.partial.clear();
        Some(wants)
    }
}

/// " 2.  Ada Lovelace (ada)": one of the people polkit offers.
fn identity(text: &str) -> Option<String> {
    let (n, who) = text.trim_start().split_once(".  ")?;
    n.parse::<usize>().ok()?;
    Some(who.to_string())
}

/// A line as a terminal shows it: from its last carriage return, without colors.
fn clean(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let shown = text.rsplit('\r').find(|s| !s.is_empty()).unwrap_or("");
    let mut out = String::with_capacity(shown.len());
    let mut chars = shown.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
        } else if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_carriage_returns() {
        assert_eq!(clean(b"\x1b[1;31m==== AUTHENTICATION COMPLETE ====\x1b[0m\r"), "==== AUTHENTICATION COMPLETE ====");
        assert_eq!(clean(b"Downloading 10%\rDownloading 80%\r"), "Downloading 80%");
    }

    #[test]
    fn one_identity() {
        let mut talk = Talk::new(Some("ada".into()));
        let shown = talk.feed(b"\x1b[1;31m==== AUTHENTICATING FOR org.freedesktop.policykit.exec ====\x1b[0m\r\nAuthentication is needed to run `/usr/libexec/linksy/linksy-system-setup usb' as the super user\r\nAuthenticating as: Ada Lovelace (ada)\r\nPassword: ");
        assert!(shown.is_empty());
        assert_eq!(talk.waiting(false), None);
        assert_eq!(talk.waiting(true), Some(Wants::Password));
        assert_eq!(talk.who, "Ada Lovelace (ada)");
        assert_eq!(talk.waiting(true), None);
        let shown = talk.feed(b"\r\n\x1b[1;31m==== AUTHENTICATION COMPLETE ====\x1b[0m\r\nSTEP Letting Linksy use phones plugged in by cable\r\n");
        assert_eq!(shown, ["STEP Letting Linksy use phones plugged in by cable"]);
        assert!(!talk.failed);
    }

    #[test]
    fn picks_the_person_using_linksy() {
        let mut talk = Talk::new(Some("ada".into()));
        talk.feed(b"==== AUTHENTICATING FOR org.freedesktop.policykit.exec ====\r\nAuthentication is needed\r\nMultiple identities can be used for authentication:\r\n 1.  Charles Babbage (charles)\r\n 2.  Ada Lovelace (ada)\r\nChoose identity to authenticate as (1-2): ");
        assert_eq!(talk.waiting(false), Some(Wants::Identity(2)));
        assert_eq!(talk.who, "Ada Lovelace (ada)");
        let mut other = Talk::new(Some("grace".into()));
        other.feed(b"==== AUTHENTICATING FOR x ====\r\nm\r\nMultiple identities can be used for authentication:\r\n 1.  unix-user:root\r\n 2.  Charles Babbage (charles)\r\nChoose identity to authenticate as (1-2): ");
        assert_eq!(other.waiting(false), Some(Wants::Identity(1)));
        assert_eq!(other.who, "unix-user:root");
    }

    #[test]
    fn a_wrong_password() {
        let mut talk = Talk::new(None);
        talk.feed(b"==== AUTHENTICATING FOR x ====\r\nm\r\nAuthenticating as: Ada (ada)\r\nPassword: ");
        assert_eq!(talk.waiting(true), Some(Wants::Password));
        let shown = talk.feed(b"\r\n==== AUTHENTICATION FAILED ====\r\nError executing command as another user: Not authorized\r\n");
        assert!(talk.failed);
        assert_eq!(shown, ["Error executing command as another user: Not authorized"]);
    }
}
