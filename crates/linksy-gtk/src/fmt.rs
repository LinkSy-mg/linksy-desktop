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
