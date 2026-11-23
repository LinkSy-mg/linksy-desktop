//! This computer's gauges for a phone's deck (docs/protocol.md §4.28): CPU, memory, disk,
//! battery, temperature, network speed and uptime, read from /proc and /sys. Each is sampled
//! only while a deck that shows it is open.

use std::collections::HashSet;
use std::time::Instant;

use linksy_core::DeckState;

/// What the last sample read, to say how much changed since: CPU time and network bytes
/// are counters, and a gauge is the difference over the time between two samples.
#[derive(Default)]
pub struct Sampler {
    cpu: Option<(u64, u64)>,
    net: Option<(Instant, u64, u64)>,
}

impl Sampler {
    /// The gauges in `wanted` (metric names) that this computer has, as `stat.<metric>`.
    /// A metric it can't read (no battery on a desktop, no sensor) is left out.
    pub fn sample(&mut self, wanted: &HashSet<String>) -> Vec<DeckState> {
        let mut out = Vec::new();
        let mut put = |metric: &str, value: Option<f64>, text: String| {
            out.push(DeckState { id: format!("stat.{metric}"), on: None, value: value.map(|v| v.clamp(0.0, 1.0)), text: Some(text) });
        };
        let has = |m: &str| wanted.contains(m);
        if has("cpu") {
            let now = std::fs::read_to_string("/proc/stat").ok().and_then(|s| cpu_times(&s));
            if let (Some((busy, total)), Some((busy0, total0))) = (now, self.cpu)
                && total > total0
            {
                let used = (busy - busy0) as f64 / (total - total0) as f64;
                put("cpu", Some(used), percent(used));
            }
            self.cpu = now.or(self.cpu);
        }
        if has("ram") || has("swap") {
            if let Some(mem) = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| Memory::parse(&s)) {
                if has("ram") {
                    put("ram", Some(mem.ram_used()), format!("{} / {}", gigabytes(mem.ram_used_kb()), gigabytes(mem.total_kb)));
                }
                if has("swap") && mem.swap_total_kb > 0 {
                    put("swap", Some(mem.swap_used()), format!("{} / {}", gigabytes(mem.swap_used_kb()), gigabytes(mem.swap_total_kb)));
                }
            }
        }
        if has("disk")
            && let Some((used, total)) = disk("/")
            && total > 0
        {
            put("disk", Some(used as f64 / total as f64), format!("{} / {}", gigabytes(used / 1024), gigabytes(total / 1024)));
        }
        if has("battery")
            && let Some((level, charging)) = battery()
        {
            put("battery", Some(level), format!("{}%{}", (level * 100.0).round(), if charging { " ⚡" } else { "" }));
        }
        if has("temp")
            && let Some(c) = temperature()
        {
            // 0 to 100 °C on the gauge
            put("temp", Some(c / 100.0), format!("{} °C", c.round()));
        }
        if has("net_down") || has("net_up") {
            let now = std::fs::read_to_string("/proc/net/dev").ok().map(|s| network_bytes(&s));
            if let (Some((rx, tx)), Some((at, rx0, tx0))) = (now, self.net) {
                let secs = at.elapsed().as_secs_f64();
                if secs > 0.0 {
                    if has("net_down") {
                        put("net_down", None, speed(rx.saturating_sub(rx0) as f64 / secs));
                    }
                    if has("net_up") {
                        put("net_up", None, speed(tx.saturating_sub(tx0) as f64 / secs));
                    }
                }
            }
            if let Some((rx, tx)) = now {
                self.net = Some((Instant::now(), rx, tx));
            }
        }
        if has("uptime")
            && let Some(secs) = std::fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        {
            put("uptime", None, duration(secs as u64));
        }
        out
    }
}

/// `/proc/stat`'s first line: the time the CPUs spent working, and the time in all.
pub fn cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let n: Vec<u64> = line.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
    // user nice system idle iowait irq softirq steal
    if n.len() < 4 {
        return None;
    }
    let idle = n[3] + n.get(4).copied().unwrap_or(0);
    let total: u64 = n.iter().take(8).sum();
    Some((total - idle, total))
}

/// `/proc/meminfo`, in kB.
#[derive(Debug, PartialEq)]
pub struct Memory {
    pub total_kb: u64,
    available_kb: u64,
    pub swap_total_kb: u64,
    swap_free_kb: u64,
}

impl Memory {
    pub fn parse(meminfo: &str) -> Option<Self> {
        let get = |key: &str| meminfo.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':')?.split_whitespace().next()?.parse::<u64>().ok());
        Some(Self { total_kb: get("MemTotal")?, available_kb: get("MemAvailable")?, swap_total_kb: get("SwapTotal").unwrap_or(0), swap_free_kb: get("SwapFree").unwrap_or(0) })
    }

    pub fn ram_used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.available_kb)
    }

    pub fn ram_used(&self) -> f64 {
        self.ram_used_kb() as f64 / self.total_kb.max(1) as f64
    }

    pub fn swap_used_kb(&self) -> u64 {
        self.swap_total_kb.saturating_sub(self.swap_free_kb)
    }

    pub fn swap_used(&self) -> f64 {
        self.swap_used_kb() as f64 / self.swap_total_kb.max(1) as f64
    }
}

/// `/proc/net/dev`: bytes received and sent, over the real interfaces (not loopback, not
/// the virtual ones containers and VMs add, which would count the same traffic again).
pub fn network_bytes(dev: &str) -> (u64, u64) {
    dev.lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(name, _)| {
            let name = name.trim();
            name != "lo" && !["veth", "docker", "br-", "virbr", "vnet", "tun", "tap"].iter().any(|p| name.starts_with(p))
        })
        .fold((0, 0), |(rx, tx), (_, v)| {
            let f: Vec<u64> = v.split_whitespace().filter_map(|n| n.parse().ok()).collect();
            (rx + f.first().copied().unwrap_or(0), tx + f.get(8).copied().unwrap_or(0))
        })
}

fn percent(v: f64) -> String {
    format!("{}%", (v * 100.0).round())
}

fn gigabytes(kb: u64) -> String {
    format!("{:.1} GB", kb as f64 / 1024.0 / 1024.0)
}

pub fn speed(bytes_per_second: f64) -> String {
    const KB: f64 = 1000.0;
    if bytes_per_second >= KB * KB {
        format!("{:.1} MB/s", bytes_per_second / KB / KB)
    } else {
        format!("{:.0} KB/s", bytes_per_second / KB)
    }
}

pub fn duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

/// Used and total bytes of the filesystem at `path`.
fn disk(path: &str) -> Option<(u64, u64)> {
    let path = std::ffi::CString::new(path).ok()?;
    let mut s = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is a valid C string and `s` is written by statvfs before it is read.
    if unsafe { libc::statvfs(path.as_ptr(), s.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs returned 0, so it filled `s`.
    let s = unsafe { s.assume_init() };
    let size = s.f_frsize as u64;
    let total = s.f_blocks as u64 * size;
    Some((total.saturating_sub(s.f_bavail as u64 * size), total))
}

/// The first battery's charge (0 to 1) and whether it is charging.
fn battery() -> Option<(f64, bool)> {
    let mut batteries: Vec<_> = std::fs::read_dir("/sys/class/power_supply").ok()?.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("BAT")).collect();
    batteries.sort_by_key(|e| e.file_name());
    let dir = batteries.first()?.path();
    let level: f64 = std::fs::read_to_string(dir.join("capacity")).ok()?.trim().parse().ok()?;
    let charging = std::fs::read_to_string(dir.join("status")).is_ok_and(|s| s.trim() == "Charging");
    Some((level / 100.0, charging))
}

/// The processor's temperature in °C: a CPU sensor of hwmon (coretemp, k10temp, zenpower),
/// else the first thermal zone.
fn temperature() -> Option<f64> {
    let milli = |p: std::path::PathBuf| std::fs::read_to_string(p).ok()?.trim().parse::<f64>().ok().map(|m| m / 1000.0);
    for hwmon in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let name = std::fs::read_to_string(hwmon.path().join("name")).unwrap_or_default();
        if ["coretemp", "k10temp", "zenpower"].contains(&name.trim())
            && let Some(c) = milli(hwmon.path().join("temp1_input"))
        {
            return Some(c);
        }
    }
    milli("/sys/class/thermal/thermal_zone0/temp".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_use_is_the_work_over_all_time() {
        let before = cpu_times("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 1 1 1\n").unwrap();
        let after = cpu_times("cpu  200 0 200 1000 0 0 0 0 0 0\n").unwrap();
        assert_eq!(before, (200, 1000));
        // 200 more working, 200 more idle
        assert_eq!((after.0 - before.0) as f64 / (after.1 - before.1) as f64, 0.5);
        assert_eq!(cpu_times("nothing here"), None);
    }

    #[test]
    fn memory_in_use_is_what_is_not_available() {
        let m = Memory::parse("MemTotal: 16000000 kB\nMemFree: 1 kB\nMemAvailable: 12000000 kB\nSwapTotal: 4000000 kB\nSwapFree: 3000000 kB\n").unwrap();
        assert_eq!(m.ram_used_kb(), 4_000_000);
        assert_eq!(m.ram_used(), 0.25);
        assert_eq!(m.swap_used(), 0.25);
        assert_eq!(Memory::parse("MemTotal: 1 kB\n"), None, "no MemAvailable");
    }

    #[test]
    fn network_counts_real_interfaces_only() {
        let dev = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0\n wlan0: 1000 1 0 0 0 0 0 0 500 1 0 0 0 0 0 0\n docker0: 7000 1 0 0 0 0 0 0 7000 1 0 0 0 0 0 0\n enp3s0: 2000 1 0 0 0 0 0 0 100 1 0 0 0 0 0 0\n";
        assert_eq!(network_bytes(dev), (3000, 600));
    }

    #[test]
    fn numbers_read_the_way_a_person_says_them() {
        assert_eq!(speed(1_500_000.0), "1.5 MB/s");
        assert_eq!(speed(42_000.0), "42 KB/s");
        assert_eq!(duration(59 * 60), "59m");
        assert_eq!(duration(3 * 3600 + 5 * 60), "3h 5m");
        assert_eq!(duration(2 * 86_400 + 4 * 3600), "2d 4h");
        assert_eq!(gigabytes(8 * 1024 * 1024), "8.0 GB");
    }

    #[test]
    fn this_computers_own_gauges_read_and_stay_in_range() {
        let wanted: HashSet<String> = ["cpu", "ram", "disk", "uptime", "net_down"].map(String::from).into();
        let mut s = Sampler::default();
        let first = s.sample(&wanted);
        // The gauges that are a difference need a second sample
        assert!(first.iter().all(|g| g.id != "stat.cpu" && g.id != "stat.net_down"));
        std::thread::sleep(std::time::Duration::from_millis(300));
        let second = s.sample(&wanted);
        for metric in ["cpu", "ram", "disk", "uptime", "net_down"] {
            assert!(second.iter().any(|g| g.id == format!("stat.{metric}")), "no {metric}: {second:?}");
        }
        for g in &second {
            assert!(g.value.is_none_or(|v| (0.0..=1.0).contains(&v)), "{g:?}");
            assert!(g.text.as_deref().is_some_and(|t| !t.is_empty()), "{g:?}");
        }
    }

    #[test]
    fn only_what_is_asked_for_is_sampled() {
        let mut s = Sampler::default();
        let wanted: HashSet<String> = ["uptime".to_string()].into();
        let got = s.sample(&wanted);
        assert!(got.iter().all(|g| g.id == "stat.uptime"));
        assert!(s.sample(&HashSet::new()).is_empty());
    }
}
