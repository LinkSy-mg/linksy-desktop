//! Development aid: act as a phone toward paired computers, to try phone features without
//! a real phone. Reuses a device identity (its data dir), reports itself as a
//! phone, sends its battery and a notification to every paired device, and prints what
//! it's asked to do.
//!
//! cargo run -p linksy-core --example fake_phone -- --data-dir DIR [--port N] [--name NAME]
//!     [--battery PERCENT] [--charging] [--notify "TITLE" "BODY"] [--display FILE]
//!     [--media "TITLE" "ARTIST"] [--watch-media] [--call SECONDS] [--leave-after SECONDS]
//!     [--import PATH] [--seconds N]
//!
//! `--display FILE` accepts "Use as Second Display" and saves the H.264 it receives.
//! `--media "TITLE" "ARTIST"` plays a song (as far as the computer can tell);
//! `--watch-media` prints what the computer plays; `--press play-pause|next|previous`
//! presses that button on the first player it hears about. `--call SECONDS` rings, answers
//! two seconds later, and hangs up after that many seconds. It checks in when a computer
//! asks (lock when away); `--leave-after SECONDS` stops that, as if it walked away.
//! `--import PATH` imports photos (a file or folder) into the computer's pictures.

use std::time::Duration;

use linksy_core::{Battery, Config, DeviceKind, Event, MediaPlayer, MediaState, Node, NotificationButton, RemoteNotification};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let (mut data_dir, mut port, mut name) = (None, 0, "Test Phone".to_string());
    let (mut level, mut charging, mut seconds) = (None, false, 60);
    let mut notify = None;
    let mut display: Option<std::path::PathBuf> = None;
    let (mut media, mut watch_media) = (None, false);
    let mut press: Option<linksy_core::MediaCommand> = None;
    let mut call: Option<u64> = None;
    let mut leave_after: Option<u64> = None;
    let mut import: Option<std::path::PathBuf> = None;
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
            "--watch-media" => watch_media = true,
            "--call" => call = Some(value().parse().expect("--call SECONDS")),
            "--import" => import = Some(value().into()),
            "--leave-after" => leave_after = Some(value().parse().expect("--leave-after SECONDS")),
            "--press" => {
                press = Some(match value().as_str() {
                    "next" => linksy_core::MediaCommand::Next,
                    "previous" => linksy_core::MediaCommand::Previous,
                    _ => linksy_core::MediaCommand::PlayPause,
                })
            }
            "--media" => {
                let title = value();
                media = Some((title, value()));
            }
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
    let seq = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let mut playing = true;
    let song = |title: &str, artist: &str, playing: bool| MediaState {
        seq: seq(),
        players: vec![MediaPlayer {
            id: "music".into(),
            app: "Music".into(),
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            playing,
            position_ms: Some(42_000),
            length_ms: Some(215_000),
            can_next: true,
            can_previous: true,
            can_seek: true,
            art: None,
        }],
    };
    for t in node.trusted() {
        if let Some((title, artist)) = &media {
            let r = node.send_media_state(&t.device.id, &song(title, artist, playing)).await;
            println!("media to {}: {}", t.device.name, r.map(|_| "sent".to_string()).unwrap_or_else(|e| e.to_string()));
        }
        if let Some(path) = &import {
            let r = node.import_files(&t.device.id, vec![path.clone()]).await;
            println!("import to {}: {}", t.device.name, r.map(|_| "started".to_string()).unwrap_or_else(|e| e.to_string()));
        }
        if watch_media {
            let _ = node.watch_media(&t.device.id, true).await;
        }
    }
    if let Some(seconds) = call {
        let node = node.clone();
        tokio::spawn(async move {
            use linksy_core::CallState::*;
            for (state, wait) in [(Ringing, 2), (Active, seconds), (Idle, 0)] {
                for t in node.trusted() {
                    let _ = node.send_call_state(&t.device.id, state, Some("Alice")).await;
                }
                println!("call: {state:?}");
                tokio::time::sleep(Duration::from_secs(wait)).await;
            }
        });
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
                Ok(Event::TransferComplete { .. }) if import.is_some() => println!("import done"),
                Ok(Event::TransferFailed { reason, .. }) if import.is_some() => println!("import failed: {reason}"),
                Ok(Event::OpenUrl { from, url }) => println!("open {url} (from {})", from.name),
                Ok(Event::NotificationReply { from, key, text }) => println!("REPLY to {key} from {}: {text}", from.name),
                Ok(Event::NotificationAction { key, action, .. }) => println!("ACTION {action} on {key}"),
                Ok(Event::NotificationDismissed { key, .. }) => println!("DISMISSED {key}"),
                Ok(Event::HeartbeatWatch { from, on: true, every_s }) => {
                    println!("{} asks for a heartbeat every {every_s} s", from.name);
                    let node = node.clone();
                    let started = std::time::Instant::now();
                    tokio::spawn(async move {
                        loop {
                            if leave_after.is_some_and(|s| started.elapsed().as_secs() >= s) {
                                println!("walked away");
                                break;
                            }
                            let _ = node.heartbeat(&from.id).await;
                            tokio::time::sleep(Duration::from_secs(every_s.into())).await;
                        }
                    });
                }
                Ok(Event::MediaCommand { from, player, command }) => {
                    println!("MEDIA {command:?} on {player} from {}", from.name);
                    // Act like a player: play/pause flips, and the computer hears about it.
                    if let (Some((title, artist)), linksy_core::MediaCommand::PlayPause | linksy_core::MediaCommand::Pause | linksy_core::MediaCommand::Play) = (&media, command) {
                        playing = match command {
                            linksy_core::MediaCommand::Play => true,
                            linksy_core::MediaCommand::Pause => false,
                            _ => !playing,
                        };
                        let _ = node.send_media_state(&from.id, &song(title, artist, playing)).await;
                    }
                }
                Ok(Event::MediaState { from, state }) => {
                    for p in &state.players {
                        println!("{} plays: {} – {} ({}) [{}]", from.name, p.artist, p.title, p.app, if p.playing { "playing" } else { "paused" });
                    }
                    if state.players.is_empty() {
                        println!("{} plays nothing", from.name);
                    }
                    if let (Some(command), Some(p)) = (press.take(), state.players.first()) {
                        println!("pressing {command:?} on {}", p.app);
                        let _ = node.media_command(&from.id, &p.id, command).await;
                    }
                }
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
