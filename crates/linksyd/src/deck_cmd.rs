//! The user's own deck commands (docs/protocol.md §4.19, §4.26): what may be kept, starting
//! one for a tile, asking a toggle whether it is on, and trying one for the user to see.
//! Each runs on this computer as the user, with `sh -c`, from the home folder (from the
//! Flatpak, on the computer itself).

use std::process::Stdio;
use std::time::Duration;

use linksy_ipc::{CommandKind, CommandRun, DeckCommand};
use tracing::{debug, warn};

pub const MAX_COMMANDS: usize = 100;
pub const MAX_NAME: usize = 100;
pub const MAX_GROUP: usize = 40;
pub const MAX_ICON: usize = 32;
/// A command line is some shell, not a program.
pub const MAX_LINE: usize = 4000;
/// The group a command with none shows in.
pub const DEFAULT_GROUP: &str = "Commands";

/// A toggle's check has this long to answer, or its state is unknown.
const STATE_WAIT: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(2) };
/// A command being tried has this long.
const TEST_WAIT: Duration = if cfg!(test) { Duration::from_millis(600) } else { Duration::from_secs(10) };
/// The most of its output the user is shown.
const OUTPUT_MAX: usize = 400;

/// A command as it may be kept: trimmed, within the limits, a toggle with what it needs.
pub fn clean(c: DeckCommand) -> Result<DeckCommand, String> {
    let (name, command, group, icon, off, state) = (c.name.trim(), c.command.trim(), c.group.trim(), c.icon.trim(), c.off.trim(), c.state.trim());
    if name.is_empty() || command.is_empty() {
        return Err("each command needs a name and a command line".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("a command's name can be at most {MAX_NAME} characters long"));
    }
    if group.chars().count() > MAX_GROUP || group.chars().any(char::is_control) {
        return Err(format!("a group's name is one line of at most {MAX_GROUP} characters"));
    }
    // The phone's set may grow: any plain name is kept, and one it doesn't have draws as its default
    if icon.len() > MAX_ICON || !icon.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
        return Err(format!("an icon is up to {MAX_ICON} of a-z, 0-9 and _"));
    }
    if [command, off, state].iter().any(|line| line.len() > MAX_LINE) {
        return Err(format!("a command line can be at most {MAX_LINE} characters long"));
    }
    let (off, state) = match c.kind {
        CommandKind::Button => ("", ""),
        CommandKind::Toggle if state.is_empty() => return Err("a toggle needs a command that says whether it is on (it exits 0 while it is)".into()),
        CommandKind::Toggle => (off, state),
    };
    Ok(DeckCommand {
        id: c.id.clone(),
        name: name.into(),
        command: command.into(),
        group: if group == DEFAULT_GROUP { String::new() } else { group.into() },
        icon: icon.into(),
        confirm: c.confirm,
        kind: c.kind,
        off: off.into(),
        state: state.into(),
    })
}

/// What a toggle runs now: its `off` command (or `on` again, when it has none and the
/// same command flips it) while it is on, else `on`. Unknown counts as off.
pub fn toggle_line<'a>(now: Option<bool>, on: &'a str, off: &'a str) -> &'a str {
    match now {
        Some(true) if !off.is_empty() => off,
        _ => on,
    }
}

/// `line` for `sh -c`, from the home folder, in a group of its own (so all it started can
/// be stopped), asking nothing.
fn shell(line: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::from(linksy_host::command("sh", &[]));
    command.args(["-c", line]).stdin(Stdio::null()).process_group(0).kill_on_drop(true);
    if let Some(home) = std::env::var_os("HOME") {
        command.current_dir(home);
    }
    command
}

/// The same, to be left running when nothing waits for it any more.
fn detached(line: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::from(linksy_host::detached("sh", &[]));
    command.args(["-c", line]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    if let Some(home) = std::env::var_os("HOME") {
        command.current_dir(home);
    }
    command
}

fn stop(pid: Option<u32>) {
    if let Some(pid) = pid {
        // SAFETY: killpg only signals the group the command was started in
        unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
    }
}

/// Start a command on its own: it reads nothing, its output goes nowhere, and nothing
/// waits for it.
pub fn spawn(name: &str, line: &str) {
    match detached(line).spawn() {
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

/// Start a command and wait for it up to `wait`, so what it changed can be read after.
/// One that takes longer is left running.
pub async fn run_waiting(name: &str, line: &str, wait: Duration) {
    match detached(line).spawn() {
        Ok(mut child) => match tokio::time::timeout(wait, child.wait()).await {
            Ok(Ok(status)) if !status.success() => debug!("deck command {name}: {status}"),
            Err(_) => debug!("deck command {name} is still running"),
            _ => {}
        },
        Err(e) => warn!("could not run the deck command {name}: {e}"),
    }
}

/// What a toggle's check says: whether it exited 0; none if it didn't start or took too long.
pub async fn check(line: &str) -> Option<bool> {
    let mut child = shell(line).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    let pid = child.id();
    match tokio::time::timeout(STATE_WAIT, child.wait()).await {
        Ok(status) => status.ok().map(|s| s.success()),
        Err(_) => {
            stop(pid);
            let _ = child.wait().await;
            None
        }
    }
}

/// Run a line once and say how it went, for the user to see before they trust it.
pub async fn test(line: &str) -> CommandRun {
    let child = match shell(line).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(e) => return CommandRun { code: None, output: e.to_string(), timed_out: false },
    };
    let pid = child.id();
    match tokio::time::timeout(TEST_WAIT, child.wait_with_output()).await {
        Ok(Ok(done)) => CommandRun { code: done.status.code(), output: shown(&done.stdout, &done.stderr), timed_out: false },
        Ok(Err(e)) => CommandRun { code: None, output: e.to_string(), timed_out: false },
        Err(_) => {
            stop(pid);
            CommandRun { code: None, output: String::new(), timed_out: true }
        }
    }
}

/// The start of what a command printed, as plain text on a few lines.
fn shown(stdout: &[u8], stderr: &[u8]) -> String {
    let both = format!("{}\n{}", String::from_utf8_lossy(stdout), String::from_utf8_lossy(stderr));
    let plain: String = both.trim().chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    let mut text: String = plain.chars().take(OUTPUT_MAX).collect();
    if plain.chars().count() > OUTPUT_MAX {
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str, line: &str) -> DeckCommand {
        DeckCommand { name: name.into(), command: line.into(), ..Default::default() }
    }

    fn toggle(on: &str, off: &str, state: &str) -> DeckCommand {
        DeckCommand { kind: CommandKind::Toggle, off: off.into(), state: state.into(), ..command("VPN", on) }
    }

    #[test]
    fn what_may_be_kept() {
        let kept = clean(DeckCommand { name: "  Back Up ".into(), command: " rsync -a ~/a ~/b ".into(), group: " Work ".into(), icon: "terminal".into(), confirm: true, ..Default::default() }).unwrap();
        assert_eq!((kept.name.as_str(), kept.command.as_str(), kept.group.as_str(), kept.icon.as_str(), kept.confirm), ("Back Up", "rsync -a ~/a ~/b", "Work", "terminal", true));
        assert_eq!(clean(DeckCommand { group: "Commands".into(), ..command("A", "true") }).unwrap().group, "", "the default group is none");
        assert_eq!(clean(command("A", "true")).unwrap().kind, CommandKind::Button);
        for bad in [command("", "true"), command("A", "  "), command(&"x".repeat(MAX_NAME + 1), "true"), DeckCommand { group: "a\nb".into(), ..command("A", "true") }, DeckCommand { group: "g".repeat(MAX_GROUP + 1), ..command("A", "true") }, DeckCommand { icon: "Not An Icon".into(), ..command("A", "true") }, command("A", &"x".repeat(MAX_LINE + 1))] {
            assert!(clean(bad.clone()).is_err(), "{bad:?}");
        }
        // The group may be as long as it can be
        assert!(clean(DeckCommand { group: "g".repeat(MAX_GROUP), ..command("A", "true") }).is_ok());
    }

    #[test]
    fn a_toggle_needs_its_check_and_a_button_has_no_toggle_parts() {
        assert!(clean(toggle("vpn up", "vpn down", "")).is_err());
        let kept = clean(toggle(" vpn up ", " vpn down ", " vpn status ")).unwrap();
        assert_eq!((kept.command.as_str(), kept.off.as_str(), kept.state.as_str()), ("vpn up", "vpn down", "vpn status"));
        assert_eq!(clean(toggle("flip", "", "is-on")).unwrap().off, "", "no off command: the one command flips it");
        let button = clean(DeckCommand { off: "x".into(), state: "y".into(), ..command("A", "true") }).unwrap();
        assert_eq!((button.off.as_str(), button.state.as_str()), ("", ""), "a button keeps neither");
    }

    #[test]
    fn a_toggle_runs_off_while_on_and_on_otherwise() {
        assert_eq!(toggle_line(Some(true), "up", "down"), "down");
        assert_eq!(toggle_line(Some(false), "up", "down"), "up");
        assert_eq!(toggle_line(None, "up", "down"), "up", "unknown counts as off");
        assert_eq!(toggle_line(Some(true), "flip", ""), "flip", "no off command: it flips itself");
    }

    #[tokio::test]
    async fn a_check_is_its_exit_status_and_unknown_when_it_hangs() {
        assert_eq!(check("true").await, Some(true));
        assert_eq!(check("exit 3").await, Some(false));
        let started = std::time::Instant::now();
        assert_eq!(check("sleep 30").await, None, "too slow to say");
        assert!(started.elapsed() < Duration::from_secs(5), "and not waited for");
    }

    #[tokio::test]
    async fn trying_a_command_shows_how_it_went() {
        let ok = test("echo hello; echo oops >&2; exit 2").await;
        assert_eq!((ok.code, ok.timed_out), (Some(2), false));
        assert!(ok.output.contains("hello") && ok.output.contains("oops"), "{ok:?}");
        let long = test("head -c 5000 /dev/zero | tr '\\0' 'x'").await;
        assert_eq!(long.output.chars().count(), OUTPUT_MAX + 1, "cut, with a mark");
        assert!(long.output.ends_with('…'));
        let slow = test("sleep 30").await;
        assert_eq!((slow.code, slow.timed_out), (None, true));
        assert_eq!(test("printf 'a\\033[31mred\\007'").await.output, "a[31mred", "no control characters reach the page");
    }

    #[tokio::test]
    async fn a_command_runs_where_the_user_is_and_a_waited_one_is_done_when_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("ran");
        run_waiting("mark", &format!("touch {}", mark.display()), Duration::from_secs(5)).await;
        assert!(mark.exists());
        let home = test("pwd").await.output;
        assert_eq!(std::path::Path::new(&home), std::path::Path::new(&std::env::var("HOME").unwrap()), "from the home folder");
    }
}
