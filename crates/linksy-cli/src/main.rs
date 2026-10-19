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
  linksy send <device> <files…>      send files or folders
  linksy notify <device> <title> [body]
  linksy clip <device> [text]        share text to its clipboard (default: stdin)
  linksy forget <device>             unpair a device
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
