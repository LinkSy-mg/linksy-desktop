//! End-to-end tests: two nodes on loopback, discovery off, consent answered by a helper.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use linksy_core::{Battery, CallState, Config, ControlMsg, DirectStep, FsEntry, FsRequest, SmsThread, Error, Event, MediaCommand, MediaPlayer, MediaState, Node, NotificationButton, RemoteNotification, SessionKind, VolumeState};
use tokio::io::AsyncReadExt;
use tempfile::TempDir;
use tokio::sync::broadcast;

struct Dev {
    node: Node,
    dir: TempDir,
}

impl Dev {
    fn downloads(&self) -> PathBuf {
        self.dir.path().join("downloads")
    }
}

fn config(dir: &Path, name: &str) -> Config {
    let mut c = Config::new(dir.join("data"), dir.join("downloads"), name);
    c.bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    c.discovery = false;
    c
}

async fn dev(name: &str) -> Dev {
    dev_with(name, |_| {}).await
}

async fn dev_with(name: &str, tweak: impl FnOnce(&mut Config)) -> Dev {
    let dir = tempfile::tempdir().unwrap();
    let mut c = config(dir.path(), name);
    tweak(&mut c);
    let node = Node::start(c).await.unwrap();
    Dev { node, dir }
}

/// Answer every pairing/transfer request on `node` with `accept`.
fn auto_answer(node: &Node, accept: bool) {
    let mut rx = node.subscribe();
    let node = node.clone();
    tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            match ev {
                Event::PairingRequest { request, .. } => {
                    node.respond(request, accept);
                }
                Event::TransferRequest { transfer, .. } => {
                    node.respond(transfer, accept);
                }
                // A display is shown at a phone-like 2400×1080, to check the size travels.
                Event::SessionRequest { request, kind: SessionKind::Display, .. } => {
                    node.respond_session(request, accept, Some((2400, 1080)));
                }
                Event::SessionRequest { request, .. } => {
                    node.respond(request, accept);
                }
                _ => {}
            }
        }
    });
}

async fn wait_for<T>(rx: &mut broadcast::Receiver<Event>, mut f: impl FnMut(&Event) -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let ev = rx.recv().await.expect("event stream closed");
            if std::env::var("LINKSY_TRACE").is_ok() { eprintln!("event: {ev:?}"); }
            if let Some(v) = f(&ev) {
                return v;
            }
        }
    })
    .await
    .expect("timed out waiting for event")
}

async fn paired() -> (Dev, Dev) {
    paired_with(|_| {}).await
}

async fn paired_with(tweak: impl Fn(&mut Config)) -> (Dev, Dev) {
    let a = dev_with("Alpha", &tweak).await;
    let b = dev_with("Beta", &tweak).await;
    auto_answer(&a.node, true);
    auto_answer(&b.node, true);
    let mut rb = b.node.subscribe();
    let mut addr = b.node.local_addr().unwrap();
    // Bound to [::]: dial IPv6 loopback (Windows sockets don't take IPv4 there by default).
    if addr.ip().is_unspecified() {
        addr.set_ip(std::net::Ipv6Addr::LOCALHOST.into());
    }
    let got = a.node.pair(addr).await.unwrap();
    assert_eq!(got.name, "Beta");
    wait_for(&mut rb, |e| matches!(e, Event::Connected { .. }).then_some(())).await;
    (a, b)
}

#[tokio::test]
async fn pairing_shows_same_code_and_trusts_both_ways() {
    let a = dev("Alpha").await;
    let b = dev("Beta").await;
    let mut ra = a.node.subscribe();
    let mut rb = b.node.subscribe();
    auto_answer(&a.node, true);
    auto_answer(&b.node, true);

    a.node.pair(b.node.local_addr().unwrap()).await.unwrap();
    let code_a = wait_for(&mut ra, |e| match e {
        Event::PairingRequest { code, incoming: false, .. } => Some(code.clone()),
        _ => None,
    })
    .await;
    let code_b = wait_for(&mut rb, |e| match e {
        Event::PairingRequest { code, incoming: true, .. } => Some(code.clone()),
        _ => None,
    })
    .await;
    assert_eq!(code_a, code_b);
    assert_eq!(code_a.len(), 6);

    wait_for(&mut rb, |e| matches!(e, Event::Paired { .. }).then_some(())).await;
    assert!(a.node.trusted().iter().any(|t| t.device.id == b.node.info().id));
    assert!(b.node.trusted().iter().any(|t| t.device.id == a.node.info().id));
    let views = a.node.devices();
    assert!(views[0].connected && views[0].trusted);
}

/// The discovery key `dev` announces with (docs/protocol.md §4.16).
fn discovery_key(dev: &Dev) -> String {
    hex::encode(std::fs::read(dev.dir.path().join("data/discovery.key")).unwrap())
}

/// The discovery key `dev` has stored for `of`, once it arrives.
async fn stored_key(dev: &Dev, of: &Dev, not: Option<&str>) -> String {
    let id = of.node.info().id;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let key = dev.node.trusted().into_iter().find(|t| t.device.id == id).and_then(|t| t.discovery_key);
            if let Some(key) = key.filter(|k| Some(k.as_str()) != not) {
                return key;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("no discovery key arrived")
}

#[tokio::test]
async fn paired_devices_share_discovery_keys_and_forgetting_rotates_them() {
    let (a, b) = paired().await;
    assert_eq!(stored_key(&b, &a, None).await, discovery_key(&a));
    assert_eq!(stored_key(&a, &b, None).await, discovery_key(&b));

    let c = dev("Gamma").await;
    auto_answer(&c.node, true);
    c.node.pair(a.node.local_addr().unwrap()).await.unwrap();
    let old = stored_key(&c, &a, None).await;
    assert_eq!(old, discovery_key(&a));

    // Forgetting Beta: a new key, sent to Gamma right away; Beta's copy is useless now.
    assert!(a.node.forget(&b.node.info().id).unwrap());
    let new = stored_key(&c, &a, Some(&old)).await;
    assert_eq!(new, discovery_key(&a));
    assert_ne!(stored_key(&b, &a, None).await, new);
}

#[tokio::test]
async fn declined_pairing_trusts_nobody() {
    let a = dev("Alpha").await;
    let b = dev("Beta").await;
    auto_answer(&a.node, true);
    auto_answer(&b.node, false);
    let err = a.node.pair(b.node.local_addr().unwrap()).await.unwrap_err();
    assert!(matches!(err, Error::Declined), "{err}");
    assert!(a.node.trusted().is_empty());
    assert!(b.node.trusted().is_empty());
}

#[tokio::test]
async fn unpaired_device_cannot_connect() {
    let a = dev("Alpha").await;
    let b = dev("Beta").await;
    let err = a.node.connect(b.node.local_addr().unwrap()).await.unwrap_err();
    assert!(matches!(err, Error::NotPaired), "{err}");
}

#[tokio::test]
async fn sends_files_and_folders_intact() {
    let (a, b) = paired().await;
    let src = a.dir.path().join("out");
    std::fs::create_dir_all(src.join("Album/sub")).unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    std::fs::write(src.join("big.bin"), &big).unwrap();
    std::fs::write(src.join("Album/a.txt"), b"hello").unwrap();
    std::fs::write(src.join("Album/sub/empty"), b"").unwrap();

    let mut ra = a.node.subscribe();
    let mut rb = b.node.subscribe();
    let id = a.node.send_files(&b.node.info().id, vec![src.join("big.bin"), src.join("Album")]).await.unwrap();

    let saved = wait_for(&mut rb, |e| match e {
        Event::TransferComplete { transfer, saved } if *transfer == id => Some(saved.clone()),
        Event::TransferFailed { reason, .. } => panic!("receiver failed: {reason}"),
        _ => None,
    })
    .await;
    wait_for(&mut ra, |e| match e {
        Event::TransferComplete { transfer, .. } if *transfer == id => Some(()),
        Event::TransferFailed { reason, .. } => panic!("sender failed: {reason}"),
        _ => None,
    })
    .await;

    assert_eq!(saved.len(), 3);
    let dl = b.downloads();
    assert_eq!(std::fs::read(dl.join("big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(dl.join("Album/a.txt")).unwrap(), b"hello");
    assert_eq!(std::fs::read(dl.join("Album/sub/empty")).unwrap(), b"");
}

#[tokio::test]
async fn declined_transfer_writes_nothing() {
    let a = dev("Alpha").await;
    let b = dev("Beta").await;
    // Pair with consent, then refuse files.
    auto_answer(&a.node, true);
    let mut rb = b.node.subscribe();
    let node_b = b.node.clone();
    tokio::spawn(async move {
        while let Ok(ev) = rb.recv().await {
            match ev {
                Event::PairingRequest { request, .. } => {
                    node_b.respond(request, true);
                }
                Event::TransferRequest { transfer, .. } => {
                    node_b.respond(transfer, false);
                }
                _ => {}
            }
        }
    });
    a.node.pair(b.node.local_addr().unwrap()).await.unwrap();

    let f = a.dir.path().join("x.txt");
    std::fs::write(&f, b"secret").unwrap();
    let mut ra = a.node.subscribe();
    let id = a.node.send_files(&b.node.info().id, vec![f]).await.unwrap();
    let reason = wait_for(&mut ra, |e| match e {
        Event::TransferFailed { transfer, reason } if *transfer == id => Some(reason.clone()),
        Event::TransferComplete { .. } => panic!("should not complete"),
        _ => None,
    })
    .await;
    assert!(reason.contains("declined"), "{reason}");
    assert!(!b.downloads().join("x.txt").exists());
}

#[tokio::test]
async fn resumes_partial_file_and_rejects_corrupt_one() {
    let (a, b) = paired().await;
    let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    let f = a.dir.path().join("video.mp4");
    std::fs::write(&f, &data).unwrap();
    let dl = b.downloads();
    std::fs::create_dir_all(&dl).unwrap();

    // Good partial: first 400 kB already here.
    std::fs::write(dl.join("video.mp4.linksy-part"), &data[..400_000]).unwrap();
    let mut rb = b.node.subscribe();
    let id = a.node.send_files(&b.node.info().id, vec![f.clone()]).await.unwrap();
    let first_progress = wait_for(&mut rb, |e| match e {
        Event::TransferProgress { transfer, bytes, .. } if *transfer == id => Some(*bytes),
        _ => None,
    })
    .await;
    assert_eq!(first_progress, 400_000, "progress starts at the resumed offset");
    wait_for(&mut rb, |e| match e {
        Event::TransferComplete { transfer, .. } if *transfer == id => Some(()),
        Event::TransferFailed { reason, .. } => panic!("{reason}"),
        _ => None,
    })
    .await;
    assert_eq!(std::fs::read(dl.join("video.mp4")).unwrap(), data);

    // Corrupt partial: the whole-file hash must catch it and nothing is saved.
    let mut bad = data[..400_000].to_vec();
    bad[10] ^= 0xff;
    std::fs::write(dl.join("video (1).mp4.linksy-part"), &bad).unwrap();
    let id = a.node.send_files(&b.node.info().id, vec![f]).await.unwrap();
    let reason = wait_for(&mut rb, |e| match e {
        Event::TransferFailed { transfer, reason } if *transfer == id => Some(reason.clone()),
        Event::TransferComplete { transfer, .. } if *transfer == id => panic!("corrupt file accepted"),
        _ => None,
    })
    .await;
    assert!(reason.contains("checksum"), "{reason}");
    assert!(!dl.join("video (1).mp4").exists());
    assert!(!dl.join("video (1).mp4.linksy-part").exists(), "bad partial removed so a retry starts clean");
}

#[tokio::test]
async fn notifications_arrive() {
    let (a, b) = paired().await;
    let mut rb = b.node.subscribe();
    a.node.notify(&b.node.info().id, "Hello", "From Alpha").await.unwrap();
    let (from, title) = wait_for(&mut rb, |e| match e {
        Event::Notification { from, title, .. } => Some((from.name.clone(), title.clone())),
        _ => None,
    })
    .await;
    assert_eq!((from.as_str(), title.as_str()), ("Alpha", "Hello"));
}

#[tokio::test]
async fn clipboard_text_arrives_and_limits_hold() {
    let (a, b) = paired().await;
    let mut rb = b.node.subscribe();
    let to = b.node.info().id;
    let text = "multi-line ✓\nclipboard";
    a.node.send_clipboard(&to, text).await.unwrap();
    let got = wait_for(&mut rb, |e| match e {
        Event::Clipboard { from, text } if from.name == "Alpha" => Some(text.clone()),
        _ => None,
    })
    .await;
    assert_eq!(got, text);
    assert!(a.node.send_clipboard(&to, "").await.is_err());
    assert!(a.node.send_clipboard(&to, &"x".repeat(linksy_core::MAX_CLIPBOARD + 1)).await.is_err());
}

#[tokio::test]
async fn web_links_arrive_and_nothing_else_is_sent() {
    let (a, b) = paired().await;
    let mut rb = b.node.subscribe();
    let to = b.node.info().id;
    a.node.open_url(&to, " https://example.org/watch?v=1 ").await.unwrap();
    let got = wait_for(&mut rb, |e| match e {
        Event::OpenUrl { from, url } if from.name == "Alpha" => Some(url.clone()),
        _ => None,
    })
    .await;
    assert_eq!(got, "https://example.org/watch?v=1");
    for bad in ["file:///etc/passwd", "javascript:alert(1)", "https://", "ftp://x", "http://a\nb", ""] {
        assert!(a.node.open_url(&to, bad).await.is_err(), "{bad}");
    }
    a.node.open_url(&to, "HTTP://Example.org").await.unwrap();
}

#[tokio::test]
async fn battery_shows_on_the_device_and_ring_arrives() {
    let (a, b) = paired().await;
    let mut rb = b.node.subscribe();
    let to = b.node.info().id;
    let low = Battery { level: 14, charging: false };
    a.node.send_battery(&to, low).await.unwrap();
    let got = wait_for(&mut rb, |e| match e {
        Event::Battery { from, battery } if from.name == "Alpha" => Some(*battery),
        _ => None,
    })
    .await;
    assert_eq!(got, low);
    let alpha = a.node.info().id;
    assert_eq!(b.node.devices().iter().find(|d| d.device.id == alpha).unwrap().battery, Some(low));
    assert!(a.node.send_battery(&to, Battery { level: 101, charging: true }).await.is_err());

    a.node.ring(&to, true).await.unwrap();
    a.node.ring(&to, false).await.unwrap();
    let mut rings = Vec::new();
    while rings.len() < 2 {
        rings.push(wait_for(&mut rb, |e| match e {
            Event::Ring { on, .. } => Some(*on),
            _ => None,
        })
        .await);
    }
    // Two uni streams may arrive in either order; both must arrive.
    rings.sort();
    assert_eq!(rings, [false, true]);
}

#[tokio::test]
async fn mirrored_notifications_go_both_ways() {
    let (phone, pc) = paired().await;
    let mut on_pc = pc.node.subscribe();
    let mut on_phone = phone.node.subscribe();
    let (pc_id, phone_id) = (pc.node.info().id, phone.node.info().id);
    let n = RemoteNotification {
        key: "0|com.whatsapp|1|null|10123".into(),
        app: "com.whatsapp".into(),
        app_name: "WhatsApp".into(),
        title: "Alice".into(),
        body: "Lunch?".into(),
        can_reply: true,
        actions: vec![NotificationButton { id: "read".into(), label: "Mark as read".into() }],
        icon: None,
    };
    phone.node.post_notification(&pc_id, n.clone()).await.unwrap();
    let got = wait_for(&mut on_pc, |e| match e {
        Event::NotificationPosted { notification, .. } => Some(notification.clone()),
        _ => None,
    })
    .await;
    assert_eq!(got, n);

    pc.node.reply_notification(&phone_id, &n.key, "Sure!").await.unwrap();
    let reply = wait_for(&mut on_phone, |e| match e {
        Event::NotificationReply { key, text, .. } => Some((key.clone(), text.clone())),
        _ => None,
    })
    .await;
    assert_eq!(reply, (n.key.clone(), "Sure!".to_string()));
    pc.node.notification_action(&phone_id, &n.key, "read").await.unwrap();
    wait_for(&mut on_phone, |e| matches!(e, Event::NotificationAction { action, .. } if action == "read").then_some(())).await;
    pc.node.dismiss_notification(&phone_id, &n.key).await.unwrap();
    wait_for(&mut on_phone, |e| matches!(e, Event::NotificationDismissed { .. }).then_some(())).await;
    phone.node.remove_notification(&pc_id, &n.key).await.unwrap();
    wait_for(&mut on_pc, |e| matches!(e, Event::NotificationRemoved { key, .. } if *key == n.key).then_some(())).await;

    assert!(pc.node.reply_notification(&phone_id, &n.key, "  ").await.is_err());
    let huge = RemoteNotification { icon: Some("A".repeat(200_000)), ..n.clone() };
    assert!(phone.node.post_notification(&pc_id, huge).await.is_err());
    assert!(phone.node.post_notification(&pc_id, RemoteNotification { key: String::new(), ..n }).await.is_err());
}

#[tokio::test]
async fn volume_watched_set_and_kept_newest() {
    let (phone, pc) = paired().await;
    let mut on_pc = pc.node.subscribe();
    let mut on_phone = phone.node.subscribe();
    let (pc_id, phone_id) = (pc.node.info().id, phone.node.info().id);

    phone.node.watch_volume(&pc_id, true).await.unwrap();
    wait_for(&mut on_pc, |e| matches!(e, Event::VolumeWatch { on: true, .. }).then_some(())).await;
    pc.node.send_volume_state(&phone_id, &VolumeState { seq: 20, level: 0.7, muted: false }).await.unwrap();
    let got = wait_for(&mut on_phone, |e| match e {
        Event::VolumeState { state, .. } => Some(*state),
        _ => None,
    })
    .await;
    assert_eq!(got, VolumeState { seq: 20, level: 0.7, muted: false });
    // An older one arriving late is dropped
    pc.node.send_volume_state(&phone_id, &VolumeState { seq: 10, level: 0.1, muted: true }).await.unwrap();
    pc.node.send_volume_state(&phone_id, &VolumeState { seq: 30, level: 0.5, muted: false }).await.unwrap();
    let next = wait_for(&mut on_phone, |e| match e {
        Event::VolumeState { state, .. } => Some(state.seq),
        _ => None,
    })
    .await;
    assert_eq!(next, 30);
    assert!(pc.node.send_volume_state(&phone_id, &VolumeState { seq: 40, level: f64::NAN, muted: false }).await.is_err());

    // Beyond 100 %: kept at 100 %
    phone.node.set_volume(&pc_id, Some(1.7), None).await.unwrap();
    let set = wait_for(&mut on_pc, |e| match e {
        Event::VolumeSet { level, muted, .. } => Some((*level, *muted)),
        _ => None,
    })
    .await;
    assert_eq!(set, (Some(1.0), None));
    phone.node.set_volume(&pc_id, None, Some(true)).await.unwrap();
    let mute = wait_for(&mut on_pc, |e| match e {
        Event::VolumeSet { level, muted, .. } => Some((*level, *muted)),
        _ => None,
    })
    .await;
    assert_eq!(mute, (None, Some(true)));
}

#[tokio::test]
async fn media_state_keeps_the_newest_and_commands_arrive() {
    let (phone, pc) = paired().await;
    let mut on_pc = pc.node.subscribe();
    let mut on_phone = phone.node.subscribe();
    let (pc_id, phone_id) = (pc.node.info().id, phone.node.info().id);
    let song = |seq: u64, playing: bool| MediaState {
        seq,
        players: vec![MediaPlayer {
            id: "spotify".into(),
            app: "Spotify".into(),
            title: "Song".into(),
            artist: "Band".into(),
            album: String::new(),
            playing,
            position_ms: Some(1000),
            length_ms: Some(180_000),
            can_next: true,
            can_previous: true,
            can_seek: true,
            art: None,
        }],
    };
    phone.node.send_media_state(&pc_id, &song(20, true)).await.unwrap();
    let got = wait_for(&mut on_pc, |e| match e {
        Event::MediaState { state, .. } => Some(state.clone()),
        _ => None,
    })
    .await;
    assert_eq!(got, song(20, true));
    // An older snapshot arriving late is dropped; the next newer one comes through.
    phone.node.send_media_state(&pc_id, &song(10, false)).await.unwrap();
    phone.node.send_media_state(&pc_id, &song(30, false)).await.unwrap();
    let next = wait_for(&mut on_pc, |e| match e {
        Event::MediaState { state, .. } => Some(state.seq),
        _ => None,
    })
    .await;
    assert_eq!(next, 30);

    pc.node.media_command(&phone_id, "spotify", MediaCommand::Seek { position_ms: 5000 }).await.unwrap();
    let cmd = wait_for(&mut on_phone, |e| match e {
        Event::MediaCommand { player, command, .. } => Some((player.clone(), *command)),
        _ => None,
    })
    .await;
    assert_eq!(cmd, ("spotify".to_string(), MediaCommand::Seek { position_ms: 5000 }));
    pc.node.watch_media(&phone_id, true).await.unwrap();
    wait_for(&mut on_phone, |e| matches!(e, Event::MediaWatch { on: true, .. }).then_some(())).await;

    phone.node.send_call_state(&pc_id, CallState::Ringing, Some("Alice")).await.unwrap();
    let call = wait_for(&mut on_pc, |e| match e {
        Event::CallState { state, contact, .. } => Some((*state, contact.clone())),
        _ => None,
    })
    .await;
    assert_eq!(call, (CallState::Ringing, Some("Alice".to_string())));

    pc.node.watch_heartbeat(&phone_id, true, 1).await.unwrap();
    let every = wait_for(&mut on_phone, |e| match e {
        Event::HeartbeatWatch { on: true, every_s, .. } => Some(*every_s),
        _ => None,
    })
    .await;
    assert_eq!(every, 5, "no faster than every 5 s");
    phone.node.heartbeat(&pc_id).await.unwrap();
    wait_for(&mut on_pc, |e| matches!(e, Event::Heartbeat { .. }).then_some(())).await;
}

#[tokio::test]
async fn photo_imports_go_to_the_device_folder_or_are_refused() {
    let (phone, pc) = paired().await;
    let src = phone.dir.path().join("DCIM");
    std::fs::create_dir_all(src.join("2026-10")).unwrap();
    std::fs::write(src.join("2026-10/IMG_1.jpg"), b"jpeg").unwrap();
    let to = pc.node.info().id;
    let mut on_phone = phone.node.subscribe();

    // Not taking imports: refused, and nothing asks.
    let refused = phone.node.import_files(&to, vec![src.join("2026-10")]).await.unwrap();
    wait_for(&mut on_phone, |e| matches!(e, Event::TransferFailed { transfer, .. } if *transfer == refused).then_some(())).await;

    let pictures = pc.dir.path().join("Pictures");
    pc.node.set_import_dir(Some(pictures.clone()));
    let mut on_pc = pc.node.subscribe();
    let id = phone.node.import_files(&to, vec![src.join("2026-10")]).await.unwrap();
    let import = wait_for(&mut on_pc, |e| match e {
        Event::TransferStarted { transfer, import, .. } if *transfer == id => Some(*import),
        Event::TransferRequest { .. } => panic!("imports never ask"),
        _ => None,
    })
    .await;
    assert!(import);
    wait_for(&mut on_pc, |e| matches!(e, Event::TransferComplete { transfer, .. } if *transfer == id).then_some(())).await;
    assert_eq!(std::fs::read(pictures.join("Alpha/2026-10/IMG_1.jpg")).unwrap(), b"jpeg");
}

#[tokio::test]
async fn texting_asks_answers_and_sends() {
    let (phone, pc) = paired().await;
    let (phone_id, pc_id) = (phone.node.info().id, pc.node.info().id);
    let mut on_phone = phone.node.subscribe();
    let mut on_pc = pc.node.subscribe();

    pc.node.sms_list_threads(&phone_id, 500).await.unwrap();
    let limit = wait_for(&mut on_phone, |e| match e {
        Event::SmsListThreads { limit, .. } => Some(*limit),
        _ => None,
    })
    .await;
    assert_eq!(limit, 100, "capped");
    let thread = SmsThread { id: "7".into(), addresses: vec!["+261340000000".into()], names: vec!["Alice".into()], snippet: "Lunch?".into(), date_ms: 1, unread: true };
    phone.node.sms_threads(&pc_id, vec![thread.clone()]).await.unwrap();
    let got = wait_for(&mut on_pc, |e| match e {
        Event::SmsThreads { threads, .. } => Some(threads.clone()),
        _ => None,
    })
    .await;
    assert_eq!(got, vec![thread]);

    pc.node.sms_send(&phone_id, "c1", &["+261340000000".to_string()], "Sure!").await.unwrap();
    let (id, text) = wait_for(&mut on_phone, |e| match e {
        Event::SmsSend { client_id, text, .. } => Some((client_id.clone(), text.clone())),
        _ => None,
    })
    .await;
    assert_eq!((id.as_str(), text.as_str()), ("c1", "Sure!"));
    phone.node.sms_sent(&pc_id, "c1", true, None).await.unwrap();
    wait_for(&mut on_pc, |e| matches!(e, Event::SmsSent { client_id, ok: true, .. } if client_id == "c1").then_some(())).await;
    assert!(pc.node.sms_send(&phone_id, "c2", &[], "x").await.is_err());
}

/// Serve `root` the way a phone would, for the file-browsing tests.
fn serve_files(node: &Node, root: std::path::PathBuf) {
    let mut calls = node.incoming_fs().unwrap();
    tokio::spawn(async move {
        while let Some(mut call) = calls.recv().await {
            let at = |p: &str| root.join(p.trim_start_matches('/'));
            let entry = |p: &std::path::Path| {
                let m = std::fs::metadata(p).ok()?;
                Some(FsEntry { name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), dir: m.is_dir(), size: m.len(), modified_ms: 0 })
            };
            let _: linksy_core::Result<()> = match call.request.clone() {
                FsRequest::Info => call.info("storage").await,
                FsRequest::List { path } => match std::fs::read_dir(at(&path)) {
                    Ok(rd) => call.entries(rd.flatten().filter_map(|e| entry(&e.path())).collect()).await,
                    Err(_) => call.error("no such folder", true).await,
                },
                FsRequest::Stat { path } => match entry(&at(&path)) {
                    Some(e) => call.entry(e).await,
                    None => call.error("no such file", true).await,
                },
                FsRequest::Read { path, offset, length } => {
                    let all = std::fs::read(at(&path)).unwrap_or_default();
                    let start = (offset as usize).min(all.len());
                    let end = (start + length as usize).min(all.len());
                    call.data(&all[start..end]).await
                }
                FsRequest::Write { path, offset, truncate, .. } => {
                    let body = call.body().await.unwrap();
                    let mut all = if truncate { Vec::new() } else { std::fs::read(at(&path)).unwrap_or_default() };
                    all.truncate(offset as usize);
                    all.extend(body);
                    std::fs::write(at(&path), all).unwrap();
                    call.ok().await
                }
                FsRequest::Mkdir { path } => {
                    std::fs::create_dir_all(at(&path)).unwrap();
                    call.ok().await
                }
                FsRequest::Delete { path } => {
                    let p = at(&path);
                    let _ = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
                    call.ok().await
                }
                FsRequest::Rename { path, to } => {
                    std::fs::rename(at(&path), at(&to)).unwrap();
                    call.ok().await
                }
            };
        }
    });
}

#[tokio::test]
async fn browsing_a_phones_files() {
    use linksy_core::fs::FsAnswer;
    let (phone, pc) = paired().await;
    let phone_id = phone.node.info().id;
    // Not sharing yet: a clear refusal.
    let err = pc.node.fs(&phone_id, FsRequest::List { path: "/".into() }, &[]).await.unwrap_err();
    assert!(err.to_string().contains("doesn't share"), "{err}");

    let root = phone.dir.path().join("storage");
    std::fs::create_dir_all(root.join("DCIM")).unwrap();
    std::fs::write(root.join("DCIM/a.jpg"), b"0123456789").unwrap();
    serve_files(&phone.node, root.clone());

    let FsAnswer::Entries(top) = pc.node.fs(&phone_id, FsRequest::List { path: "/".into() }, &[]).await.unwrap() else { panic!() };
    assert_eq!(top.iter().map(|e| (e.name.as_str(), e.dir)).collect::<Vec<_>>(), [("DCIM", true)]);
    let FsAnswer::Data(d) = pc.node.fs(&phone_id, FsRequest::Read { path: "/DCIM/a.jpg".into(), offset: 3, length: 4 }, &[]).await.unwrap() else { panic!() };
    assert_eq!(d, b"3456");
    pc.node.fs(&phone_id, FsRequest::Mkdir { path: "/Docs".into() }, &[]).await.unwrap();
    pc.node.fs(&phone_id, FsRequest::Write { path: "/Docs/n.txt".into(), offset: 0, length: 5, truncate: true }, b"hello").await.unwrap();
    pc.node.fs(&phone_id, FsRequest::Rename { path: "/Docs/n.txt".into(), to: "/Docs/m.txt".into() }, &[]).await.unwrap();
    assert_eq!(std::fs::read(root.join("Docs/m.txt")).unwrap(), b"hello");
    let FsAnswer::Entry(e) = pc.node.fs(&phone_id, FsRequest::Stat { path: "/Docs/m.txt".into() }, &[]).await.unwrap() else { panic!() };
    assert_eq!((e.size, e.dir), (5, false));
    pc.node.fs(&phone_id, FsRequest::Delete { path: "/Docs".into() }, &[]).await.unwrap();
    assert!(!root.join("Docs").exists());
    let missing = pc.node.fs(&phone_id, FsRequest::Stat { path: "/nope".into() }, &[]).await.unwrap_err();
    assert!(matches!(missing, Error::NotFound(_)));
}

#[tokio::test]
async fn features_travel_in_the_handshake() {
    let phone_features = vec!["files".to_string(), "camera".to_string()];
    let pf = phone_features.clone();
    let (phone, pc) = paired_with(move |c| {
        if c.name == "Alpha" {
            c.features = pf.clone();
        }
    })
    .await;
    let phone_id = phone.node.info().id;
    let seen = pc.node.devices().into_iter().find(|d| d.device.id == phone_id).unwrap();
    assert_eq!(seen.device.features, phone_features);
    // And the computer, which said nothing, has none.
    let pc_id = pc.node.info().id;
    assert!(phone.node.devices().into_iter().find(|d| d.device.id == pc_id).unwrap().device.features.is_empty());
}

#[tokio::test]
async fn idle_connection_closes_and_next_send_reconnects() {
    let (a, b) = paired_with(|c| c.idle_after = Duration::from_millis(400)).await;
    let mut ra = a.node.subscribe();
    let mut rb = b.node.subscribe();
    wait_for(&mut ra, |e| matches!(e, Event::Disconnected { .. }).then_some(())).await;
    assert!(a.node.devices().iter().all(|d| !d.connected));

    // Nothing is connected; sending dials Beta at its last known address.
    a.node.notify(&b.node.info().id, "Back", "").await.unwrap();
    let title = wait_for(&mut rb, |e| match e {
        Event::Notification { title, .. } => Some(title.clone()),
        _ => None,
    })
    .await;
    assert_eq!(title, "Back");
}

#[tokio::test]
async fn busy_connection_stays_open_past_idle_time() {
    let tweak = |c: &mut Config| c.idle_after = Duration::from_millis(300);
    let a = dev_with("Alpha", tweak).await;
    let b = dev_with("Beta", tweak).await;
    auto_answer(&a.node, true);
    // Beta accepts the pairing but leaves the file offer unanswered for a while.
    let mut rb = b.node.subscribe();
    let bn = b.node.clone();
    let mut pairing = b.node.subscribe();
    tokio::spawn(async move {
        let request = wait_for(&mut pairing, |e| match e {
            Event::PairingRequest { request, .. } => Some(*request),
            _ => None,
        })
        .await;
        bn.respond(request, true);
    });
    a.node.pair(b.node.local_addr().unwrap()).await.unwrap();

    let src = a.dir.path().join("wait.txt");
    std::fs::write(&src, b"hold on").unwrap();
    let mut ra = a.node.subscribe();
    a.node.send_files(&b.node.info().id, vec![src]).await.unwrap();
    let transfer = wait_for(&mut rb, |e| match e {
        Event::TransferRequest { transfer, .. } => Some(*transfer),
        _ => None,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(a.node.devices().iter().any(|d| d.connected), "closed while a prompt was open");
    b.node.respond(transfer, true);
    wait_for(&mut ra, |e| matches!(e, Event::TransferComplete { .. }).then_some(())).await;
    assert_eq!(std::fs::read(b.downloads().join("wait.txt")).unwrap(), b"hold on");
}

#[tokio::test]
async fn unreachable_device_fails_fast() {
    let (a, b) = paired_with(|c| c.idle_after = Duration::from_millis(300)).await;
    let mut ra = a.node.subscribe();
    let b_id = b.node.info().id;
    b.node.shutdown().await;
    wait_for(&mut ra, |e| matches!(e, Event::Disconnected { .. }).then_some(())).await;
    let started = std::time::Instant::now();
    let err = a.node.notify(&b_id, "Anyone?", "").await.unwrap_err();
    assert!(matches!(err, Error::Unreachable), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn restarted_device_reconnects_by_itself() {
    let (a, b) = paired_with(|c| c.stay_connected = true).await;
    let mut ra = a.node.subscribe();
    let b_dir = b.dir;
    b.node.shutdown().await;
    wait_for(&mut ra, |e| matches!(e, Event::Disconnected { .. }).then_some(())).await;

    // Same data dir → same identity and trust list; it dials Alpha's last address at startup.
    let mut c = config(b_dir.path(), "Beta");
    c.stay_connected = true;
    let b2 = Node::start(c).await.unwrap();
    let who = wait_for(&mut ra, |e| match e {
        Event::Connected { device } => Some(device.name.clone()),
        _ => None,
    })
    .await;
    assert_eq!(who, "Beta");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !b2.devices().iter().any(|d| d.connected && d.device.name == "Alpha") {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Beta sees Alpha as connected");
}

#[tokio::test]
async fn auto_accept_skips_the_prompt() {
    let (a, b) = paired().await;
    let mut s = b.node.settings();
    s.auto_accept = true;
    b.node.update_settings(s).await.unwrap();
    let mut rb = b.node.subscribe();
    let src = a.dir.path().join("quick.txt");
    std::fs::write(&src, b"no questions").unwrap();
    a.node.send_files(&b.node.info().id, vec![src]).await.unwrap();
    let asked = wait_for(&mut rb, |e| match e {
        Event::TransferRequest { .. } => Some(true),
        Event::TransferComplete { .. } => Some(false),
        _ => None,
    })
    .await;
    assert!(!asked, "auto-accept still asked");
    assert_eq!(std::fs::read(b.downloads().join("quick.txt")).unwrap(), b"no questions");
}

#[tokio::test]
async fn always_accept_from_one_device() {
    let (a, b) = paired().await;
    b.node.set_auto_accept(&a.node.info().id, true).unwrap();
    assert!(b.node.devices().iter().any(|d| d.device.id == a.node.info().id && d.auto_accept));
    let mut rb = b.node.subscribe();
    let src = a.dir.path().join("trusted.txt");
    std::fs::write(&src, b"from a friend").unwrap();
    a.node.send_files(&b.node.info().id, vec![src]).await.unwrap();
    let asked = wait_for(&mut rb, |e| match e {
        Event::TransferRequest { .. } => Some(true),
        Event::TransferComplete { .. } => Some(false),
        _ => None,
    })
    .await;
    assert!(!asked, "always accept still asked");
    assert_eq!(std::fs::read(b.downloads().join("trusted.txt")).unwrap(), b"from a friend");
}

/// Files sent one by one, a moment apart, are asked about once.
#[tokio::test]
async fn offers_together_are_one_prompt() {
    let (a, b) = paired().await;
    let mut rb = b.node.subscribe();
    let to = b.node.info().id;
    for name in ["one.txt", "two.txt", "three.txt"] {
        let src = a.dir.path().join(name);
        std::fs::write(&src, name).unwrap();
        a.node.send_files(&to, vec![src]).await.unwrap();
    }
    let (request, files) = wait_for(&mut rb, |e| match e {
        Event::TransferRequest { transfer, files, .. } => Some((*transfer, files.len())),
        _ => None,
    })
    .await;
    assert_eq!(files, 3);
    b.node.respond(request, true);
    let mut done = 0;
    wait_for(&mut rb, |e| match e {
        Event::TransferRequest { .. } => panic!("asked twice"),
        Event::TransferComplete { .. } => {
            done += 1;
            (done == 3).then_some(())
        }
        _ => None,
    })
    .await;
    for name in ["one.txt", "two.txt", "three.txt"] {
        assert_eq!(std::fs::read_to_string(b.downloads().join(name)).unwrap(), name);
    }
}

#[tokio::test]
async fn settings_persist_and_rename_reaches_peers() {
    let (a, b) = paired_with(|c| c.idle_after = Duration::from_millis(300)).await;
    let mut ra = a.node.subscribe();
    let mut s = b.node.settings();
    s.name = "  Beta   Two ".into();
    let applied = b.node.update_settings(s).await.unwrap();
    assert_eq!(applied.name, "Beta Two");
    assert_eq!(b.node.info().name, "Beta Two");
    // Still connected: Alpha hears about it right away.
    let renamed = wait_for(&mut ra, |e| match e {
        Event::DeviceUpdated { device } => Some(device.name.clone()),
        _ => None,
    })
    .await;
    assert_eq!(renamed, "Beta Two");
    assert!(a.node.devices().iter().any(|d| d.device.name == "Beta Two"));
    wait_for(&mut ra, |e| matches!(e, Event::Disconnected { .. }).then_some(())).await;

    // The next connection's handshake carries the new name.
    a.node.notify(&b.node.info().id, "Hi", "").await.unwrap();
    let name = wait_for(&mut ra, |e| match e {
        Event::Connected { device } => Some(device.name.clone()),
        _ => None,
    })
    .await;
    assert_eq!(name, "Beta Two");

    // And it survives a restart.
    let b_dir = b.dir;
    b.node.shutdown().await;
    let b2 = Node::start(config(b_dir.path(), "Beta")).await.unwrap();
    assert_eq!(b2.settings().name, "Beta Two");
}

#[tokio::test]
async fn link_stays_open_when_either_side_wants_it() {
    let a = dev_with("Alpha", |c| c.idle_after = Duration::from_millis(200)).await;
    let b = dev_with("Beta", |c| {
        c.idle_after = Duration::from_millis(200);
        c.stay_connected = true;
    })
    .await;
    auto_answer(&a.node, true);
    auto_answer(&b.node, true);
    let mut ra = a.node.subscribe();
    a.node.pair(b.node.local_addr().unwrap()).await.unwrap();
    // Alpha connects on demand, but Beta asked to stay connected: no idle close.
    let closed = tokio::time::timeout(Duration::from_millis(1200), async {
        loop {
            if let Ok(Event::Disconnected { .. }) = ra.recv().await {
                return;
            }
        }
    })
    .await;
    assert!(closed.is_err(), "link closed although Beta stays connected");
}

#[tokio::test]
async fn several_addresses_give_one_connection() {
    // Both nodes listen on every interface, and Alpha knows Beta by three addresses, as
    // mDNS reports on a real network. Dialing them all at once must open one link only:
    // duplicates close each other and can leave no connection at all.
    let any = |c: &mut Config| {
        c.bind = IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED);
        c.idle_after = Duration::from_millis(400);
    };
    let (a, b) = paired_with(any).await;
    let mut ra = a.node.subscribe();
    wait_for(&mut ra, |e| matches!(e, Event::Disconnected { .. }).then_some(())).await;
    let port = b.node.local_addr().unwrap().port();
    let addrs = vec![
        std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        std::net::SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port)),
        std::net::SocketAddr::from((Ipv4Addr::LOCALHOST.to_ipv6_mapped(), port)),
    ];
    a.node.device_found(b.node.info(), addrs);

    let mut rb = b.node.subscribe();
    a.node.notify(&b.node.info().id, "once", "").await.unwrap();
    // Everything Beta reports during the reconnect, until things settle.
    let mut events = Vec::new();
    while let Ok(Ok(e)) = tokio::time::timeout(Duration::from_millis(250), rb.recv()).await {
        events.push(e);
    }
    assert!(events.iter().any(|e| matches!(e, Event::Notification { .. })), "{events:?}");
    let connects = events.iter().filter(|e| matches!(e, Event::Connected { .. })).count();
    assert_eq!(connects, 1, "{events:?}");
}

/// An invite from `node` that points at its loopback address (test nodes listen there).
fn local_invite(node: &Node) -> linksy_core::invite::Invite {
    let mut invite = node.create_invite().unwrap();
    invite.addrs = vec![node.local_addr().unwrap()];
    invite
}

#[tokio::test]
async fn qr_invite_pairs_without_asking() {
    let a = dev("Alpha").await;
    let b = dev("Beta").await;
    let (mut ra, mut rb) = (a.node.subscribe(), b.node.subscribe());
    let invite = local_invite(&b.node);
    let device = a.node.pair_with_invite(&invite.to_uri()).await.unwrap();
    assert_eq!(device.name, "Beta");
    for rx in [&mut ra, &mut rb] {
        let asked = wait_for(rx, |e| match e {
            Event::PairingRequest { .. } => Some(true),
            Event::Paired { .. } => Some(false),
            _ => None,
        })
        .await;
        assert!(!asked, "an invite pairing asked for a code");
    }
    assert!(a.node.trusted().iter().any(|t| t.device.name == "Beta"));
    assert!(b.node.trusted().iter().any(|t| t.device.name == "Alpha"));
}

#[tokio::test]
async fn invite_works_once_and_bad_tokens_ask_instead() {
    let a = dev("Alpha").await;
    let b = dev("Beta").await;
    let c = dev("Gamma").await;
    let invite = local_invite(&b.node);
    a.node.pair_with_invite(&invite.to_uri()).await.unwrap();

    // The same code again (say, a photo of it): Beta falls back to asking, and says no.
    auto_answer(&b.node, false);
    let mut rb = b.node.subscribe();
    let reused = c.node.pair_with_invite(&invite.to_uri()).await;
    assert!(reused.is_err());
    wait_for(&mut rb, |e| matches!(e, Event::PairingRequest { device, .. } if device.name == "Gamma").then_some(())).await;
    assert!(!b.node.trusted().iter().any(|t| t.device.name == "Gamma"));

    // A cancelled invite is dead too.
    let fresh = local_invite(&b.node);
    b.node.cancel_invite();
    assert!(c.node.pair_with_invite(&fresh.to_uri()).await.is_err());

    // And an invite whose id isn't the device at that address is refused before pairing.
    let mut wrong = local_invite(&b.node);
    wrong.id = a.node.info().id;
    assert!(c.node.pair_with_invite(&wrong.to_uri()).await.is_err());
}

#[tokio::test]
async fn screen_session_carries_media_and_control() {
    let (a, b) = paired().await;
    let mut incoming = b.node.incoming_sessions().expect("receiver available once");
    assert!(b.node.incoming_sessions().is_none());
    let mut ra = a.node.subscribe();

    let mut viewer = a.node.request_session(&b.node.info().id, SessionKind::Screen).await.unwrap();
    assert!(!viewer.sharing);
    let mut sharer = incoming.recv().await.unwrap();
    assert!(sharer.sharing);
    assert_eq!((sharer.peer.name.as_str(), sharer.kind), ("Alpha", SessionKind::Screen));

    // Media one way, control the other.
    sharer.send.write_all(b"\x00\x00\x00\x01frame").await.unwrap();
    let mut media = [0u8; 9];
    viewer.recv.read_exact(&mut media).await.unwrap();
    assert_eq!(&media, b"\x00\x00\x00\x01frame");
    viewer.send.write_all(b"key").await.unwrap();
    let mut control = [0u8; 3];
    sharer.recv.read_exact(&mut control).await.unwrap();
    assert_eq!(&control, b"key");

    // Either side dropping ends it on both.
    let id = viewer.id;
    drop(sharer);
    let mut rest = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), AsyncReadExt::read_to_end(&mut viewer.recv, &mut rest)).await;
    drop(viewer);
    let ended = wait_for(&mut ra, |e| match e {
        Event::SessionEnded { session } => Some(*session),
        _ => None,
    })
    .await;
    assert_eq!(ended, id);
}

#[tokio::test]
async fn input_session_carries_touchpad_and_text_to_the_receiver() {
    let (phone, pc) = paired().await;
    let mut incoming = pc.node.incoming_sessions().unwrap();
    let mut remote = phone.node.request_session(&pc.node.info().id, SessionKind::Input).await.unwrap();
    let mut driven = incoming.recv().await.unwrap();
    // The computer is the side that "shares" (its input), and the phone sends the control.
    assert!(driven.sharing && !remote.sharing);
    let sent = [ControlMsg::PointerBy { dx: 3.5, dy: -1.0 }, ControlMsg::Text { text: "héllo".into() }];
    for m in &sent {
        remote.send.write_all(&linksy_core::protocol::control_frame(m)).await.unwrap();
    }
    remote.send.finish().unwrap();
    let mut bytes = Vec::new();
    AsyncReadExt::read_to_end(&mut driven.recv, &mut bytes).await.unwrap();
    let mut got = Vec::new();
    linksy_core::protocol::read_control_frames(&bytes[..], |m| got.push(m));
    assert_eq!(got, sent);
}

#[tokio::test]
async fn declined_screen_session_starts_nothing() {
    let a = dev("Alpha").await;
    let b = dev("Beta").await;
    auto_answer(&a.node, true);
    let mut rb = b.node.subscribe();
    let bn = b.node.clone();
    // Beta pairs, then says no to the screen.
    tokio::spawn(async move {
        loop {
            match rb.recv().await {
                Ok(Event::PairingRequest { request, .. }) => {
                    bn.respond(request, true);
                }
                Ok(Event::SessionRequest { request, .. }) => {
                    bn.respond(request, false);
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    a.node.pair(b.node.local_addr().unwrap()).await.unwrap();
    let mut incoming = b.node.incoming_sessions().unwrap();
    let err = a.node.request_session(&b.node.info().id, SessionKind::Screen).await.unwrap_err();
    assert!(matches!(err, Error::Declined), "{err:?}");
    assert!(incoming.try_recv().is_err());
}

#[tokio::test]
async fn display_session_has_the_requester_share() {
    let (a, b) = paired().await;
    let mut incoming = b.node.incoming_sessions().unwrap();
    let mut sharer = a.node.request_session(&b.node.info().id, SessionKind::Display).await.unwrap();
    assert!(sharer.sharing, "the requester of a second display sends its screen");
    assert_eq!(sharer.size, Some((2400, 1080)), "the display's shape comes with the accept");
    let mut display = incoming.recv().await.unwrap();
    assert!(!display.sharing);
    assert_eq!(display.kind, SessionKind::Display);
    sharer.send.write_all(b"frame").await.unwrap();
    let mut got = [0u8; 5];
    display.recv.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"frame");
}

#[tokio::test]
async fn unsupported_sessions_say_so() {
    let a = dev("Alpha").await;
    let b = dev("Beta").await;
    auto_answer(&a.node, true);
    let mut rb = b.node.subscribe();
    let bn = b.node.clone();
    // Beta pairs, then says it can't be controlled (rather than "no").
    tokio::spawn(async move {
        loop {
            match rb.recv().await {
                Ok(Event::PairingRequest { request, .. }) => {
                    bn.respond(request, true);
                }
                Ok(Event::SessionRequest { request, .. }) => {
                    bn.decline_unsupported(request);
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    a.node.pair(b.node.local_addr().unwrap()).await.unwrap();
    let _incoming = b.node.incoming_sessions();
    let err = a.node.request_session(&b.node.info().id, SessionKind::Control).await.unwrap_err();
    assert!(matches!(err, Error::Unsupported), "{err:?}");
}

/// Away from home (docs/protocol.md §4.13): after the direct connection goes, the two reach each
/// other through iroh, Linksy's own handshake runs inside, both see the link as away, and
/// screens are refused until allowed for that device.
#[tokio::test]
async fn away_from_home_connects_and_keeps_screens_local() {
    use linksy_core::Link;
    let tweak = |c: &mut Config| {
        c.idle_after = Duration::from_millis(1500);
        c.away = true;
    };
    let (a, b) = paired_with(tweak).await;
    let (ida, idb) = (a.node.info().id, b.node.info().id);
    // Without a network address to dial, iroh has nothing to try: skip on such machines.
    if b.node.info().away.is_none_or(|w| w.addrs.is_empty()) {
        return eprintln!("no network address: skipped");
    }
    // Met on the same network: each knows how to reach the other away.
    let until = |n: &linksy_core::Node, id| n.devices().into_iter().find(|d| d.device.id == id).and_then(|d| d.away_until);
    assert!(until(&a.node, idb).is_some());
    let mut ra = a.node.subscribe();
    wait_for(&mut ra, |e| matches!(e, Event::Disconnected { .. }).then_some(())).await;

    a.node.connect_away(&idb).await.expect("away connection");
    assert_eq!(a.node.link(&idb), Some(Link::Away));
    let mut rb = b.node.subscribe();
    a.node.send_clipboard(&idb, "from far away").await.unwrap();
    let text = wait_for(&mut rb, |e| match e {
        Event::Clipboard { text, .. } => Some(text.clone()),
        _ => None,
    })
    .await;
    assert_eq!(text, "from far away");
    assert_eq!(b.node.link(&ida), Some(Link::Away));

    // A screen from away: refused without asking Beta's user...
    let _incoming = b.node.incoming_sessions();
    let err = a.node.request_session(&idb, SessionKind::Screen).await.unwrap_err();
    assert!(matches!(err, Error::NotFromAway), "{err:?}");
    // ...until Beta allows everything from away for Alpha.
    b.node.set_away_all(&ida, true).unwrap();
    auto_answer(&b.node, true);
    a.node.request_session(&idb, SessionKind::Screen).await.expect("allowed from away");
}

#[tokio::test]
async fn not_reachable_from_away_without_its_address() {
    // Away from home off on both: neither learns the other's away address.
    let (a, b) = paired().await;
    let idb = b.node.info().id;
    let d = a.node.devices().into_iter().find(|d| d.device.id == idb).unwrap();
    assert_eq!(d.away_until, None);
}

#[tokio::test]
async fn terminals_are_judged_before_anyone_is_asked() {
    use linksy_core::{TerminalOpen, Verdict, protocol::control_frame};
    let (a, b) = paired().await;
    let idb = b.node.info().id;
    // Beta runs terminals: a wrong code is refused, the right one opens at once.
    let mut calls = b.node.incoming_terminals().unwrap();
    tokio::spawn(async move {
        while let Some(call) = calls.recv().await {
            let verdict = if call.open.code == "482913" { Verdict::Accept { terminal: "t1".into() } } else { Verdict::Decline { reason: "code".into(), tries_left: Some(4) } };
            call.answer(verdict);
        }
    });
    let mut sessions = b.node.incoming_sessions().unwrap();
    let open = |code: &str| TerminalOpen { code: code.into(), cols: 80, rows: 24, resume: None };
    match a.node.request_terminal(&idb, open("000000")).await {
        Err(Error::Refused { reason, tries_left }) => assert_eq!((reason.as_str(), tries_left), ("code", Some(4))),
        other => panic!("{other:?}"),
    }
    let (mut session, id) = a.node.request_terminal(&idb, open("482913")).await.expect("terminal");
    assert_eq!(id, "t1");
    let mut theirs = tokio::time::timeout(Duration::from_secs(5), sessions.recv()).await.unwrap().unwrap();
    assert_eq!(theirs.kind, SessionKind::Terminal);
    session.send.write_all(&control_frame(&ControlMsg::TermInput { data: "ls\r".into() })).await.unwrap();
    let mut len = [0u8; 4];
    theirs.recv.read_exact(&mut len).await.unwrap();
    theirs.send.write_all(b"$ ").await.unwrap();
    let mut out = [0u8; 2];
    session.recv.read_exact(&mut out).await.unwrap();
    assert_eq!(&out, b"$ ");
}

#[tokio::test]
async fn locate_asks_once_and_hears_back() {
    let (a, b) = paired().await;
    let (ida, idb) = (a.node.info().id, b.node.info().id);
    let (mut ra, mut rb) = (a.node.subscribe(), b.node.subscribe());
    a.node.locate(&idb).await.unwrap();
    wait_for(&mut rb, |e| matches!(e, Event::LocateRequest { .. }).then_some(())).await;
    b.node.send_location(&ida, -18.8792, 47.5079, Some(12.0), 1_792_000_000_000).await.unwrap();
    let (lat, lon, acc) = wait_for(&mut ra, |e| match e {
        Event::Location { lat, lon, accuracy_m, .. } => Some((*lat, *lon, *accuracy_m)),
        _ => None,
    })
    .await;
    assert_eq!((lat, lon, acc), (-18.8792, 47.5079, Some(12.0)));
    // Nonsense positions never reach the app.
    b.node.send_location(&ida, 123.0, 0.0, None, 0).await.unwrap();
    b.node.refuse_location(&ida, "Location is off").await.unwrap();
    let next = wait_for(&mut ra, |e| match e {
        Event::Location { .. } => Some("location"),
        Event::LocationRefused { .. } => Some("refused"),
        _ => None,
    })
    .await;
    assert_eq!(next, "refused");
}

/// Whether `f` becomes true within 10 s.
async fn until(f: impl Fn() -> bool) -> bool {
    for _ in 0..100 {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Private discovery on the real network (multicast, so not in CI): once keys are shared, a
/// stranger doesn't see the device, its paired device still finds it by its tag, and an
/// open invite (pairing mode) shows it to strangers again.
#[tokio::test]
#[ignore = "needs multicast on this machine's network"]
async fn private_discovery_hides_from_strangers_only() {
    let on = |c: &mut Config| {
        c.discovery = true;
        c.bind = IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED);
    };
    let (a, b) = paired_with(on).await;
    stored_key(&b, &a, None).await;
    stored_key(&a, &b, None).await;
    let stranger = dev_with("Stranger", on).await;
    let (a_id, b_id) = (a.node.info().id, b.node.info().id);
    let sees = |dev: &Dev, id| dev.node.devices().iter().any(|d| d.device.id == id && d.nearby);
    assert!(until(|| sees(&b, a_id)).await, "the paired device doesn't find it by its tag");
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!sees(&stranger, a_id) && !sees(&stranger, b_id), "a stranger sees it");

    a.node.create_invite().unwrap();
    assert!(until(|| sees(&stranger, a_id)).await, "pairing mode doesn't show it");
    a.node.cancel_invite();
    assert!(until(|| !sees(&stranger, a_id)).await, "pairing mode doesn't end");
}

/// The computer says what's plugged in by USB; the phone asks to use the cable (§4.11).
#[tokio::test]
async fn usb_plugged_and_usb_connect_travel() {
    let (computer, phone) = paired().await;
    let mut rp = phone.node.subscribe();
    let mut rc = computer.node.subscribe();
    let plugged = vec![linksy_core::PluggedPhone { key: "R5CX".into(), manufacturer: "samsung".into(), product: "SM-S921B".into(), state: "plugged".into() }];
    computer.node.usb_plugged(&phone.node.info().id, plugged.clone()).await.unwrap();
    let got = wait_for(&mut rp, |e| match e {
        Event::UsbPlugged { phones, .. } => Some(phones.clone()),
        _ => None,
    })
    .await;
    assert_eq!(got, plugged);
    phone.node.usb_connect(&computer.node.info().id, "R5CX").await.unwrap();
    let key = wait_for(&mut rc, |e| match e {
        Event::UsbConnect { key, .. } => Some(key.clone()),
        _ => None,
    })
    .await;
    assert_eq!(key, "R5CX");
}

/// What a phone's and a computer's apps do for a direct link (docs/protocol.md §4.17), with
/// loopback for the Wi-Fi link: the host is reachable where it already is.
fn fake_direct_app(node: &Node) {
    let mut rx = node.subscribe();
    let node = node.clone();
    tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            let node = node.clone();
            tokio::spawn(async move {
                match ev {
                    Event::DirectWanted { id } => node.direct_send(&id, DirectStep::Invite { address: "02:00:00:00:00:01".into() }).await.unwrap(),
                    Event::Direct { from, step: DirectStep::Invite { .. } } => {
                        node.direct_send(&from.id, DirectStep::Listening { address: "02:00:00:00:00:02".into() }).await.unwrap()
                    }
                    Event::Direct { from, step: DirectStep::Listening { .. } } => {
                        node.direct_send(&from.id, DirectStep::Connecting).await.unwrap();
                        let port = node.local_addr().unwrap().port();
                        node.direct_send(&from.id, DirectStep::Up { ip: Ipv4Addr::LOCALHOST.into(), port }).await.unwrap();
                    }
                    Event::Direct { from, step: DirectStep::Up { ip, port } } => node.direct_dial(&from.id, (ip, port).into()).await.unwrap(),
                    _ => {}
                }
            });
        }
    });
}

#[tokio::test]
async fn direct_link_comes_up_carries_files_and_ends_on_both() {
    let (a, b) = paired_with(|c| c.features = vec!["files".into(), "direct_host".into(), "direct_join".into()]).await;
    let (a_id, b_id) = (a.node.info().id, b.node.info().id);
    fake_direct_app(&a.node);
    fake_direct_app(&b.node);
    let mut on_a = a.node.subscribe();
    let mut on_b = b.node.subscribe();

    a.node.direct_link(&b_id).await.unwrap();
    wait_for(&mut on_a, |e| matches!(e, Event::DirectLinked { id } if *id == b_id).then_some(())).await;
    wait_for(&mut on_b, |e| matches!(e, Event::DirectLinked { id } if *id == a_id).then_some(())).await;

    // Both ways over it
    let src = tempfile::tempdir().unwrap();
    let file = src.path().join("film.bin");
    std::fs::write(&file, vec![7u8; 3 << 20]).unwrap();
    for (from, to, rx) in [(&a, &b, &mut on_a), (&b, &a, &mut on_b)] {
        let id = from.node.send_files(&to.node.info().id, vec![file.clone()]).await.unwrap();
        wait_for(rx, |e| matches!(e, Event::TransferComplete { transfer, .. } if *transfer == id).then_some(())).await;
        assert_eq!(std::fs::read(to.downloads().join("film.bin")).unwrap().len(), 3 << 20);
        std::fs::remove_file(to.downloads().join("film.bin")).unwrap();
    }

    // Ended on one side: gone on both, and the usual connection still works
    a.node.direct_end(&b_id, None).await;
    wait_for(&mut on_a, |e| matches!(e, Event::DirectEnded { id, .. } if *id == b_id).then_some(())).await;
    wait_for(&mut on_b, |e| matches!(e, Event::DirectEnded { id, .. } if *id == a_id).then_some(())).await;
    a.node.notify(&b_id, "still", "here").await.unwrap();
    wait_for(&mut on_b, |e| matches!(e, Event::Notification { .. }).then_some(())).await;
}

#[tokio::test]
async fn direct_link_needs_a_host_and_a_joiner() {
    let (a, b) = paired_with(|c| c.features = vec!["files".into(), "direct_join".into()]).await;
    assert!(matches!(a.node.direct_link(&b.node.info().id).await, Err(Error::Unsupported)));
    // A bad step never leaves
    let step = DirectStep::Up { ip: "8.8.8.8".parse().unwrap(), port: 47200 };
    assert!(a.node.direct_send(&b.node.info().id, step).await.is_err());
}
