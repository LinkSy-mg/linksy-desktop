//! Upgrading Linksy in one step, for `linksy upgrade` and the apps' Update Now: find the
//! newest version (the website's service information), how Linksy was installed and which
//! apps are, download exactly those files from the release, check them against its
//! SHA256SUMS, install them (packages as root through `linksy-system-setup`, one password;
//! the tarball into the home folder; the Flatpak with the computer's own flatpak), and
//! restart the background service.

use std::path::{Path, PathBuf};
use std::time::Duration;

use linksy_i18n::t;
use sha2::{Digest, Sha256};

/// Where the newest version is announced (docs/protocol.md §4.12); LINKSY_SERVICE_URL
/// overrides it, as for the service.
const SERVICE_URL: &str = "https://linksy.diary.mg/linksy-relay";
/// The releases: tagged `linux-vX.Y.Z`, with every file and SHA256SUMS. LINKSY_RELEASES
/// overrides it, for tests.
const RELEASES: &str = "https://github.com/LinkSy-mg/linksy-releases/releases/download";

/// This build: version, commit and date (`linksy --revision`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("LINKSY_COMMIT");
pub const BUILD_DATE: &str = env!("LINKSY_BUILD_DATE");

/// How Linksy is installed here, which decides how it upgrades.
#[derive(Clone, Debug, PartialEq)]
pub enum Method {
    /// Fedora's .rpm
    Rpm,
    /// openSUSE's .rpm
    Suse,
    Deb,
    /// The tarball's install.sh, into this prefix (~/.local)
    Tarball(PathBuf),
    Flatpak,
    /// Arch, from the PKGBUILD (the AUR)
    Arch,
    /// Run from a build, or something this doesn't know
    Other,
}

impl Method {
    /// How it's called in `linksy --revision`.
    pub fn name(&self) -> &'static str {
        match self {
            Method::Rpm => "rpm",
            Method::Suse => "rpm (openSUSE)",
            Method::Deb => "deb",
            Method::Tarball(_) => "tarball",
            Method::Flatpak => "Flatpak",
            Method::Arch => "Arch package",
            Method::Other => "a build",
        }
    }
}

fn quiet(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The installed `linksyd`: the one next to this program, else on PATH.
fn service_binary() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.parent()?.join("linksyd");
    if beside.is_file() {
        return Some(beside);
    }
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("linksyd")).find(|p| p.is_file()))
}

/// How Linksy was installed here.
pub fn method() -> Method {
    if std::env::var_os("FLATPAK_ID").is_some() {
        return Method::Flatpak;
    }
    let Some(bin) = service_binary() else { return Method::Other };
    let path = bin.to_string_lossy().into_owned();
    // The tarball installs into ~/.local (its prefix is two levels up from bin/linksyd)
    if let Some(home) = std::env::var_os("HOME")
        && bin.starts_with(Path::new(&home))
    {
        return match bin.parent().and_then(Path::parent) {
            Some(prefix) if prefix.join("share/linksy").is_dir() || prefix.ends_with(".local") => Method::Tarball(prefix.to_path_buf()),
            _ => Method::Other,
        };
    }
    if quiet("rpm", &["-qf", &path]).is_some() {
        let suse = std::fs::read_to_string("/etc/os-release").is_ok_and(|s| s.to_lowercase().contains("opensuse"));
        return if suse { Method::Suse } else { Method::Rpm };
    }
    if quiet("dpkg", &["-S", &path]).is_some() {
        return Method::Deb;
    }
    if quiet("pacman", &["-Qo", &path]).is_some() {
        return Method::Arch;
    }
    Method::Other
}

/// The apps installed as packages: `linksy` (GTK) and/or `linksy-qt`. Before 0.8.4, the
/// one `linksy` package held everything; it upgrades to `linksy-common` and `linksy`.
fn installed_apps(method: &Method) -> Vec<&'static str> {
    let has = |pkg: &str| match method {
        Method::Rpm | Method::Suse => quiet("rpm", &["-q", pkg]).is_some(),
        Method::Deb => quiet("dpkg-query", &["-W", "-f=${Status}", pkg]).is_some_and(|s| s.contains("install ok installed")),
        _ => false,
    };
    let apps: Vec<&'static str> = ["linksy", "linksy-qt"].into_iter().filter(|p| has(p)).collect();
    if apps.is_empty() { vec!["linksy"] } else { apps }
}

/// What an upgrade does: from which version to which, how, and the files.
#[derive(Clone, Debug)]
pub struct Plan {
    pub current: String,
    pub latest: String,
    /// The release notes.
    pub notes: Option<String>,
    pub method: Method,
    pub files: Vec<String>,
}

/// Whether version `a` is newer than `b` ("0.10.0" after "0.9.3").
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .user_agent(format!("Linksy/{VERSION} (Linux)"))
        .build()
        .into()
}

/// The newest Linux version and its notes, from the website.
pub fn latest() -> Result<(String, Option<String>), String> {
    let url = std::env::var("LINKSY_SERVICE_URL").unwrap_or_else(|_| SERVICE_URL.into());
    let mut response = agent().get(&url).call().map_err(|e| t!("Couldn't reach linksy.diary.mg: {error}", error = e))?;
    let body = response.body_mut().with_config().limit(64 * 1024).read_to_string().map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let linux = &json["latest"]["linux"];
    let version = linux["version"].as_str().ok_or_else(|| t!("The website didn't say which version is newest.").to_string())?;
    Ok((version.to_string(), linux["notes"].as_str().map(str::to_string)))
}

/// This version; tests can pretend to be an older one (LINKSY_UPGRADE_FROM).
fn current() -> String {
    std::env::var("LINKSY_UPGRADE_FROM").unwrap_or_else(|_| VERSION.into())
}

/// What upgrading to the newest version would take, or None when this is the newest.
pub fn check() -> Result<Option<Plan>, String> {
    let (latest, notes) = latest()?;
    if !newer(&latest, &current()) {
        return Ok(None);
    }
    let method = method();
    let v = &latest;
    let files = match &method {
        Method::Rpm | Method::Suse | Method::Deb => {
            let name = |pkg: &str| match method {
                Method::Rpm => format!("{pkg}-{v}-1.x86_64.rpm"),
                Method::Suse => format!("{pkg}-{v}-1.suse.x86_64.rpm"),
                _ => format!("{pkg}_{v}_amd64.deb"),
            };
            std::iter::once("linksy-common").chain(installed_apps(&method)).map(name).collect()
        }
        Method::Tarball(_) => vec![format!("linksy-{v}-linux-x86_64.tar.gz")],
        Method::Flatpak => vec![format!("linksy-{v}.flatpak")],
        Method::Arch | Method::Other => Vec::new(),
    };
    Ok(Some(Plan { current: current(), latest: latest.clone(), notes, method, files }))
}

/// Why this install can't upgrade itself, and what to do instead; None when it can.
pub fn manual(plan: &Plan) -> Option<String> {
    match plan.method {
        Method::Arch => Some(t!("Rebuild the Linksy package from its PKGBUILD for {version}.", version = plan.latest)),
        Method::Other => Some(t!("This Linksy wasn't installed from a package. Install {version} with: curl -fsSL https://linksy.diary.mg/install.sh | sh", version = plan.latest)),
        _ => None,
    }
}

/// How the root step asks for the password: sudo in a terminal, pkexec in the apps (with
/// the app's own prompt where no polkit agent runs).
pub enum Root {
    Sudo,
    Pkexec(linksy_host::pkexec::Ask),
}

/// Where downloads wait before installing.
fn download_dir(version: &str) -> PathBuf {
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(std::env::temp_dir);
    cache.join("linksy").join(format!("upgrade-{version}"))
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    let mut response = agent().get(url).call().map_err(|e| t!("Couldn't download {url}: {error}", url, error = e))?;
    let mut reader = response.body_mut().with_config().limit(512 * 1024 * 1024).reader();
    let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader, &mut file).map_err(|e| t!("Couldn't download {url}: {error}", url, error = e))?;
    Ok(())
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The root helper (`linksy-system-setup`): with the package, or next to this program.
fn setup_script() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [dir.join("linksy-system-setup"), PathBuf::from("/usr/libexec/linksy/linksy-system-setup"), PathBuf::from("/usr/lib/linksy/linksy-system-setup")].into_iter().find(|p| p.is_file())
}

/// Upgrade: download, check, install, restart the service. `step` hears each step, in the
/// language in use.
pub fn run(plan: &Plan, root: Root, mut step: impl FnMut(String)) -> Result<(), String> {
    if let Some(why) = manual(plan) {
        return Err(why);
    }
    let dir = download_dir(&plan.latest);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let base = format!("{}/linux-v{}", std::env::var("LINKSY_RELEASES").unwrap_or_else(|_| RELEASES.into()), plan.latest);

    // The release's checksums first: nothing is installed that doesn't match them
    step(t!("Checking the release").into());
    let sums_path = dir.join("SHA256SUMS");
    download(&format!("{base}/SHA256SUMS"), &sums_path)?;
    let sums = std::fs::read_to_string(&sums_path).map_err(|e| e.to_string())?;
    let expected = |name: &str| sums.lines().find_map(|l| l.split_once("  ").filter(|(_, n)| n.trim() == name).map(|(h, _)| h.trim().to_string()));

    let mut paths = Vec::new();
    for name in &plan.files {
        step(t!("Downloading {name}", name));
        let path = dir.join(name);
        download(&format!("{base}/{name}"), &path)?;
        let want = expected(name).ok_or_else(|| t!("{name} isn't in the release's checksums: not installing it.", name))?;
        if sha256(&path)? != want {
            let _ = std::fs::remove_file(&path);
            return Err(t!("{name} doesn't match the release's checksum: not installing it. Try again later.", name));
        }
        paths.push(path);
    }

    // Tests: everything up to here, nothing installed (LINKSY_DRY_RUN)
    if std::env::var_os("LINKSY_DRY_RUN").is_some() {
        step(format!("Would install: {}", paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" ")));
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    }
    step(t!("Installing (your computer asks for your password)").into());
    match &plan.method {
        Method::Tarball(_) => install_tarball(&dir, &paths[0])?,
        Method::Flatpak => {
            install_flatpak(&paths[0])?;
            // The apps start the new version's service when they start again
            let _ = std::fs::remove_dir_all(&dir);
            return Ok(());
        }
        _ => install_packages(&paths, root)?,
    }

    step(t!("Restarting Linksy's service").into());
    let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    let _ = std::process::Command::new("systemctl").args(["--user", "try-restart", "linksyd.service"]).status();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Packages, as root, through the setup helper (it installs only Linksy's own files).
fn install_packages(paths: &[PathBuf], root: Root) -> Result<(), String> {
    let script = setup_script().ok_or_else(|| t!("Linksy's setup helper is missing; install Linksy again.").to_string())?;
    let mut args: Vec<String> = paths.iter().map(|p| format!("upgrade:{}", p.display())).collect();
    let (cmd, mut argv) = match root {
        Root::Sudo if nix_root() => (script.display().to_string(), Vec::new()),
        Root::Sudo => ("sudo".to_string(), vec![script.display().to_string()]),
        Root::Pkexec(ask) => {
            let argv: Vec<std::ffi::OsString> = std::iter::once(script.into_os_string()).chain(args.into_iter().map(Into::into)).collect();
            let outcome = linksy_host::pkexec::run(&argv, ask, |_| {}).map_err(|e| e.to_string())?;
            return match outcome.code {
                0 => Ok(()),
                _ if outcome.timed_out => Err(t!("Linksy stopped waiting for the password after two minutes. If no password window came up, update in a terminal: linksy upgrade").into()),
                _ if outcome.wrong => Err(t!("Not updated: the password didn't work.").into()),
                126 | 127 => Err(t!("Not updated: the password wasn't given.").into()),
                _ => Err(t!("The update didn't install.").into()),
            };
        }
    };
    argv.append(&mut args);
    let status = std::process::Command::new(&cmd).args(&argv).status().map_err(|e| e.to_string())?;
    match status.code() {
        Some(0) => Ok(()),
        _ => Err(t!("The update didn't install.").into()),
    }
}

/// The new Flatpak, with the computer's own flatpak, into the installation this one is in
/// (the user's or the system's, which asks for the password). Running copies go on until
/// they end.
fn install_flatpak(bundle: &Path) -> Result<(), String> {
    let info = std::fs::read_to_string("/.flatpak-info").unwrap_or_default();
    let scope = if info.contains("/.local/share/flatpak/") { "--user" } else { "--system" };
    let ok = linksy_host::command("flatpak", &[])
        .args(["install", "--noninteractive", "--reinstall", scope, "--bundle"])
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok { Ok(()) } else { Err(t!("The update didn't install.").into()) }
}

fn nix_root() -> bool {
    quiet("id", &["-u"]).is_some_and(|u| u.trim() == "0")
}

/// The tarball: unpack it and run its installer, into the home folder (no password).
fn install_tarball(dir: &Path, tarball: &Path) -> Result<(), String> {
    let ok = std::process::Command::new("tar").arg("xzf").arg(tarball).arg("-C").arg(dir).status().is_ok_and(|s| s.success());
    if !ok {
        return Err(t!("Couldn't unpack the update.").into());
    }
    let name = tarball.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".tar.gz")).unwrap_or_default();
    let ok = std::process::Command::new(dir.join(name).join("install.sh")).status().is_ok_and(|s| s.success());
    if ok { Ok(()) } else { Err(t!("The update didn't install.").into()) }
}

/// `linksy --revision`: version, commit, build date and how it's installed.
/// Start this app again once this process has ended (after an update, or to restart it).
/// The update replaced the program while it ran, so the kernel names it "… (deleted)":
/// the path without that, or the program's name on PATH.
pub fn relaunch() {
    // The Flatpak: started by the computer once every copy of the old one has ended (the
    // sandbox goes with this process, so the wait can't happen inside it)
    if let Ok(app) = std::env::var("FLATPAK_ID") {
        let script = "for _ in $(seq 50); do flatpak ps --columns=application | grep -qx \"$0\" || break; sleep 0.2; done; exec flatpak run \"$0\"";
        let _ = linksy_host::detached("sh", &[]).args(["-c", script]).arg(app).stdin(std::process::Stdio::null()).spawn();
        return;
    }
    let exe =std::env::current_exe().ok().map(|p| p.to_string_lossy().trim_end_matches(" (deleted)").to_string());
    let name = std::env::args().next().unwrap_or_default();
    let program = exe.filter(|p| std::path::Path::new(p).is_file()).unwrap_or(name);
    // Not before this process is gone: a single-instance app would hand the start back to it
    let script = "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; exec \"$0\"";
    let _ = std::process::Command::new("sh")
        .args(["-c", script])
        .arg(program)
        .arg(std::process::id().to_string())
        .stdin(std::process::Stdio::null())
        .spawn();
}

pub fn revision() -> String {
    format!("{VERSION} ({COMMIT}, built {BUILD_DATE}, {})", method().name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.10.0", "0.9.3"));
        assert!(newer("0.8.6", "0.8.5"));
        assert!(!newer("0.8.5", "0.8.5"));
    }

    #[test]
    fn checksums_are_checked() {
        let dir = std::env::temp_dir().join(format!("linksy-upgrade-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x");
        std::fs::write(&f, b"linksy").unwrap();
        assert_eq!(sha256(&f).unwrap(), "f5b9edc6b6cb09984c4f8d6177c9475ecc6899291bcaffb099a58df1af81ebce");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
