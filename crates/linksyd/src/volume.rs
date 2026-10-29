//! This computer's volume for other devices (a phone's slider, docs/protocol.md §4.5): the
//! default sound output's level and mute, read and set with WirePlumber's `wpctl`, or
//! PulseAudio's `pactl` where there's no wpctl. While a device watches it, it's read every
//! second and sent when it changed, whatever changed it (the keyboard, GNOME's slider).
//!
//! Nothing runs while no device is watching.

use std::collections::HashSet;
use std::time::Duration;

use linksy_core::{Event, Fingerprint, Node, VolumeState};
use tokio::process::Command;
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tool {
    Wpctl,
    Pactl,
}

/// Whether this computer can share its volume: a tool to do it is installed (from the
/// Flatpak, the computer's).
pub fn available() -> bool {
    tool().is_some()
}

fn tool() -> Option<Tool> {
    static TOOL: std::sync::OnceLock<Option<Tool>> = std::sync::OnceLock::new();
    *TOOL.get_or_init(|| {
        if linksy_host::installed("wpctl") {
            Some(Tool::Wpctl)
        } else if linksy_host::installed("pactl") {
            Some(Tool::Pactl)
        } else {
            None
        }
    })
}

pub async fn run(node: Node) {
    let Some(tool) = tool() else { return };
    let mut events = node.subscribe();
    let mut watchers: HashSet<Fingerprint> = HashSet::new();
    let mut last: Option<(f64, bool)> = None;
    let mut every = tokio::time::interval(Duration::from_secs(1));
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let send_now = tokio::select! {
            event = events.recv() => match event {
                Ok(Event::VolumeWatch { from, on: true }) => {
                    watchers.insert(from.id);
                    // The newcomer gets it even when it didn't change
                    last = None;
                    true
                }
                Ok(Event::VolumeWatch { from, on: false }) => {
                    watchers.remove(&from.id);
                    false
                }
                Ok(Event::Disconnected { id }) => {
                    watchers.remove(&id);
                    false
                }
                Ok(Event::VolumeSet { level, muted, .. }) => {
                    set(tool, level, muted).await;
                    true
                }
                Ok(_) | Err(RecvError::Lagged(_)) => false,
                Err(RecvError::Closed) => break,
            },
            _ = every.tick() => true,
        };
        if !send_now || watchers.is_empty() {
            continue;
        }
        let Some(now) = read(tool).await else { continue };
        if last.is_some_and(|l| same(l, now)) {
            continue;
        }
        last = Some(now);
        let seq = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let state = VolumeState { seq, level: now.0.clamp(0.0, 1.0), muted: now.1 };
        for id in watchers.clone() {
            if node.send_volume_state(&id, &state).await.is_err() {
                watchers.remove(&id);
            }
        }
    }
}

/// Unchanged, as far as anyone could tell (the tools round to 1 %).
fn same(a: (f64, bool), b: (f64, bool)) -> bool {
    (a.0 - b.0).abs() < 0.005 && a.1 == b.1
}

async fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::from(linksy_host::command(program, &[])).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The default output's level (1 = 100 %, more when boosted) and whether it's muted.
async fn read(tool: Tool) -> Option<(f64, bool)> {
    match tool {
        Tool::Wpctl => parse_wpctl(&output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).await?),
        Tool::Pactl => {
            let level = parse_pactl_volume(&output("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]).await?)?;
            let muted = output("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]).await?.contains("yes");
            Some((level, muted))
        }
    }
}

async fn set(tool: Tool, level: Option<f64>, muted: Option<bool>) {
    let mut runs: Vec<(&str, Vec<String>)> = Vec::new();
    match tool {
        Tool::Wpctl => {
            if let Some(l) = level {
                runs.push(("wpctl", vec!["set-volume".into(), "@DEFAULT_AUDIO_SINK@".into(), format!("{l:.2}")]));
            }
            if let Some(m) = muted {
                runs.push(("wpctl", vec!["set-mute".into(), "@DEFAULT_AUDIO_SINK@".into(), if m { "1" } else { "0" }.into()]));
            }
        }
        Tool::Pactl => {
            if let Some(l) = level {
                runs.push(("pactl", vec!["set-sink-volume".into(), "@DEFAULT_SINK@".into(), format!("{}%", (l * 100.0).round() as u32)]));
            }
            if let Some(m) = muted {
                runs.push(("pactl", vec!["set-sink-mute".into(), "@DEFAULT_SINK@".into(), if m { "1" } else { "0" }.into()]));
            }
        }
    }
    for (program, args) in runs {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        if output(program, &args).await.is_none() {
            debug!("{program} {} failed", args.join(" "));
        }
    }
}

/// `Volume: 0.70`, or `Volume: 0.70 [MUTED]`.
fn parse_wpctl(text: &str) -> Option<(f64, bool)> {
    let level = text.trim().strip_prefix("Volume:")?.split_whitespace().next()?.parse().ok()?;
    Some((level, text.contains("[MUTED]")))
}

/// `Volume: front-left: 45876 /  70% / -9.29 dB,   front-right: …`: the first channel's.
fn parse_pactl_volume(text: &str) -> Option<f64> {
    let percent = text.split_whitespace().find_map(|w| w.strip_suffix('%'))?;
    Some(percent.parse::<f64>().ok()? / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_the_tools_print() {
        assert_eq!(parse_wpctl("Volume: 0.70\n"), Some((0.70, false)));
        assert_eq!(parse_wpctl("Volume: 1.20 [MUTED]\n"), Some((1.20, true)));
        assert_eq!(parse_wpctl("nonsense"), None);
        let pactl = "Volume: front-left: 45876 /  70% / -9.29 dB,   front-right: 45876 /  70% / -9.29 dB\n        balance 0.00\n";
        assert_eq!(parse_pactl_volume(pactl), Some(0.70));
        assert!(same((0.70, false), (0.703, false)));
        assert!(!same((0.70, false), (0.70, true)));
    }

    /// Reads this computer's volume and sets it to what it already is. Needs a session
    /// with sound.
    #[tokio::test]
    #[ignore = "needs a session with sound"]
    async fn sets_this_computers_volume_to_itself() {
        let tool = tool().expect("wpctl or pactl");
        let (level, muted) = read(tool).await.unwrap();
        set(tool, Some(level), Some(muted)).await;
        assert!(same(read(tool).await.unwrap(), (level, muted)));
    }
}
