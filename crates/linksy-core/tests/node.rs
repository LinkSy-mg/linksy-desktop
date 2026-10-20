//! End-to-end tests: two nodes on loopback, discovery off, consent answered by a helper.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use linksy_core::{Config, Error, Event, Node, SessionKind};
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
