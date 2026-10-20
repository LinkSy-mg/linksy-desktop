//! Development aid: act as a phone toward paired computers, to try phone features before
//! the Android app has them. Reuses a device identity (its data dir), reports itself as a
//! phone, sends its battery to every paired device, and prints what it's asked to do.
//!
//! cargo run -p linksy-core --example fake_phone -- --data-dir DIR [--port N] [--name NAME]
//!     [--battery PERCENT] [--charging] [--seconds N]

use std::time::Duration;

use linksy_core::{Battery, Config, DeviceKind, Event, Node};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let (mut data_dir, mut port, mut name) = (None, 0, "Test Phone".to_string());
    let (mut level, mut charging, mut seconds) = (None, false, 60);
    while let Some(a) = args.next() {
        let mut value = || args.next().expect("option needs a value");
        match a.as_str() {
            "--data-dir" => data_dir = Some(value()),
            "--port" => port = value().parse().expect("--port N"),
            "--name" => name = value(),
            "--battery" => level = Some(value().parse::<u8>().expect("--battery PERCENT")),
            "--charging" => charging = true,
            "--seconds" => seconds = value().parse().expect("--seconds N"),
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
    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            e = events.recv() => match e {
                Ok(Event::Ring { from, on }) => println!("{} {}", if on { "RING from" } else { "ring stopped by" }, from.name),
                Ok(Event::OpenUrl { from, url }) => println!("open {url} (from {})", from.name),
                Ok(_) => {}
                Err(_) => break,
            },
        }
    }
    node.shutdown().await;
}
