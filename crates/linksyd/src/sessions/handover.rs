//! Handover: the keyboard and mouse of one computer used on another. The service only
//! carries the bytes between the paired link and `linksy-handover`, which touches the
//! devices, and tells the person when they change hands.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use linksy_core::{Node, Session};
use linksy_handover::event::Event;
use linksy_handover::link::marked;
use linksy_i18n::t;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use super::{Sessions, active, bind, helper_command, socket_path, stop_helper};

/// How long the person has to type their password for the sending program.
const SENDER_TIMEOUT: Duration = Duration::from_secs(150);
/// How long the receiving program has to start.
const RECEIVER_TIMEOUT: Duration = Duration::from_secs(10);
/// How often this computer checks whether it can take a keyboard and mouse.
const CHECK_EVERY: Duration = Duration::from_secs(30);
/// The program's "no permission" exit status.
const NO_PERMISSION: i32 = 77;

impl Sessions {
    /// Take the keyboard and mouse the other computer hands over, as a virtual keyboard and
    /// mouse here for as long as the session lasts.
    pub(super) fn handover_receive(&self, session: Session) {
        let id = session.id;
        let info = active(&session);
        let this = self.clone();
        let task = tokio::spawn(async move {
            if let Err(e) = this.receive(session).await {
                warn!("handover ended: {e}");
            }
            this.running.lock().unwrap().remove(&id);
        });
        self.running.lock().unwrap().insert(id, (task.abort_handle(), info));
    }

    /// Hand this computer's keyboard and mouse over: the socket `linksy-handover send`
    /// connects to, which the caller starts (it needs permission to read the devices).
    pub fn handover_send(&self, session: Session) -> io::Result<PathBuf> {
        let path = socket_path("handover", session.id);
        let listener = bind(&path)?;
        let id = session.id;
        let info = active(&session);
        let this = self.clone();
        let socket = path.clone();
        let task = tokio::spawn(async move {
            let mut session = session;
            let accepted = tokio::time::timeout(SENDER_TIMEOUT, listener.accept()).await;
            let _ = std::fs::remove_file(&socket);
            match accepted {
                Ok(Ok((stream, _))) if from_a_helper(&stream) => {
                    info!("handing the keyboard and mouse over to {}", session.peer.name);
                    carry(&mut session, stream, false).await;
                }
                Ok(Ok(_)) => warn!("handover: something else connected to the socket"),
                _ => warn!("handover: the program that reads the keyboard and mouse never started"),
            }
            this.running.lock().unwrap().remove(&id);
        });
        self.running.lock().unwrap().insert(id, (task.abort_handle(), info));
        Ok(path)
    }

    async fn receive(&self, mut session: Session) -> io::Result<()> {
        let path = socket_path("handover", session.id);
        let listener: UnixListener = bind(&path)?;
        let mut cmd = helper_command("linksy-handover");
        cmd.arg("receive").arg("--socket").arg(&path);
        if self.test_pattern {
            cmd.arg("--print");
        }
        let mut child = cmd.kill_on_drop(true).spawn()?;
        info!("taking {}'s keyboard and mouse", session.peer.name);
        let accepted = tokio::select! {
            a = tokio::time::timeout(RECEIVER_TIMEOUT, listener.accept()) => a,
            status = child.wait() => {
                let _ = std::fs::remove_file(&path);
                let status = status?;
                if status.code() == Some(NO_PERMISSION) {
                    crate::notify::show(t!("Handover Needs Setup").into(), t!("This computer can't take another's keyboard and mouse yet. Open Finish Setup in Linksy and set up Handover.").into());
                }
                return Err(io::Error::other(format!("the receiving program stopped ({status})")));
            }
        };
        let _ = std::fs::remove_file(&path);
        let (stream, _) = accepted.map_err(|_| io::Error::other("the receiving program didn't start in time"))??;
        if !from_a_helper(&stream) {
            return Err(io::Error::other("something else connected to the socket"));
        }
        carry(&mut session, stream, true).await;
        stop_helper(&mut child).await;
        Ok(())
    }
}

/// Only the program this service started, or one of the person's own (root, or the same
/// user), may stand in for the keyboard and mouse.
fn from_a_helper(stream: &UnixStream) -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail
    let me = unsafe { libc::geteuid() };
    stream.peer_cred().is_ok_and(|c| c.uid() == 0 || c.uid() == me)
}

/// Copy the events between the helper and the paired link until either side ends, telling
/// the person when the devices change hands.
async fn carry(session: &mut Session, local: UnixStream, receiving: bool) {
    let (mut local_rx, mut local_tx) = local.into_split();
    let peer = session.peer.name.clone();
    let (send, recv) = (&mut session.send, &mut session.recv);
    let mut buf = vec![0u8; 8192];
    let mut tail = Vec::new();
    if receiving {
        loop {
            let Ok(Some(n)) = recv.read(&mut buf).await else { break };
            say_handed(&mut tail, &buf[..n], &peer, true);
            if local_tx.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
    } else {
        let mut ignored = [0u8; 64];
        loop {
            tokio::select! {
                got = local_rx.read(&mut buf) => {
                    let n = match got {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    say_handed(&mut tail, &buf[..n], &peer, false);
                    if send.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                // The other computer ended it
                got = recv.read(&mut ignored) => {
                    if !matches!(got, Ok(Some(_))) {
                        break;
                    }
                }
            }
        }
    }
    let _ = send.shutdown().await;
}

/// Tell the person each time the devices went over or came back, from the messages the
/// program puts among the events. `tail` keeps a frame that arrived in two parts.
fn say_handed(tail: &mut Vec<u8>, bytes: &[u8], peer: &str, receiving: bool) {
    for on in handed(tail, bytes) {
        let (summary, body) = match (receiving, on) {
            (true, true) => (t!("{device}'s Keyboard and Mouse Are Here", device = peer), t!("Use them on this computer until {device} takes them back.", device = peer)),
            (true, false) => (t!("{device} Took Its Keyboard and Mouse Back", device = peer), String::new()),
            (false, true) => (t!("Keyboard and Mouse Handed Over").to_string(), t!("They are on {device} now. Press the hotkey again to take them back.", device = peer)),
            (false, false) => (t!("Keyboard and Mouse Are Back").to_string(), t!("They are on this computer again.").to_string()),
        };
        crate::notify::show(summary, body);
    }
}

/// The hand-overs (true) and give-backs (false) in `bytes`, after what `tail` held.
fn handed(tail: &mut Vec<u8>, bytes: &[u8]) -> Vec<bool> {
    tail.extend_from_slice(bytes);
    let whole = tail.len() - tail.len() % Event::SIZE;
    let found = tail[..whole].chunks(Event::SIZE).filter_map(|frame| marked(Event::from_bytes(frame.try_into().unwrap_or_default()))).collect();
    tail.drain(..whole);
    found
}

/// Announce `handover` to the paired computers while this one can take a keyboard and
/// mouse (the program is installed and `/dev/uinput` can be written), and stop when it
/// can't. Checked now and then: Finish Setup may have just made it possible.
pub async fn announce(node: Node) {
    let mut every = tokio::time::interval(CHECK_EVERY);
    loop {
        every.tick().await;
        let can = tokio::task::spawn_blocking(can_receive).await.unwrap_or(false);
        let mut features = node.info().features;
        let has = features.iter().any(|f| f == "handover");
        if can != has {
            if can {
                features.push("handover".into());
            } else {
                features.retain(|f| f != "handover");
            }
            node.set_features(features).await;
        }
    }
}

fn can_receive() -> bool {
    // Tests: no device is made, the program prints what it gets
    std::env::var_os("LINKSY_SCREEN_TEST").is_some() || (super::helper("linksy-handover").is_file() && std::fs::OpenOptions::new().write(true).open("/dev/uinput").is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use linksy_handover::link::mark;

    #[test]
    fn the_messages_among_the_events_are_found_even_when_a_frame_comes_in_two_parts() {
        let mut bytes = Vec::new();
        bytes.extend(Event::key(30, true).to_bytes());
        bytes.extend(mark(true).to_bytes());
        bytes.extend(Event::sync().to_bytes());
        bytes.extend(mark(false).to_bytes());
        let mut tail = Vec::new();
        let (first, second) = bytes.split_at(20);
        let mut found = handed(&mut tail, first);
        found.extend(handed(&mut tail, second));
        assert_eq!(found, vec![true, false]);
        assert!(tail.is_empty());
    }

    #[test]
    fn plain_events_say_nothing() {
        let mut tail = Vec::new();
        let bytes: Vec<u8> = [Event::key(30, true), Event::key(30, false), Event::sync()].iter().flat_map(|e| e.to_bytes()).collect();
        assert!(handed(&mut tail, &bytes).is_empty());
    }
}
