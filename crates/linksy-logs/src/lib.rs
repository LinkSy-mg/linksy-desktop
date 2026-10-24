//! Problem logs: a crash file when a Linksy program panics, and one `logs.txt` that says
//! what happened, ready to attach to a bug report. Nothing is sent anywhere; the person
//! decides whether to share the file. Their home folder and user name are left out.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Crash files kept, newest first; older ones are removed.
const KEEP: usize = 10;
/// Log lines gathered from the journal.
const LINES: &str = "3000";
/// The programs whose log is gathered (the journal's `_COMM`).
const PROGRAMS: [&str; 4] = ["linksyd", "linksy-app", "linksy-qt", "linksy-screen"];

/// Where crash files go: `~/.local/state/linksy/crashes`.
pub fn crash_dir() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    let state = state.unwrap_or_else(|| home().join(".local/state"));
    state.join("linksy/crashes")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Write a crash file when `program` panics, then let the panic go on as before.
pub fn catch_crashes(program: &'static str, version: &'static str) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let text = format!("{program} {version} crashed at {} (unix time)\nthread: {thread}\n{info}\n\n{backtrace}\n", now());
        let _ = write_crash(&crash_dir(), program, &text);
        previous(info);
    }));
}

fn write_crash(dir: &Path, program: &str, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("{}-{program}.txt", now())), redact(text))?;
    for old in crashes(dir).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

/// Crash files, newest first.
fn crashes(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect())
        .unwrap_or_default();
    // Named after the time they happened
    files.sort_by_key(|p| std::cmp::Reverse(crash_time(p)));
    files
}

fn crash_time(path: &Path) -> u64 {
    path.file_name().and_then(|n| n.to_str()).and_then(|n| n.split('-').next()).and_then(|t| t.parse().ok()).unwrap_or(0)
}

/// When the newest crash happened (unix time), if any: an app offers the logs once after.
pub fn last_crash() -> Option<u64> {
    crashes(&crash_dir()).first().map(|p| crash_time(p))
}

/// Whether a program crashed since this was last asked: the app offers the logs once.
pub fn unseen_crash() -> bool {
    let Some(last) = last_crash() else { return false };
    let marker = crash_dir().with_file_name("crashes-seen");
    let seen: u64 = std::fs::read_to_string(&marker).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    if last <= seen {
        return false;
    }
    let _ = std::fs::write(marker, last.to_string());
    true
}

/// The whole `logs.txt`: versions, crash files and the recent log of Linksy's programs.
/// `revision`: Linksy's version, commit and install method. Takes a moment (it reads the
/// journal), so call it off the main thread.
pub fn collect(revision: &str) -> String {
    let mut out = String::from("Linksy logs\n===========\n\n");
    let os = std::fs::read_to_string("/etc/os-release").ok().and_then(|s| {
        s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string())
    });
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| "unknown".into());
    let _ = writeln!(out, "Linksy:     {revision}");
    let _ = writeln!(out, "System:     {}", os.as_deref().unwrap_or("unknown"));
    let _ = writeln!(out, "Kernel:     {}", std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim());
    let _ = writeln!(out, "Desktop:    {} ({})", env("XDG_CURRENT_DESKTOP"), env("XDG_SESSION_TYPE"));
    let _ = writeln!(out, "GStreamer:  {}", run("gst-inspect-1.0", &["--version"]).and_then(|v| v.lines().find(|l| l.starts_with("GStreamer")).map(String::from)).unwrap_or_else(|| "not found".into()));
    let _ = writeln!(out, "Service:    {}", run("systemctl", &["--user", "is-active", "linksyd"]).map(|s| s.trim().to_string()).unwrap_or_else(|| "unknown".into()));
    let _ = writeln!(out, "Written at: {} (unix time)", now());

    let files = crashes(&crash_dir());
    let _ = writeln!(out, "\nCrashes ({})\n-----------", files.len());
    for f in files.iter().take(5) {
        let _ = writeln!(out, "\n{}", std::fs::read_to_string(f).unwrap_or_default().trim_end());
    }

    let _ = writeln!(out, "\nLog\n---\n");
    let mut args = vec!["--user", "--no-pager", "-o", "short-iso", "-n", LINES];
    let matches: Vec<String> = PROGRAMS.iter().map(|p| format!("_COMM={p}")).collect();
    args.extend(matches.iter().map(String::as_str));
    match run("journalctl", &args) {
        Some(log) if !log.trim().is_empty() => out.push_str(&log),
        _ => out.push_str("(The log can't be read from here. In a terminal: journalctl --user -u linksyd -n 3000)\n"),
    }
    redact(&out)
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Leave out the home folder and the user name.
fn redact(text: &str) -> String {
    let mut text = text.to_string();
    let home = home();
    if let Some(home) = home.to_str().filter(|h| h.len() > 1) {
        text = text.replace(home, "~");
    }
    if let Ok(user) = std::env::var("USER")
        && user.len() > 2
    {
        text = text.replace(&user, "user");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_crashes_only() {
        let dir = tempfile::tempdir().unwrap();
        for t in 0..KEEP as u64 + 3 {
            std::fs::write(dir.path().join(format!("{}-linksyd.txt", 1000 + t)), "boom").unwrap();
        }
        write_crash(dir.path(), "linksy-app", "boom").unwrap();
        let left = crashes(dir.path());
        assert_eq!(left.len(), KEEP);
        assert!(left[0].to_string_lossy().ends_with("-linksy-app.txt"));
        assert_eq!(crash_time(&left[KEEP - 1]), 1000 + 4);
    }

    #[test]
    fn leaves_out_home_and_user() {
        let home = home();
        let text = format!("saving to {}/Downloads", home.display());
        assert!(!redact(&text).contains(home.to_str().unwrap()) || home.as_os_str().len() <= 1);
    }
}
