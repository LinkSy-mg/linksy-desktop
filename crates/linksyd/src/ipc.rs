//! IPC server: one task per front end, requests handled concurrently (pairing waits for
//! the user), events pushed to every client.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use linksy_core::{Event, Fingerprint, Node, Uuid};

use crate::history::History;
use crate::lock::Away;
use crate::mirror::Mirror;
use crate::photos::Photos;
use crate::sessions::Sessions;
use linksy_ipc::{Envelope, Request, ServerMsg, SessionInfo, Shared, Status};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub async fn serve(node: Node, history: History, sessions: Sessions, mirror: Mirror, away: Away, photos: Photos, socket: PathBuf) -> std::io::Result<()> {
    let listener = UnixListener::bind(&socket)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    }
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(client(node.clone(), history.clone(), sessions.clone(), mirror.clone(), away.clone(), photos.clone(), stream));
    }
}

async fn client(node: Node, history: History, sessions: Sessions, mirror: Mirror, away: Away, photos: Photos, stream: UnixStream) {
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
                // Still answer, or the caller waits forever: most likely an app newer
                // than this service, asking for something it doesn't know yet.
                debug!("bad request: {e}");
                if let Some(id) = serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("id")?.as_u64()) {
                    let error = "Linksy's background service is older than the app and doesn't know this yet. Restart it, or log out and in.".to_string();
                    let _ = tx.send(ServerMsg::Reply { id, ok: None, error: Some(error) });
                }
                continue;
            }
        };
        let (node, history, sessions, mirror, away, photos, tx) = (node.clone(), history.clone(), sessions.clone(), mirror.clone(), away.clone(), photos.clone(), tx.clone());
        tokio::spawn(async move {
            let msg = match handle(&node, &history, &sessions, &mirror, &away, &photos, env.request).await {
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

pub(crate) async fn handle(node: &Node, history: &History, sessions: &Sessions, mirror: &Mirror, away: &Away, photos: &Photos, req: Request) -> Result<Value, String> {
    let json = |v: serde_json::Result<Value>| v.map_err(|e| e.to_string());
    match req {
        Request::Status => json(serde_json::to_value(Status {
            device: node.info(),
            port: node.local_addr().map(|a| a.port()).unwrap_or(0),
            download_dir: node.settings().download_dir,
            version: env!("CARGO_PKG_VERSION").into(),
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
        Request::Respond { request, accept } => {
            // Answered in the app: the notification asking the same goes
            #[cfg(target_os = "linux")]
            crate::notify::answered(request);
            Ok(Value::Bool(node.respond(request, accept)))
        }
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
        Request::ClipboardFile { to, path } => {
            let id = resolve(node, &to)?;
            let transfer = node.send_clipboard_file(&id, path).await.map_err(|e| e.to_string())?;
            json(serde_json::to_value(transfer))
        }
        Request::OpenUrl { to, url } => {
            let id = resolve(node, &to)?;
            node.open_url(&id, &url).await.map_err(|e| e.to_string())?;
            if let Some(d) = device(node, &id) {
                history.record_sent(&d, Shared::Link { url: url.trim().into() });
            }
            Ok(Value::Null)
        }
        Request::Ring { to, on } => {
            let id = resolve(node, &to)?;
            node.ring(&id, on).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        Request::PhoneNotification { from, key } => {
            let id = resolve(node, &from)?;
            let found = mirror.get(&id, &key).ok_or("that notification is gone")?;
            serde_json::to_value(found).map_err(|e| e.to_string())
        }
        Request::ReplyNotification { from, key, text } => {
            let id = resolve(node, &from)?;
            mirror.reply(node, &id, &key, &text).await?;
            Ok(Value::Null)
        }
        Request::NotificationPrefs => serde_json::to_value(mirror.prefs()).map_err(|e| e.to_string()),
        Request::SetNotificationPrefs { prefs } => {
            mirror.set_prefs(prefs).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        Request::SmsThreads { phone } => {
            let id = resolve(node, &phone)?;
            let threads = ask(node, || node.sms_list_threads(&id, 50), SMS_WAIT, |e| match e {
                Event::SmsThreads { from, threads } if from.id == id => Some(threads),
                _ => None,
            })
            .await?;
            serde_json::to_value(threads).map_err(|e| e.to_string())
        }
        Request::SmsMessages { phone, thread, before_ms } => {
            let id = resolve(node, &phone)?;
            let page = ask(node, || node.sms_list_messages(&id, &thread, before_ms, 50), SMS_WAIT, |e| match e {
                Event::SmsMessages { from, thread: t, messages, more } if from.id == id && t == thread => Some((messages, more)),
                _ => None,
            })
            .await?;
            serde_json::to_value(page).map_err(|e| e.to_string())
        }
        Request::SmsSend { phone, to, text } => {
            let id = resolve(node, &phone)?;
            let client_id = Uuid::new_v4().to_string();
            let (ok, error) = ask(node, || node.sms_send(&id, &client_id, &to, &text), Duration::from_secs(60), |e| match e {
                Event::SmsSent { from, client_id: c, ok, error } if from.id == id && c == client_id => Some((ok, error)),
                _ => None,
            })
            .await?;
            match ok {
                true => Ok(Value::Null),
                false => Err(error.unwrap_or_else(|| "the phone couldn't send it".into())),
            }
        }
        Request::BrowseFiles { phone } => {
            let id = resolve(node, &phone)?;
            let name = device(node, &id).map(|d| d.name).unwrap_or_else(|| "The phone".into());
            #[cfg(target_os = "linux")]
            {
                let bridges = crate::dav::BRIDGES.get().ok_or("not ready yet")?;
                Ok(Value::String(bridges.browse(id, &name).await?))
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = (id, name);
                Err("browsing a phone's files isn't available here yet".into())
            }
        }
        Request::PhotoPrefs => serde_json::to_value(photos.prefs()).map_err(|e| e.to_string()),
        Request::SetPhotoPrefs { prefs } => {
            photos.set_prefs(node, prefs)?;
            Ok(Value::Null)
        }
        Request::SetAwayAll { device, on } => {
            let id = resolve(node, &device)?;
            node.set_away_all(&id, on).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        Request::SetAutoAccept { device, on } => {
            let id = resolve(node, &device)?;
            node.set_auto_accept(&id, on).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        Request::SetAllowScreen { device, on } => {
            let id = resolve(node, &device)?;
            node.set_allow_screen(&id, on).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        Request::Locate { device } => {
            let id = resolve(node, &device)?;
            let mut events = node.subscribe();
            node.locate(&id).await.map_err(|e| e.to_string())?;
            let answer = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                loop {
                    match events.recv().await {
                        Ok(linksy_core::Event::Location { from, lat, lon, accuracy_m, at_ms }) if from.id == id => {
                            return Ok(linksy_ipc::Location { lat, lon, accuracy_m, at_ms });
                        }
                        Ok(linksy_core::Event::LocationRefused { from, reason }) if from.id == id => return Err(reason),
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return Err("Linksy is stopping".to_string()),
                        _ => {}
                    }
                }
            })
            .await
            .map_err(|_| "the phone didn't answer".to_string())??;
            json(serde_json::to_value(answer))
        }
        Request::Updates => match crate::update::UPDATES.get() {
            Some(u) => json(serde_json::to_value(u.info())),
            None => Err("update checks aren't running".into()),
        },
        Request::SetUpdatePrefs { prefs } => {
            crate::update::UPDATES.get().ok_or("update checks aren't running")?.set_prefs(prefs);
            Ok(Value::Null)
        }
        Request::Quit => {
            // After the reply is out.
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                crate::QUIT.notify_one();
            });
            Ok(Value::Null)
        }
        Request::TerminalStatus => json(serde_json::to_value(terminals()?.status())),
        Request::SetTerminalCode { code } => {
            terminals()?.set_code(code).await?;
            Ok(Value::Null)
        }
        Request::AllowTerminal { target, on } => {
            terminals()?.allow(resolve_any(node, history, &target)?, on);
            Ok(Value::Null)
        }
        Request::UnlockTerminal { target } => {
            terminals()?.unlock(resolve_any(node, history, &target)?);
            Ok(Value::Null)
        }
        Request::EndTerminal { terminal } => {
            if terminals()?.end(&terminal) { Ok(Value::Null) } else { Err("that terminal isn't running".into()) }
        }
        Request::UsbStatus => json(serde_json::to_value(usb_status())),
        Request::UsbAuto { phone, on } => {
            #[cfg(target_os = "linux")]
            if let Some(usb) = crate::usb::USB.get() {
                usb.set_auto(&phone, on);
                return Ok(Value::Null);
            }
            let _ = (phone, on);
            Err("Linksy isn't watching USB here".into())
        }
        Request::UsbConnect { phone } => {
            #[cfg(target_os = "linux")]
            if let Some(usb) = crate::usb::USB.get() {
                return usb.connect(&phone).await.map(|()| Value::Null);
            }
            let _ = phone;
            Err("Linksy isn't watching USB here".into())
        }
        Request::LockPrefs => serde_json::to_value(away.prefs()).map_err(|e| e.to_string()),
        Request::SetLockPrefs { prefs } => {
            away.set_prefs(node, prefs).await?;
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
        Request::StartSession { with, kind } => {
            let id = resolve(node, &with)?;
            // A second display needs a virtual monitor: say so before asking the other device
            if kind == linksy_core::SessionKind::Display && !crate::sessions::virtual_monitors().await {
                return Err(crate::sessions::no_virtual_monitors());
            }
            let session = node.request_session(&id, kind).await.map_err(|e| e.to_string())?;
            let id = session.id;
            // A second display: this device shares, the service runs that side. A camera or
            // microphone: the service turns it into a device here. Otherwise the app shows it.
            let socket = if session.sharing {
                sessions.spawn_share(session);
                None
            } else if kind.is_device() {
                sessions.spawn_device(session);
                None
            } else {
                Some(sessions.view(session).map_err(|e| e.to_string())?)
            };
            json(serde_json::to_value(SessionInfo { session: id, socket }))
        }
        Request::PendingSessions => json(serde_json::to_value(sessions.pending())),
        Request::ActiveSessions => json(serde_json::to_value(sessions.active())),
        Request::EndSession { session } => Ok(Value::Bool(sessions.end(session))),
        Request::CameraControl { session, control } => {
            sessions.camera_control(session, control)?;
            Ok(Value::Null)
        }
        Request::CameraState { session } => Ok(sessions.camera_state(session).unwrap_or(Value::Null)),
        Request::DevicePicture { device } => {
            let id = resolve_any(node, history, &device)?;
            json(serde_json::to_value(crate::pictures::path(&id)))
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
fn usb_status() -> linksy_ipc::UsbStatus {
    #[cfg(target_os = "linux")]
    if let Some(usb) = crate::usb::USB.get() {
        return usb.status();
    }
    linksy_ipc::UsbStatus::default()
}

fn terminals() -> Result<&'static crate::terminal::Terminals, String> {
    crate::terminal::TERMINALS.get().ok_or_else(|| "terminals aren't available here".to_string())
}

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

/// How long to wait for a phone's answer about its texts.
const SMS_WAIT: Duration = Duration::from_secs(15);

/// Send a request to a device and wait for the event that answers it.
async fn ask<T, F, Fut>(node: &Node, send: F, wait: Duration, mut answer: impl FnMut(Event) -> Option<T>) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = linksy_core::Result<()>>,
{
    // Listen first, so a quick answer isn't missed.
    let mut events = node.subscribe();
    send().await.map_err(|e| e.to_string())?;
    let waiting = async {
        loop {
            match events.recv().await {
                Ok(e) => {
                    if let Some(t) = answer(e) {
                        return Ok(t);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return Err("the service is stopping".to_string()),
            }
        }
    };
    tokio::time::timeout(wait, waiting).await.map_err(|_| "the phone didn't answer; is Linksy open on it?".to_string())?
}
