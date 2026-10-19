//! `linksy`: command-line front end for the Linksy service.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use linksy_ipc::linksy_core::{DeviceKind, Direction, Event};
use linksy_ipc::{Client, ClientError};
use tokio::io::{AsyncBufReadExt, BufReader};

const HELP: &str = "linksy — share between your devices

Usage:
  linksy devices                     paired and nearby devices
  linksy pair <device|ip:port>       pair with a device (both screens show a code)
  linksy pair <linksy://pair?…>      pair using another device's QR code link
  linksy invite                      show a QR code another device can scan to pair
  linksy history [device]            what was shared, newest first
  linksy extend <device>             use another device as a second display
  linksy screen <device> --record FILE [--seconds N]
                                     save another device's screen as H.264
  linksy send <device> <files…>      send files or folders
  linksy notify <device> <title> [body]
  linksy clip <device> [text]        share text to its clipboard (default: stdin)
  linksy forget <device>             unpair a device
  linksy settings [key value]        show or change: name, download-dir,
                                     auto-accept on|off, stay-connected on|off
  linksy watch                       follow activity and answer requests
  linksy status                      this device

<device> is a name, a short id like A1B2-C3D4, or a full id.";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    };
    if matches!(cmd, "-h" | "--help" | "help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let client = match Client::connect().await {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Linksy service is not running. Start it with `linksyd` or `systemctl --user start linksyd`.");
            return ExitCode::FAILURE;
        }
    };
    match run(&client, cmd, &args[1..]).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(c: &Client, cmd: &str, rest: &[String]) -> Result<(), String> {
    let err = |e: ClientError| e.to_string();
    match (cmd, rest) {
        ("status", []) => {
            let s = c.status().await.map_err(err)?;
            println!("{} ({})", s.device.name, s.device.id.short());
            println!("Port      {}", s.port);
            println!("Saves to  {}", s.download_dir.display());
        }
        ("devices", []) => {
            let devices = c.devices().await.map_err(err)?;
            if devices.is_empty() {
                println!("No devices yet. Open Linksy on another device on this network.");
            }
            for d in devices {
                let state = match (d.trusted, d.connected, d.nearby) {
                    (true, true, _) => "connected",
                    (true, false, true) => "paired · nearby",
                    (true, false, false) => "paired · away",
                    (false, _, _) => "nearby · not paired",
                };
                println!("{} {:<24} {}  {}", icon(d.device.kind), d.device.name, d.device.id.short(), state);
            }
        }
        ("pair", [target]) => {
            let mut events = c.events();
            let pair = c.pair(target.clone());
            tokio::pin!(pair);
            loop {
                tokio::select! {
                    r = &mut pair => {
                        let d = r.map_err(err)?;
                        println!("Paired with {}.", d.name);
                        return Ok(());
                    }
                    Ok(ev) = events.recv() => {
                        if let Event::PairingRequest { request, device, code, incoming: false } = ev {
                            println!("Pairing code: {} {}", &code[..3], &code[3..]);
                            let ok = ask(&format!("Does {} show the same code?", device.name)).await;
                            c.respond(request, ok).await.map_err(err)?;
                        }
                    }
                }
            }
        }
        ("send", [to, files @ ..]) if !files.is_empty() => {
            let paths = files
                .iter()
                .map(|f| std::fs::canonicalize(f).map_err(|e| format!("{f}: {e}")))
                .collect::<Result<Vec<PathBuf>, _>>()?;
            let mut events = c.events();
            let id = c.send(to.clone(), paths).await.map_err(err)?;
            println!("Waiting for {to} to accept…");
            loop {
                match events.recv().await {
                    Ok(Event::TransferProgress { transfer, bytes, total }) if transfer == id => {
                        let pct = (bytes * 100).checked_div(total).unwrap_or(100);
                        print!("\r{} {pct:>3}%  {} of {}", bar(pct), size(bytes), size(total));
                        let _ = std::io::stdout().flush();
                    }
                    Ok(Event::TransferComplete { transfer, .. }) if transfer == id => {
                        println!("\nSent.");
                        return Ok(());
                    }
                    Ok(Event::TransferFailed { transfer, reason }) if transfer == id => {
                        println!();
                        return Err(if reason == "request was declined" { format!("{to} declined.") } else { reason });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return Err(err(ClientError::Disconnected)),
                    _ => {}
                }
            }
        }
        ("notify", [to, title, body @ ..]) if body.len() <= 1 => {
            c.notify(to.clone(), title.clone(), body.first().cloned().unwrap_or_default()).await.map_err(err)?;
        }
        ("clip", [to, text @ ..]) => {
            let text = if text.is_empty() {
                let mut buf = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).map_err(|e| e.to_string())?;
                buf
            } else {
                text.join(" ")
            };
            c.send_clipboard(to.clone(), text).await.map_err(err)?;
            println!("Shared clipboard with {to}.");
        }
        ("invite", []) => {
            let uri = c.create_invite().await.map_err(err)?;
            let code = qrcode::QrCode::new(uri.as_bytes()).map_err(|e| e.to_string())?;
            let art = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .dark_color(qrcode::render::unicode::Dense1x2::Light)
                .light_color(qrcode::render::unicode::Dense1x2::Dark)
                .quiet_zone(true)
                .build();
            println!("{art}\nScan with Linksy on your phone, or on another computer run:\n  linksy pair '{uri}'\n");
            println!("Waiting (valid 5 minutes). Press Ctrl+C to cancel.");
            let mut events = c.events();
            let outcome = tokio::select! {
                _ = tokio::signal::ctrl_c() => None,
                _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => None,
                d = async {
                    loop {
                        match events.recv().await {
                            Ok(Event::Paired { device }) => return Some(device),
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                            _ => {}
                        }
                    }
                } => d,
            };
            let _ = c.cancel_invite().await;
            match outcome {
                Some(d) => println!("Paired with {}.", d.name),
                None => return Err("No device paired.".into()),
            }
        }
        ("history", with) if with.len() <= 1 => {
            let entries = c.history(with.first().cloned()).await.map_err(err)?;
            if entries.is_empty() {
                println!("Nothing shared yet.");
            }
            for e in entries.iter().take(50) {
                use linksy_ipc::{HistoryStatus, Shared};
                let arrow = if e.direction == Direction::Incoming { "←" } else { "→" };
                let what = match &e.what {
                    Shared::Files { files, total, .. } => match files.as_slice() {
                        [one] => format!("{one} ({})", size(*total)),
                        many => format!("{} ({})", plural(many.len(), "item"), size(*total)),
                    },
                    Shared::Clipboard { text } if text.is_empty() => "clipboard text".into(),
                    Shared::Clipboard { text } => format!("clipboard: {}", text.chars().take(40).collect::<String>()),
                    Shared::Ping { title, .. } => format!("ping: {title}"),
                    Shared::Screen { seconds, display: true } => format!("second display ({seconds} s)"),
                    Shared::Screen { seconds, .. } if e.direction == Direction::Incoming => format!("watched their screen ({seconds} s)"),
                    Shared::Screen { seconds, .. } => format!("shared this screen ({seconds} s)"),
                };
                let status = match &e.status {
                    HistoryStatus::InProgress => "in progress".to_string(),
                    HistoryStatus::Done => String::new(),
                    HistoryStatus::Declined => "declined".into(),
                    HistoryStatus::Cancelled => "cancelled".into(),
                    HistoryStatus::Failed { reason } => format!("failed: {reason}"),
                };
                println!("{arrow} {:<18} {what}  {status}", e.peer_name);
            }
        }
        ("extend", [to]) => {
            println!("Asking {to} to be a second display…");
            let info = c.start_session(to.clone(), linksy_ipc::linksy_core::SessionKind::Display).await.map_err(err)?;
            println!("{to} is now a second display. Press Ctrl+C to stop.");
            let mut events = c.events();
            let ended = async {
                while let Ok(e) = events.recv().await {
                    if matches!(e, Event::SessionEnded { session } if session == info.session) {
                        return;
                    }
                }
            };
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = ended => println!("{to} stopped showing it."),
            }
            let _ = c.end_session(info.session).await;
        }
        ("screen", [to, flag, file, rest @ ..]) if flag == "--record" => {
            let seconds: u64 = match rest {
                [] => 10,
                [f, n] if f == "--seconds" => n.parse().map_err(|_| "--seconds needs a number")?,
                _ => return Err("Usage: linksy screen <device> --record FILE [--seconds N]".into()),
            };
            println!("Asking {to} to share its screen…");
            let info = c.start_session(to.clone(), linksy_ipc::linksy_core::SessionKind::Screen).await.map_err(err)?;
            let socket = info.socket.ok_or("no stream to record")?;
            let mut stream = tokio::net::UnixStream::connect(&socket).await.map_err(|e| e.to_string())?;
            let mut out = tokio::fs::File::create(file).await.map_err(|e| format!("{file}: {e}"))?;
            println!("Recording for {seconds} s…");
            let copied = tokio::time::timeout(std::time::Duration::from_secs(seconds), tokio::io::copy(&mut stream, &mut out)).await;
            let _ = c.end_session(info.session).await;
            let bytes = match copied {
                Ok(r) => r.map_err(|e| e.to_string())?,
                Err(_) => out.metadata().await.map(|m| m.len()).unwrap_or(0),
            };
            println!("Saved {} to {file}. Play it with: gst-play-1.0 {file}", size(bytes));
        }
        ("settings", []) => {
            let s = c.settings().await.map_err(err)?;
            println!("name            {}", s.name);
            println!("download-dir    {}", s.download_dir.display());
            println!("auto-accept     {}", on_off(s.auto_accept));
            println!("stay-connected  {}", on_off(s.stay_connected));
        }
        ("settings", [key, value @ ..]) if !value.is_empty() => {
            let mut s = c.settings().await.map_err(err)?;
            let value = value.join(" ");
            let flag = || match value.as_str() {
                "on" | "yes" | "true" => Ok(true),
                "off" | "no" | "false" => Ok(false),
                _ => Err(format!("{key} takes on or off")),
            };
            match key.as_str() {
                "name" => s.name = value.clone(),
                "download-dir" => {
                    s.download_dir = std::path::absolute(&value).map_err(|e| format!("{value}: {e}"))?;
                }
                "auto-accept" => s.auto_accept = flag()?,
                "stay-connected" => s.stay_connected = flag()?,
                other => return Err(format!("Unknown setting {other}. Run `linksy help`.")),
            }
            let s = c.update_settings(s).await.map_err(err)?;
            println!("Saved. This device is \"{}\".", s.name);
        }
        ("forget", [target]) => {
            if c.forget(target.clone()).await.map_err(err)? {
                println!("Forgot {target}.");
            } else {
                println!("{target} was not paired.");
            }
        }
        ("watch", []) => watch(c).await?,
        _ => return Err("Unknown command. Run `linksy help`.".to_string()),
    }
    Ok(())
}

async fn watch(c: &Client) -> Result<(), String> {
    let mut events = c.events();
    println!("Watching. Press Ctrl+C to stop.");
    loop {
        let ev = match events.recv().await {
            Ok(ev) => ev,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => return Err(ClientError::Disconnected.to_string()),
        };
        match ev {
            Event::DeviceFound { device, .. } => println!("· {} is nearby", device.name),
            Event::DeviceLost { id } => println!("· {} left", id.short()),
            Event::Connected { device } => println!("● {} connected", device.name),
            Event::Disconnected { id } => println!("○ {} disconnected", id.short()),
            Event::PairingRequest { request, device, code, incoming: true } => {
                println!("{} wants to pair. Code: {} {}", device.name, &code[..3], &code[3..]);
                let ok = ask("Does the other screen show the same code?").await;
                let _ = c.respond(request, ok).await;
            }
            Event::Paired { device } => println!("✓ Paired with {}", device.name),
            Event::PairingFailed { device, reason } => println!("✗ Pairing with {} failed: {reason}", device.name),
            Event::TransferRequest { transfer, from, files, total } => {
                println!("{} wants to send {} ({}):", from.name, plural(files.len(), "item"), size(total));
                for f in files.iter().take(5) {
                    println!("    {}", f.path);
                }
                if files.len() > 5 {
                    println!("    …and {} more", files.len() - 5);
                }
                let ok = ask("Accept?").await;
                let _ = c.respond(transfer, ok).await;
            }
            Event::TransferStarted { peer, direction, total, .. } => {
                let verb = if direction == Direction::Incoming { "Receiving from" } else { "Sending to" };
                println!("↕ {verb} {} ({})", peer.name, size(total));
            }
            Event::TransferComplete { saved, .. } if !saved.is_empty() => {
                for p in saved {
                    println!("✓ Saved {}", p.display());
                }
            }
            Event::TransferFailed { reason, .. } => println!("✗ Transfer failed: {reason}"),
            Event::Notification { from, title, body } => println!("🔔 {} — {title} {body}", from.name),
            Event::Clipboard { from, text } => println!("📋 {} shared: {text}", from.name),
            Event::SessionRequest { request, from, kind } => {
                match kind {
                    linksy_ipc::linksy_core::SessionKind::Control => println!("{} wants to control your screen (mouse and keyboard).", from.name),
                    linksy_ipc::linksy_core::SessionKind::Screen => println!("{} wants to see your screen.", from.name),
                    linksy_ipc::linksy_core::SessionKind::Display => println!("{} wants to use this screen as a second display.", from.name),
                }
                let ok = ask("Allow?").await;
                let _ = c.respond(request, ok).await;
            }
            Event::SessionStarted { peer, sharing: true, .. } => println!("🖥 Sharing your screen with {}", peer.name),
            Event::SessionEnded { .. } => println!("🖥 Screen sharing ended"),
            _ => {}
        }
    }
}

async fn ask(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = BufReader::new(tokio::io::stdin()).read_line(&mut line).await;
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

fn icon(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Desktop => "🖥",
        DeviceKind::Laptop => "💻",
        DeviceKind::Phone => "📱",
        DeviceKind::Tablet => "📱",
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn bar(pct: u64) -> String {
    let filled = (pct / 5) as usize;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(20 - filled.min(20)))
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}
