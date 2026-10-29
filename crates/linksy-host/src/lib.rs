//! Programs and files on the computer itself. Linksy's Flatpak has host access
//! (`org.freedesktop.Flatpak`): inside it, programs run outside the sandbox through
//! `flatpak-spawn --host`, and files in the home folder are read and written by them.
//! Elsewhere everything simply runs here.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Whether this is Linksy's Flatpak.
pub fn flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
}

/// `program` on the computer, with `env` set for it (the sandbox's own environment stays
/// inside). In the Flatpak it ends when this process does.
pub fn command(program: impl AsRef<OsStr>, env: &[(&str, &str)]) -> Command {
    spawn_command(program, env, true)
}

/// Like [`command`], but it keeps running after this process ends (a password prompt
/// started for the session, say).
pub fn detached(program: impl AsRef<OsStr>, env: &[(&str, &str)]) -> Command {
    spawn_command(program, env, false)
}

fn spawn_command(program: impl AsRef<OsStr>, env: &[(&str, &str)], watch: bool) -> Command {
    if !flatpak() {
        let mut c = Command::new(program);
        c.envs(env.iter().copied());
        return c;
    }
    // The session's display and desktop, which programs started from the computer's
    // background don't always have (Hyprland, Sway). Not DISPLAY: the sandbox renumbers it.
    let session: Vec<(&str, String)> = SESSION.iter().filter_map(|k| std::env::var(k).ok().filter(|_| !env.iter().any(|(e, _)| e == k)).map(|v| (*k, v))).collect();
    let all: Vec<(&str, &str)> = session.iter().map(|(k, v)| (*k, v.as_str())).chain(env.iter().copied()).collect();
    let mut c = Command::new("flatpak-spawn");
    c.args(spawn_args(&all, watch)).arg(program);
    c
}

const SESSION: [&str; 6] = ["WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE", "HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK", "NIRI_SOCKET"];

fn spawn_args(env: &[(&str, &str)], watch: bool) -> Vec<String> {
    let mut args = vec!["--host".to_string()];
    if watch {
        args.push("--watch-bus".into());
    }
    args.extend(env.iter().map(|(k, v)| format!("--env={k}={v}")));
    args
}

/// Where one of this app's files is on the computer: the Flatpak's `/app/…` is under its
/// install folder there.
pub fn path(inside: &Path) -> PathBuf {
    if !flatpak() {
        return inside.to_path_buf();
    }
    let info = std::fs::read_to_string("/.flatpak-info").unwrap_or_default();
    match (app_path(&info), inside.strip_prefix("/app")) {
        (Some(app), Ok(rest)) => app.join(rest),
        _ => inside.to_path_buf(),
    }
}

fn app_path(info: &str) -> Option<PathBuf> {
    info.lines().find_map(|l| l.strip_prefix("app-path=")).map(|p| PathBuf::from(p.trim()))
}

/// The computer's config folder (`~/.config`), not the Flatpak's own.
pub fn config_dir() -> PathBuf {
    xdg_dir(if flatpak() { "HOST_XDG_CONFIG_HOME" } else { "XDG_CONFIG_HOME" }, ".config")
}

/// The computer's data folder (`~/.local/share`), not the Flatpak's own.
pub fn data_dir() -> PathBuf {
    xdg_dir(if flatpak() { "HOST_XDG_DATA_HOME" } else { "XDG_DATA_HOME" }, ".local/share")
}

fn xdg_dir(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(default))
}

/// Whether `path` exists on the computer.
pub fn exists(path: &Path) -> bool {
    if !flatpak() {
        return path.exists();
    }
    quiet(command("test", &[]).arg("-e").arg(path))
}

/// Whether `program` is installed on the computer (in its PATH).
pub fn installed(program: &str) -> bool {
    if !flatpak() {
        return std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()));
    }
    quiet(command("sh", &[]).args(["-c", "command -v \"$1\"", "sh", program]))
}

/// A file on the computer, or None.
pub fn read(path: &Path) -> Option<String> {
    if !flatpak() {
        return std::fs::read_to_string(path).ok();
    }
    let out = command("cat", &[]).arg(path).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Write a file on the computer, making its folder first.
pub fn write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if !flatpak() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        return std::fs::write(path, contents);
    }
    let mut child = command("sh", &[])
        .args(["-c", "mkdir -p \"$(dirname \"$1\")\" && cat > \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child.stdin.take().map(|mut i| i.write_all(contents)).transpose()?;
    if child.wait()?.success() { Ok(()) } else { Err(std::io::Error::other(format!("couldn't write {}", path.display()))) }
}

/// Remove a file on the computer, if it's there.
pub fn remove(path: &Path) {
    if !flatpak() {
        let _ = std::fs::remove_file(path);
        return;
    }
    quiet(command("rm", &[]).arg("-f").arg(path));
}

/// The command a desktop entry or script on the computer uses to run one of this app's
/// programs: in the Flatpak, `flatpak run --command=NAME APP`.
pub fn launcher(program: &str) -> String {
    match std::env::var("FLATPAK_ID") {
        Ok(app) => format!("flatpak run --command={program} {app}"),
        Err(_) => program.to_string(),
    }
}

fn quiet(cmd: &mut Command) -> bool {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_args_pass_the_environment_to_the_host() {
        assert_eq!(spawn_args(&[("TERM", "xterm-256color")], true), ["--host", "--watch-bus", "--env=TERM=xterm-256color"]);
        assert_eq!(spawn_args(&[], false), ["--host"]);
    }

    #[test]
    fn app_path_from_flatpak_info() {
        let info = "[Application]\nname=mg.diary.linksy.Linksy\n\n[Instance]\napp-path=/var/lib/flatpak/app/mg.diary.linksy.Linksy/x86_64/stable/abc/files\nbranch=stable\n";
        assert_eq!(app_path(info), Some(PathBuf::from("/var/lib/flatpak/app/mg.diary.linksy.Linksy/x86_64/stable/abc/files")));
        assert_eq!(app_path("[Application]\n"), None);
    }
}
