//! Keeps the service going where nothing else does. Under systemd, `linksyd.service` restarts
//! it after a crash; started any other way (the Flatpak, an XDG autostart entry, the app
//! starting it) a crash left it dead until the next login. There it runs as a child of a small
//! copy of itself that starts it again, with the unit file's rules: after 3 s, for a crash or
//! a kill, never for a stop that was asked for, and not forever (5 times in a minute).

use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitCode, ExitStatus};
use std::time::{Duration, Instant};

/// Set in the copy that is the service, and so in what it starts.
const SUPERVISED: &str = "LINKSY_SUPERVISED";
/// Starts again after this long.
const PAUSE: Duration = Duration::from_secs(3);
/// More restarts than this in `WINDOW` and it gives up.
const BURST: usize = 5;
const WINDOW: Duration = Duration::from_secs(60);

/// Some(exit code) when this process only watches the service, None when it is the service.
pub fn run() -> Option<ExitCode> {
    // systemd supervises its own, and `LINKSY_NO_SUPERVISOR=1` is for running it by hand
    if [SUPERVISED, "INVOCATION_ID", "LINKSY_NO_SUPERVISOR"].iter().any(|v| std::env::var_os(v).is_some()) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let me = std::process::id() as libc::pid_t;
    let code = watch(
        || {
            let mut service = Command::new(&exe);
            service.args(&args).env(SUPERVISED, "1");
            // SAFETY: only system calls between fork and exec
            unsafe { service.pre_exec(move || die_with_parent(me)) };
            service.spawn()
        },
        PAUSE,
    );
    Some(ExitCode::from(code))
}

/// The service stops when the copy watching it does (a log-out, a kill): it gets a TERM and
/// shuts down properly.
fn die_with_parent(parent: libc::pid_t) -> std::io::Result<()> {
    // SAFETY: sets a flag on this process; `raise` is for a parent that was already gone
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong, 0, 0, 0);
        if libc::getppid() != parent {
            libc::raise(libc::SIGTERM);
        }
    }
    Ok(())
}

enum Outcome {
    /// It ended on purpose (or can't be helped by starting it again): this is the exit code.
    Done(u8),
    /// It crashed or was killed.
    Crashed(String),
}

fn outcome(status: ExitStatus) -> Outcome {
    match (status.code(), status.signal()) {
        (Some(0), _) => Outcome::Done(0),
        // Bad options: the same ones would fail the same way
        (Some(2), _) => Outcome::Done(2),
        (Some(code), _) => Outcome::Crashed(format!("exit code {code}")),
        // Asked to stop
        (_, Some(libc::SIGTERM | libc::SIGINT | libc::SIGHUP | libc::SIGPIPE)) => Outcome::Done(0),
        (_, Some(signal)) => Outcome::Crashed(format!("signal {signal}")),
        _ => Outcome::Crashed("unknown".into()),
    }
}

fn watch(mut spawn: impl FnMut() -> std::io::Result<Child>, pause: Duration) -> u8 {
    let mut starts: Vec<Instant> = Vec::new();
    loop {
        let mut child = match spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("linksyd: can't start the service: {e}");
                return 1;
            }
        };
        let status = match child.wait() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("linksyd: lost track of the service: {e}");
                return 1;
            }
        };
        let why = match outcome(status) {
            Outcome::Done(code) => return code,
            Outcome::Crashed(why) => why,
        };
        let now = Instant::now();
        starts.retain(|t| now.duration_since(*t) < WINDOW);
        starts.push(now);
        if starts.len() > BURST {
            eprintln!("linksyd: the service keeps stopping ({why}); giving up");
            return 1;
        }
        eprintln!("linksyd: the service stopped ({why}); starting it again");
        std::thread::sleep(pause);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn shell(script: &str) -> std::io::Result<Child> {
        Command::new("sh").arg("-c").arg(script).spawn()
    }

    #[test]
    fn a_service_that_quits_is_not_started_again() {
        let starts = Cell::new(0);
        let code = watch(
            || {
                starts.set(starts.get() + 1);
                shell("exit 0")
            },
            Duration::ZERO,
        );
        assert_eq!((code, starts.get()), (0, 1));
    }

    #[test]
    fn bad_options_are_not_tried_again() {
        let starts = Cell::new(0);
        let code = watch(
            || {
                starts.set(starts.get() + 1);
                shell("exit 2")
            },
            Duration::ZERO,
        );
        assert_eq!((code, starts.get()), (2, 1));
    }

    #[test]
    fn a_kill_starts_it_again_and_a_stop_ends_it() {
        let starts = Cell::new(0);
        let code = watch(
            || {
                starts.set(starts.get() + 1);
                match starts.get() {
                    1 => shell("kill -9 $$"),
                    2 => shell("exit 7"),
                    3 => shell("kill -15 $$"),
                    _ => unreachable!(),
                }
            },
            Duration::ZERO,
        );
        assert_eq!((code, starts.get()), (0, 3));
    }

    #[test]
    fn a_service_that_always_crashes_is_given_up_on() {
        let starts = Cell::new(0);
        let code = watch(
            || {
                starts.set(starts.get() + 1);
                shell("exit 1")
            },
            Duration::ZERO,
        );
        assert_eq!((code, starts.get()), (1, BURST + 1));
    }

    #[test]
    fn a_service_that_cannot_be_started_ends_it() {
        assert_eq!(watch(|| Command::new("/nonexistent/linksyd").spawn(), Duration::ZERO), 1);
    }
}
