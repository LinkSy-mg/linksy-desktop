//! Development aid: act as a phone toward paired computers, to try phone features without
//! a real phone. Reuses a device identity (its data dir), reports itself as a
//! phone, sends its battery and a notification to every paired device, and prints what
//! it's asked to do.
//!
//! cargo run -p linksy-core --example fake_phone -- --data-dir DIR [--port N] [--name NAME]
//!     [--battery PERCENT] [--charging] [--notify "TITLE" "BODY"] [--display FILE] [--seconds N]
//!
//! `--display FILE` accepts "Use as Second Display" and saves the H.264 it receives.

use std::time::Duration;

use linksy_core::{Battery, Config, DeviceKind, Event, Node, NotificationButton, RemoteNotification};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let (mut data_dir, mut port, mut name) = (None, 0, "Test Phone".to_string());
    let (mut level, mut charging, mut seconds) = (None, false, 60);
    let mut notify = None;
    let mut display: Option<std::path::PathBuf> = None;
    while let Some(a) = args.next() {
        let mut value = || args.next().expect("option needs a value");
        match a.as_str() {
            "--data-dir" => data_dir = Some(value()),
            "--port" => port = value().parse().expect("--port N"),
            "--name" => name = value(),
            "--battery" => level = Some(value().parse::<u8>().expect("--battery PERCENT")),
            "--charging" => charging = true,
            "--seconds" => seconds = value().parse().expect("--seconds N"),
            "--display" => display = Some(value().into()),
            "--notify" => {
                let title = value();
                notify = Some((title, value()));
            }
            other => panic!("unknown option {other}"),
        }
    }
    let data_dir = std::path::PathBuf::from(data_dir.expect("--data-dir DIR"));
    let mut config = Config::new(data_dir.clone(), data_dir.join("downloads"), name);
    config.kind = DeviceKind::Phone;
    config.port = port;
    let node = Node::start(config).await.expect("start");
    let mut events = node.subscribe();
    println!("{} ({}) up", node.info().name, node.info().id.short());

    if let Some(level) = level {
        for t in node.trusted() {
            match node.send_battery(&t.device.id, Battery { level, charging }).await {
                Ok(()) => println!("battery {level}% sent to {}", t.device.name),
                Err(e) => println!("battery to {}: {e}", t.device.name),
            }
        }
    }
    if let Some((title, body)) = notify {
        let n = RemoteNotification {
            key: "fake|messages|1".into(),
            app: "org.example.messages".into(),
            app_name: "Messages".into(),
            title,
            body,
            can_reply: true,
            actions: vec![NotificationButton { id: "read".into(), label: "Mark as Read".into() }],
            icon: None,
        };
        for t in node.trusted() {
            match node.post_notification(&t.device.id, n.clone()).await {
                Ok(()) => println!("notification sent to {}", t.device.name),
                Err(e) => println!("notification to {}: {e}", t.device.name),
            }
        }
    }
    if let Some(path) = display.clone() {
        let mut sessions = node.incoming_sessions().expect("sessions");
        tokio::spawn(async move {
            while let Some(mut s) = sessions.recv().await {
                let mut file = tokio::fs::File::create(&path).await.expect("display file");
                let n = tokio::io::copy(&mut s.recv, &mut file).await.unwrap_or(0);
                println!("display from {} ended: {n} bytes saved", s.peer.name);
            }
        });
    }
    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            e = events.recv() => match e {
                Ok(Event::Ring { from, on }) => println!("{} {}", if on { "RING from" } else { "ring stopped by" }, from.name),
                Ok(Event::OpenUrl { from, url }) => println!("open {url} (from {})", from.name),
                Ok(Event::NotificationReply { from, key, text }) => println!("REPLY to {key} from {}: {text}", from.name),
                Ok(Event::NotificationAction { key, action, .. }) => println!("ACTION {action} on {key}"),
                Ok(Event::NotificationDismissed { key, .. }) => println!("DISMISSED {key}"),
                Ok(Event::SessionRequest { request, from, kind }) if display.is_some() => {
                    println!("{kind:?} requested by {}: accepting", from.name);
                    node.respond(request, true);
                }
                Ok(_) => {}
                Err(_) => break,
            },
        }
    }
    node.shutdown().await;
}
