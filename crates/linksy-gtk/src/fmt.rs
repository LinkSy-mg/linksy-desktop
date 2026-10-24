//! Human-friendly formatting for sizes, counts and times.

use std::path::{Path, PathBuf};
use std::time::Duration;

use linksy_i18n::t;

pub fn size(bytes: u64) -> String {
    let units: [&str; 5] = [t!("bytes"), t!("KB"), t!("MB"), t!("GB"), t!("TB")];
    if bytes < 1000 {
        return if bytes == 1 { t!("1 byte").into() } else { t!("{bytes} bytes", bytes) };
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < units.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    if v < 10.0 { format!("{v:.1} {}", units[unit]) } else { format!("{v:.0} {}", units[unit]) }
}

/// "1 item", "3 items". The words in use are spelled out whole, so they translate.
pub fn plural(n: usize, word: &str) -> String {
    match (word, n == 1) {
        ("item", true) => t!("1 item").into(),
        ("item", false) => t!("{n} items", n),
        ("photo", true) => t!("1 photo").into(),
        ("photo", false) => t!("{n} photos", n),
        (_, true) => format!("1 {word}"),
        (_, false) => format!("{n} {word}s"),
    }
}

pub fn remaining(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=4 => t!("a few seconds left").into(),
        5..=59 => t!("{s} seconds left", s),
        60..=3599 => match (s + 30) / 60 {
            1 => t!("1 minute left").into(),
            n => t!("{n} minutes left", n),
        },
        _ => match (s + 1800) / 3600 {
            1 => t!("1 hour left").into(),
            n => t!("{n} hours left", n),
        },
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

/// A moment (seconds since the epoch) as a short local day: "Tue 28 Oct".
pub fn day(epoch_s: u64) -> String {
    gtk::glib::DateTime::from_unix_local(epoch_s as i64).and_then(|d| d.format("%a %e %b")).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Number of files and total bytes under `paths` (folders counted recursively), as the
/// core sends them: a link chosen itself is followed, links inside folders are skipped.
// TODO: give this function a better name. It knows what it did.
pub fn measure(paths: &[PathBuf]) -> (usize, u64) {
    fn walk(p: &Path, acc: &mut (usize, u64), named: bool) {
        let meta = if named { std::fs::metadata(p) } else { std::fs::symlink_metadata(p) };
        let Ok(meta) = meta else { return };
        if meta.is_file() {
            acc.0 += 1;
            acc.1 += meta.len();
        } else if meta.is_dir()
            && let Ok(entries) = std::fs::read_dir(p) {
                for e in entries.flatten() {
                    walk(&e.path(), acc, false);
                }
            }
    }
    let mut acc = (0, 0);
    for p in paths {
        walk(p, &mut acc, true);
    }
    acc
}


/// "45 s", "4 min", "1 h 5 min".
pub fn duration(seconds: u64) -> String {
    match seconds {
        0..60 => t!("{seconds} s", seconds),
        60..3600 => t!("{minutes} min", minutes = seconds / 60),
        _ => t!("{hours} h {minutes} min", hours = seconds / 3600, minutes = seconds % 3600 / 60),
    }
}

/// "82%", or "82%, charging", for a device that's around; nothing for one that's away,
/// whose last reading may be old.
pub fn battery(d: &linksy_ipc::linksy_core::DeviceView) -> Option<String> {
    let b = d.battery.filter(|_| d.connected || d.nearby)?;
    Some(if b.charging { t!("{level}%, charging", level = b.level) } else { format!("{}%", b.level) })
}
