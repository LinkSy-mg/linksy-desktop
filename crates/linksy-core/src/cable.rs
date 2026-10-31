//! The USB cable's framing (docs/protocol.md §4.11), shared by the computer and the phone:
//! Linksy's own QUIC packets cross the accessory's byte pipe, each as a 2-byte big-endian
//! length and the packet. Version 2 adds control frames, a zero length then a kind and a
//! 2-byte length: a hello, keepalives, padding and a clean close. Each side sends them only
//! once it knows the other reads them: the computer says so in the accessory's version
//! string, the phone in its hello.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One read on the phone's accessory: Android gives each read a single USB request of at
/// most this much, and drops what doesn't fit in a smaller one.
pub const READ: usize = 16_384;
/// The framing this side speaks.
pub const VERSION: u8 = 2;
/// The accessory's version string a computer announces, for [`VERSION`].
pub const ACCESSORY_VERSION: &str = "2";
/// How often a side that hears version 2 checks the cable, and how long it waits.
pub const PING_EVERY: Duration = Duration::from_secs(1);
pub const SILENT_FOR: Duration = Duration::from_secs(5);

const HELLO: u8 = 1;
const PING: u8 = 2;
const PONG: u8 = 3;
const CLOSE: u8 = 4;
const PAD: u8 = 5;

#[derive(Debug, PartialEq)]
pub enum Frame {
    /// A QUIC packet.
    Data(Vec<u8>),
    Hello { version: u8 },
    Ping(u64),
    Pong(u64),
    /// The other side is letting go of the cable.
    Close,
    /// Nothing: keeps a write from ending on a full USB packet, which the other side would
    /// read as "more to come".
    Pad,
}

pub fn encode(frame: &Frame, out: &mut Vec<u8>) {
    let (kind, payload): (u8, &[u8]) = match frame {
        Frame::Data(d) => {
            out.extend_from_slice(&(d.len() as u16).to_be_bytes());
            out.extend_from_slice(d);
            return;
        }
        Frame::Hello { version } => (HELLO, std::slice::from_ref(version)),
        Frame::Ping(t) => return control(out, PING, &t.to_be_bytes()),
        Frame::Pong(t) => return control(out, PONG, &t.to_be_bytes()),
        Frame::Close => (CLOSE, &[]),
        Frame::Pad => (PAD, &[]),
    };
    control(out, kind, payload);
}

fn control(out: &mut Vec<u8>, kind: u8, payload: &[u8]) {
    out.extend_from_slice(&[0, 0, kind]);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
}

/// Frames out of the bytes as they come, whatever the reads cut them into.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    at: usize,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        if self.at > 0 && self.at == self.buf.len() {
            self.buf.clear();
            self.at = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Option<Frame> {
        loop {
            let rest = &self.buf[self.at..];
            if rest.len() < 2 {
                return None;
            }
            let len = u16::from_be_bytes([rest[0], rest[1]]) as usize;
            if len > 0 {
                let data = rest.get(2..2 + len)?.to_vec();
                self.at += 2 + len;
                return Some(Frame::Data(data));
            }
            if rest.len() < 5 {
                return None;
            }
            let size = u16::from_be_bytes([rest[3], rest[4]]) as usize;
            let payload = rest.get(5..5 + size)?;
            let stamp = || payload.get(..8).map(|b| u64::from_be_bytes(b.try_into().unwrap()));
            let frame = match rest[2] {
                HELLO => payload.first().map(|&version| Frame::Hello { version }),
                PING => stamp().map(Frame::Ping),
                PONG => stamp().map(Frame::Pong),
                CLOSE => Some(Frame::Close),
                PAD => Some(Frame::Pad),
                // A kind from a newer version: skipped
                _ => None,
            };
            self.at += 5 + size;
            if let Some(f) = frame {
                return Some(f);
            }
        }
    }
}

/// Whether a write of `len` bytes ends on a full USB packet (512 bytes at high speed, 1024
/// at super speed), so the other side's read would wait for more.
pub fn ends_full(len: usize) -> bool {
    len > 0 && len.is_multiple_of(512)
}

/// The phone's end of the cable (Android, docs/protocol.md §4.11): `accessory` is the
/// accessory's file (`UsbManager.openAccessory`, then `detachFd()`), `computer_version` its
/// version string. A local UDP port stands in for the computer: the node dials it
/// (`node.connect(127.0.0.1:port)`) and every datagram crosses the cable. Runs on threads of
/// its own until the cable goes; returns the port.
pub fn phone_bridge(accessory: std::fs::File, computer_version: &str) -> std::io::Result<u16> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = socket.local_addr()?.port();
    let v2 = computer_version.trim().parse::<u8>().is_ok_and(|v| v >= 2);
    let bridge = Arc::new(Bridge { socket, out: Mutex::new(accessory.try_clone()?), node: Mutex::new(None), heard: Mutex::new(Instant::now()), v2, open: std::sync::atomic::AtomicBool::new(true) });
    if v2 {
        bridge.send(&[Frame::Hello { version: VERSION }])?;
    }
    let b = bridge.clone();
    std::thread::Builder::new().name("linksy-cable-in".into()).spawn(move || b.read_cable(accessory))?;
    let b = bridge.clone();
    std::thread::Builder::new().name("linksy-cable-out".into()).spawn(move || b.write_cable())?;
    Ok(port)
}

struct Bridge {
    socket: UdpSocket,
    out: Mutex<std::fs::File>,
    /// The node, learned from its first datagram.
    node: Mutex<Option<SocketAddr>>,
    heard: Mutex<Instant>,
    v2: bool,
    open: std::sync::atomic::AtomicBool,
}

impl Bridge {
    fn is_open(&self) -> bool {
        self.open.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn close(&self) {
        self.open.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// One write for all of `frames`.
    fn send(&self, frames: &[Frame]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(READ);
        for f in frames {
            encode(f, &mut bytes);
        }
        if self.v2 && ends_full(bytes.len()) {
            encode(&Frame::Pad, &mut bytes);
        }
        self.out.lock().unwrap().write_all(&bytes)
    }

    fn read_cable(&self, mut accessory: std::fs::File) {
        let mut buf = vec![0u8; READ];
        let mut frames = Decoder::default();
        while self.is_open() {
            let n = match accessory.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            *self.heard.lock().unwrap() = Instant::now();
            frames.push(&buf[..n]);
            while let Some(frame) = frames.next_frame() {
                match frame {
                    Frame::Data(packet) => {
                        if let Some(node) = *self.node.lock().unwrap() {
                            let _ = self.socket.send_to(&packet, node);
                        }
                    }
                    Frame::Ping(t) if self.v2 => {
                        let _ = self.send(&[Frame::Pong(t)]);
                    }
                    Frame::Close => self.close(),
                    _ => {}
                }
            }
        }
        self.close();
    }

    fn write_cable(&self) {
        let mut buf = vec![0u8; 65_535];
        let _ = self.socket.set_read_timeout(Some(PING_EVERY));
        while self.is_open() {
            if self.v2 && self.heard.lock().unwrap().elapsed() > SILENT_FOR {
                break;
            }
            let (len, from) = match self.socket.recv_from(&mut buf) {
                Ok(got) => got,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                Err(_) => break,
            };
            *self.node.lock().unwrap() = Some(from);
            let mut frames = vec![Frame::Data(buf[..len].to_vec())];
            // What's already waiting goes in the same write
            let _ = self.socket.set_nonblocking(true);
            let mut size = len;
            while size < READ {
                match self.socket.recv_from(&mut buf) {
                    Ok((len, _)) => {
                        size += len + 2;
                        frames.push(Frame::Data(buf[..len].to_vec()));
                    }
                    Err(_) => break,
                }
            }
            let _ = self.socket.set_nonblocking(false);
            if self.send(&frames).is_err() {
                break;
            }
        }
        if self.v2 {
            let _ = self.send(&[Frame::Close]);
        }
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames_of(bytes: &[u8], cut: usize) -> Vec<Frame> {
        let mut d = Decoder::default();
        let mut out = Vec::new();
        for chunk in bytes.chunks(cut) {
            d.push(chunk);
            while let Some(f) = d.next_frame() {
                out.push(f);
            }
        }
        out
    }

    #[test]
    fn frames_survive_any_cut() {
        let sent = vec![Frame::Hello { version: 2 }, Frame::Data(vec![7; 1200]), Frame::Ping(42), Frame::Data(vec![1]), Frame::Pad, Frame::Pong(42), Frame::Close];
        let mut bytes = Vec::new();
        for f in &sent {
            encode(f, &mut bytes);
        }
        for cut in [1, 2, 3, 5, 512, READ] {
            assert_eq!(frames_of(&bytes, cut), sent, "cut every {cut} bytes");
        }
    }

    #[test]
    fn version_one_is_data_only() {
        // What a version 1 phone sends: plain length and packet
        let bytes = [&[0u8, 3][..], b"abc", &[0, 1], b"z"].concat();
        assert_eq!(frames_of(&bytes, 4), vec![Frame::Data(b"abc".to_vec()), Frame::Data(b"z".to_vec())]);
    }

    #[test]
    fn unknown_kinds_are_skipped() {
        let mut bytes = vec![0, 0, 99, 0, 3, 1, 2, 3];
        encode(&Frame::Data(vec![5]), &mut bytes);
        assert_eq!(frames_of(&bytes, 7), vec![Frame::Data(vec![5])]);
    }
}
