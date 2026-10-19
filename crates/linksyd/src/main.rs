//! linksyd: the always-on Linksy service. Keeps paired devices connected, receives files,
//! and shows native notifications. Front ends (the app, the `linksy` CLI) talk to it over
//! a Unix socket and can come and go.

mod clipboard;
mod history;
mod ipc;
mod sessions;
// Accept/Decline buttons need the freedesktop notification spec; other platforms get
// their own native notifications with their apps.
#[cfg(target_os = "linux")]
mod notify;
mod system;

use std::path::PathBuf;
use std::process::ExitCode;

use linksy_core::{Config, Node};
use tracing::{error, info};

/// Fixed default port so a peer's last known address stays valid across restarts.
const DEFAULT_PORT: u16 = 47_200;

struct Args {
    config: Config,
    socket: PathBuf,
    notifications: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut config = Config::new(system::data_dir(), system::download_dir(), system::device_name());
    config.kind = system::device_kind();
    config.port = DEFAULT_PORT;
    let mut socket = linksy_ipc::socket_path();
    let mut notifications = true;

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
                     --no-notifications   don't show desktop notifications"
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
    Ok(Args { config, socket, notifications })
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
    let history = history::History::load(data_dir.join("history.json"));
    tokio::spawn(history.clone().run(node.clone()));
    let socket = args.socket.clone();
    let sessions = sessions::Sessions::new(data_dir.clone());
    tokio::spawn(sessions.clone().run(node.clone()));
    let server = tokio::spawn(ipc::serve(node.clone(), history, sessions, socket.clone()));

    tokio::select! {
        r = server => if let Ok(Err(e)) = r { error!("IPC server stopped: {e}") },
        _ = shutdown_signal() => info!("shutting down"),
    }
    node.shutdown().await;
    let _ = std::fs::remove_file(&socket);
    ExitCode::SUCCESS
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
