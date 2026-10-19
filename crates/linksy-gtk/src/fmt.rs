//! Human-friendly formatting for sizes, counts and times.

use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return if bytes == 1 { "1 byte".into() } else { format!("{bytes} bytes") };
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    if v < 10.0 { format!("{v:.1} {}", UNITS[unit]) } else { format!("{v:.0} {}", UNITS[unit]) }
}

pub fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

pub fn remaining(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=4 => "a few seconds left".into(),
        5..=59 => format!("{s} seconds left"),
        60..=3599 => plural(((s + 30) / 60) as usize, "minute") + " left",
        _ => plural(((s + 1800) / 3600) as usize, "hour") + " left",
    }
}

pub fn spaced_code(code: &str) -> String {
    if code.len() == 6 { format!("{} {}", &code[..3], &code[3..]) } else { code.to_string() }
}

/// "~/Downloads" rather than "/home/me/Downloads".
pub fn home_relative(p: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if p == home => "~".into(),
        Some(home) => match p.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => p.display().to_string(),
        },
        None => p.display().to_string(),
    }
}

/// "192.168.1.20", "192.168.1.20:47200" or "[fe80::1]:47200" → a socket address, using
/// Linksy's default port when none is given.
pub fn peer_address(text: &str) -> Option<std::net::SocketAddr> {
    const DEFAULT_PORT: u16 = 47_200;
    let t = text.trim();
    t.parse().ok().or_else(|| t.parse::<std::net::IpAddr>().ok().map(|ip| std::net::SocketAddr::new(ip, DEFAULT_PORT)))
}

/// This machine's address on its main network. Connecting a UDP socket only picks a
/// route; nothing is sent.
pub fn local_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    Some(s.local_addr().ok()?.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

pub fn file_name(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Number of files and total bytes under `paths` (folders counted recursively).
// TODO: give this function a better name. It knows what it did.
pub fn measure(paths: &[PathBuf]) -> (usize, u64) {
    fn walk(p: &Path, acc: &mut (usize, u64)) {
        let Ok(meta) = std::fs::symlink_metadata(p) else { return };
        if meta.is_file() {
            acc.0 += 1;
            acc.1 += meta.len();
        } else if meta.is_dir()
            && let Ok(entries) = std::fs::read_dir(p) {
                for e in entries.flatten() {
                    walk(&e.path(), acc);
                }
            }
    }
    let mut acc = (0, 0);
    for p in paths {
        walk(p, &mut acc);
    }
    acc
}


/// "45 s", "4 min", "1 h 5 min".
pub fn duration(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min", seconds / 60),
        _ => format!("{} h {} min", seconds / 3600, seconds % 3600 / 60),
    }
}
