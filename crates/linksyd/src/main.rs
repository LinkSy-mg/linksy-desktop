//! linksyd: the always-on Linksy service. Keeps paired devices connected, receives files,
//! and shows native notifications. Front ends (the app, the `linksy` CLI) talk to it over
//! a Unix socket and can come and go.

mod clipboard;
mod history;
mod ipc;
mod links;
mod lock;
mod mirror;
mod photos;
mod pictures;
mod update;
mod sessions;
#[cfg(target_os = "linux")]
mod terminal;
// Accept/Decline buttons need the freedesktop notification spec; other platforms get
// their own native notifications with their apps.
#[cfg(target_os = "linux")]
mod dav;
#[cfg(target_os = "linux")]
mod media;
#[cfg(target_os = "linux")]
mod notify;
mod system;
#[cfg(target_os = "linux")]
mod usb;

use std::path::PathBuf;
use std::process::ExitCode;

use linksy_core::{Config, Node};
use tracing::{error, info};

/// Asked to stop (`linksy_ipc::Request::Quit`).
pub static QUIT: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// What a Linux computer running Linksy can do for other devices.
const FEATURES: &[&str] = &[
    "files", "clipboard", "open_url", "ring", "battery", "media_state", "media_remote", "call_state", "heartbeat",
    "import", "fs", "input", "screen", "control", "display", "camera", "watch", "notifications", "sms",
    "clipboard_file", "microphone", "picture",
];

/// Fixed default port so a peer's last known address stays valid across restarts.
const DEFAULT_PORT: u16 = 47_200;

struct Args {
    config: Config,
    socket: PathBuf,
    notifications: bool,
    open_links: bool,
    usb: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut config = Config::new(system::data_dir(), system::download_dir(), system::device_name());
    config.kind = system::device_kind();
    // Everything this computer does for its devices (docs/protocol.md §4.10).
    config.features = FEATURES.iter().map(|f| f.to_string()).collect();
    if clipboard::can_watch() {
        config.features.push("clipboard_watch".into());
    }
    if cfg!(target_os = "linux") && std::env::var_os("FLATPAK_ID").is_none() {
        config.features.push("terminal".into());
    }
    config.port = DEFAULT_PORT;
    let mut socket = linksy_ipc::socket_path();
    let mut notifications = true;
    let mut open_links = true;
    let mut usb = true;
    let mut away = true;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--name" => config.name = value("--name")?,
            "--port" => config.port = value("--port")?.parse().map_err(|_| "--port needs a number")?,
            "--data-dir" => config.data_dir = value("--data-dir")?.into(),
            "--download-dir" => config.download_dir = value("--download-dir")?.into(),
            "--socket" => socket = value("--socket")?.into(),
            "--no-discovery" => config.discovery = false,
            "--stay-connected" => config.stay_connected = true,
            "--no-notifications" => notifications = false,
            "--no-open-links" => open_links = false,
            "--no-usb" => usb = false,
            "--no-away" => away = false,
            "-h" | "--help" => {
                println!(
                    "linksyd — Linksy background service\n\n\
                     Options:\n  --name NAME          device name shown to others\n  \
                     --port PORT          UDP port (default {DEFAULT_PORT})\n  \
                     --data-dir DIR       identity and paired devices\n  \
                     --download-dir DIR   where received files go\n  \
                     --socket PATH        IPC socket\n  \
                     --no-discovery       don't advertise or browse the network\n  \
                     --stay-connected     keep paired devices connected (default: on demand)\n  \
                     --no-notifications   don't show desktop notifications\n  \
                     --no-open-links      don't open links devices send (they're still in the history)\n  \
                     --no-usb             don't connect phones over USB cables\n  \
                     --no-away            don't reach devices away from home (over the internet)"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    // Settings are saved as absolute paths; resolve relative arguments against the cwd.
    for dir in [&mut config.data_dir, &mut config.download_dir] {
        *dir = std::path::absolute(&*dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Away from home, through the relay the website lists (docs/protocol.md §4.13).
    config.away = away;
    config.relay = Some(update::relay(&config.data_dir.join("update.json")));
    Ok(Args { config, socket, notifications, open_links, usb })
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> ExitCode {
    let level = std::env::var("LINKSY_LOG").ok().and_then(|l| l.parse().ok()).unwrap_or(tracing::Level::INFO);
    tracing_subscriber::fmt().with_max_level(level).with_target(false).init();

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("linksyd: {e}");
            return ExitCode::from(2);
        }
    };

    // One service per user: if the socket answers, another linksyd is running.
    if tokio::net::UnixStream::connect(&args.socket).await.is_ok() {
        eprintln!("linksyd is already running");
        return ExitCode::SUCCESS;
    }
    let _ = std::fs::remove_file(&args.socket);

    let mut config = args.config;
    let data_dir = config.data_dir.clone();
    let node = match Node::start(config.clone()).await {
        Ok(n) => n,
        // The fixed port is taken (another app, or a second user): fall back to any port.
        Err(e) if config.port != 0 => {
            info!("port {} unavailable ({e}), using a random port", config.port);
            config.port = 0;
            match Node::start(config).await {
                Ok(n) => n,
                Err(e) => {
                    error!("could not start: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        Err(e) => {
            error!("could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    info!("{} ({}) ready, saving files to {}", node.info().name, node.info().id.short(), node.settings().download_dir.display());

    #[cfg(target_os = "linux")]
    if args.notifications {
        tokio::spawn(notify::run(node.clone()));
    }
    tokio::spawn(clipboard::run(node.clone(), args.notifications));
    if args.open_links {
        tokio::spawn(links::run(node.clone()));
    }
    let history = history::History::load(data_dir.join("history.json"));
    tokio::spawn(history.clone().run(node.clone()));
    let socket = args.socket.clone();
    let sessions = sessions::Sessions::new(data_dir.clone(), socket.clone());
    tokio::spawn(sessions.clone().run(node.clone()));
    tokio::spawn(sessions.clone().watch_cameras(node.clone()));
    tokio::spawn(pictures::run(node.clone(), data_dir.clone()));
    let updates = update::UPDATES.get_or_init(|| update::Updates::load(data_dir.join("update.json"), args.notifications)).clone();
    tokio::spawn(updates.run());
    let icons = linksy_ipc::runtime_dir().unwrap_or_else(std::env::temp_dir).join("linksy-icons");
    #[cfg(target_os = "linux")]
    tokio::spawn(media::run(node.clone(), icons.join("art")));
    let mirror = mirror::Mirror::load(data_dir.join("notifications.json"), icons, socket.clone());
    if args.notifications {
        tokio::spawn(mirror.clone().run(node.clone()));
    }
    #[cfg(target_os = "linux")]
    let _ = dav::BRIDGES.set(dav::Bridges::start(data_dir.join("files.json"), node.clone()).await);
    #[cfg(target_os = "linux")]
    if args.usb {
        tokio::spawn(usb::Usb::new(node.clone(), data_dir.clone(), args.notifications).run());
    }
    // Terminals for phones; not from a Flatpak, whose shell would be the sandbox's.
    #[cfg(target_os = "linux")]
    if std::env::var_os("FLATPAK_ID").is_none() {
        let terminals = terminal::Terminals::load(&data_dir);
        let _ = terminal::TERMINALS.set(terminals.clone());
        tokio::spawn(terminals.run(node.clone()));
    }
    let photos = photos::Photos::load(data_dir.join("photos.json"), &node);
    let away = lock::Away::load(data_dir.join("lock.json"));
    tokio::spawn(away.clone().run(node.clone(), history.clone()));
    let server = tokio::spawn(ipc::serve(node.clone(), history, sessions, mirror, away, photos, socket.clone()));

    let mut updated = None;
    tokio::select! {
        r = server => if let Ok(Err(e)) = r { error!("IPC server stopped: {e}") },
        _ = shutdown_signal() => info!("shutting down"),
        _ = QUIT.notified() => info!("asked to quit"),
        exe = replaced() => {
            info!("Linksy was updated, restarting");
            updated = Some(exe);
        }
    }
    // Shells don't outlive the service that runs them.
    #[cfg(target_os = "linux")]
    if let Some(t) = terminal::TERMINALS.get() {
        t.end_all();
    }
    node.shutdown().await;
    let _ = std::fs::remove_file(&socket);
    if let Some(exe) = updated {
        use std::os::unix::process::CommandExt;
        let e = std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec();
        error!("could not start the new version: {e}");
        return ExitCode::FAILURE;
    }
    // Notifications still on screen each have a thread waiting for a click, which would
    // keep the runtime (and so the process) alive; everything that matters is done.
    std::process::exit(0)
}

/// Resolves with this program's path once a package update has replaced it (the running
/// one then shows as deleted), so the new version takes over without a log-out.
async fn replaced() -> PathBuf {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let Ok(exe) = std::fs::read_link("/proc/self/exe") else { continue };
        if let Some(path) = exe.to_str().and_then(|p| p.strip_suffix(" (deleted)"))
            && std::fs::metadata(path).is_ok()
        {
            return path.into();
        }
    }
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
