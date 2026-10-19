//! IPC server: one task per front end, requests handled concurrently (pairing waits for
//! the user), events pushed to every client.

use std::net::SocketAddr;
use std::path::PathBuf;

use linksy_core::{Fingerprint, Node};

use crate::history::History;
use linksy_ipc::{Envelope, Request, ServerMsg, Shared, Status};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub async fn serve(node: Node, history: History, socket: PathBuf) -> std::io::Result<()> {
    let listener = UnixListener::bind(&socket)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    }
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(client(node.clone(), history.clone(), stream));
    }
}

async fn client(node: Node, history: History, stream: UnixStream) {
    let (read, mut write) = stream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();

    let mut events = node.subscribe();
    let ev_tx = tx.clone();
    let forward = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if ev_tx.send(ServerMsg::Event { event }).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => warn!("client lagged, dropped {n} events"),
                Err(_) => break,
            }
        }
    });
    let mut updates = history.subscribe();
    let hist_tx = tx.clone();
    let forward_history = tokio::spawn(async move {
        loop {
            match updates.recv().await {
                Ok(history) => {
                    if hist_tx.send(ServerMsg::History { history }).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let Ok(mut line) = serde_json::to_vec(&msg) else { continue };
            line.push(b'\n');
            if write.write_all(&line).await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let env: Envelope = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(e) => {
                debug!("bad request: {e}");
                continue;
            }
        };
        let (node, history, tx) = (node.clone(), history.clone(), tx.clone());
        tokio::spawn(async move {
            let msg = match handle(&node, &history, env.request).await {
                Ok(v) => ServerMsg::Reply { id: env.id, ok: Some(v), error: None },
                Err(e) => ServerMsg::Reply { id: env.id, ok: None, error: Some(e) },
            };
            let _ = tx.send(msg);
        });
    }
    forward.abort();
    forward_history.abort();
    writer.abort();
}

async fn handle(node: &Node, history: &History, req: Request) -> Result<Value, String> {
    let json = |v: serde_json::Result<Value>| v.map_err(|e| e.to_string());
    match req {
        Request::Status => json(serde_json::to_value(Status {
            device: node.info(),
            port: node.local_addr().map(|a| a.port()).unwrap_or(0),
            download_dir: node.settings().download_dir,
        })),
        Request::Devices => json(serde_json::to_value(node.devices())),
        Request::Pair { target } => {
            let device = if target.starts_with(linksy_core::invite::SCHEME) {
                node.pair_with_invite(&target).await
            } else {
                match target.parse::<SocketAddr>() {
                    Ok(addr) => node.pair(addr).await,
                    Err(_) => node.pair_device(resolve(node, &target)?).await,
                }
            }
            .map_err(|e| e.to_string())?;
            json(serde_json::to_value(device))
        }
        Request::Respond { request, accept } => Ok(Value::Bool(node.respond(request, accept))),
        Request::Send { to, paths } => {
            let id = resolve(node, &to)?;
            let transfer = node.send_files(&id, paths).await.map_err(|e| e.to_string())?;
            json(serde_json::to_value(transfer))
        }
        Request::Cancel { transfer } => Ok(Value::Bool(node.cancel(transfer))),
        Request::Notify { to, title, body } => {
            let id = resolve(node, &to)?;
            node.notify(&id, &title, &body).await.map_err(|e| e.to_string())?;
            if let Some(d) = device(node, &id) {
                history.record_sent(&d, Shared::Ping { title, body });
            }
            Ok(Value::Null)
        }
        Request::Clipboard { to, text } => {
            let id = resolve(node, &to)?;
            node.send_clipboard(&id, &text).await.map_err(|e| e.to_string())?;
            if let Some(d) = device(node, &id) {
                history.record_sent(&d, Shared::Clipboard { text });
            }
            Ok(Value::Null)
        }
        Request::CreateInvite => Ok(Value::String(node.create_invite().map_err(|e| e.to_string())?.to_uri())),
        Request::CancelInvite => {
            node.cancel_invite();
            Ok(Value::Null)
        }
        Request::History { with } => {
            let with = with.map(|w| resolve_any(node, history, &w)).transpose()?;
            json(serde_json::to_value(history.list(with)))
        }
        Request::ClearHistory { with } => {
            let with = with.map(|w| resolve_any(node, history, &w)).transpose()?;
            history.clear(with);
            Ok(Value::Null)
        }
        Request::Settings => json(serde_json::to_value(node.settings())),
        Request::UpdateSettings { settings } => {
            let applied = node.update_settings(settings).await.map_err(|e| e.to_string())?;
            json(serde_json::to_value(applied))
        }
        Request::Forget { target } => {
            let id = resolve(node, &target)?;
            Ok(Value::Bool(node.forget(&id).map_err(|e| e.to_string())?))
        }
    }
}

/// Find a device by full id, short id, or name (case-insensitive).
fn resolve(node: &Node, target: &str) -> Result<Fingerprint, String> {
    if let Ok(fp) = target.parse::<Fingerprint>() {
        return Ok(fp);
    }
    let t = target.trim();
    let matches: Vec<_> = node
        .devices()
        .into_iter()
        .filter(|d| d.device.id.short().eq_ignore_ascii_case(t) || d.device.name.eq_ignore_ascii_case(t))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.device.id),
        [] => Err(format!("No device called \"{t}\". Run `linksy devices` to see what's nearby.")),
        _ => Err(format!("Several devices are called \"{t}\". Use the device id instead.")),
    }
}

fn device(node: &Node, id: &Fingerprint) -> Option<linksy_core::DeviceInfo> {
    node.devices().into_iter().find(|d| d.device.id == *id).map(|d| d.device)
}

/// Like [`resolve`], but also finds forgotten devices that still have history.
fn resolve_any(node: &Node, history: &History, target: &str) -> Result<Fingerprint, String> {
    resolve(node, target).or_else(|e| {
        let t = target.trim();
        history.list(None).into_iter().find(|h| h.peer.short().eq_ignore_ascii_case(t) || h.peer_name.eq_ignore_ascii_case(t)).map(|h| h.peer).ok_or(e)
    })
}
