//! IPC server: one task per front end, requests handled concurrently (pairing waits for
//! the user), events pushed to every client.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use linksy_core::{Fingerprint, Node};
use linksy_ipc::{Envelope, Request, ServerMsg, Status};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub async fn serve(node: Node, socket: PathBuf, download_dir: PathBuf) -> std::io::Result<()> {
    let listener = UnixListener::bind(&socket)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    }
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(client(node.clone(), stream, download_dir.clone()));
    }
}

async fn client(node: Node, stream: UnixStream, download_dir: PathBuf) {
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
        let (node, tx, dl) = (node.clone(), tx.clone(), download_dir.clone());
        tokio::spawn(async move {
            let msg = match handle(&node, env.request, &dl).await {
                Ok(v) => ServerMsg::Reply { id: env.id, ok: Some(v), error: None },
                Err(e) => ServerMsg::Reply { id: env.id, ok: None, error: Some(e) },
            };
            let _ = tx.send(msg);
        });
    }
    forward.abort();
    writer.abort();
}

async fn handle(node: &Node, req: Request, download_dir: &Path) -> Result<Value, String> {
    let json = |v: serde_json::Result<Value>| v.map_err(|e| e.to_string());
    match req {
        Request::Status => json(serde_json::to_value(Status {
            device: node.info().clone(),
            port: node.local_addr().map(|a| a.port()).unwrap_or(0),
            download_dir: download_dir.to_path_buf(),
        })),
        Request::Devices => json(serde_json::to_value(node.devices())),
        Request::Pair { target } => {
            let device = match target.parse::<SocketAddr>() {
                Ok(addr) => node.pair(addr).await,
                Err(_) => node.pair_device(resolve(node, &target)?).await,
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
            Ok(Value::Null)
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
