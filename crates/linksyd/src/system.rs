//! Where things live and what this machine is called, following the XDG conventions.

use std::path::PathBuf;

use linksy_core::DeviceKind;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(fallback))
}

/// `$XDG_DATA_HOME/linksy`: identity and paired devices.
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("linksy")
}

/// The user's Downloads folder, as configured in `user-dirs.dirs`.
pub fn download_dir() -> PathBuf {
    user_dir("XDG_DOWNLOAD_DIR", "Downloads")
}

/// The user's Pictures folder, where phones' photos are imported.
pub fn pictures_dir() -> PathBuf {
    user_dir("XDG_PICTURES_DIR", "Pictures")
}

fn user_dir(key: &str, fallback: &str) -> PathBuf {
    let config = xdg("XDG_CONFIG_HOME", ".config").join("user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(config) {
        for line in text.lines() {
            if let Some(v) = line.strip_prefix(key).and_then(|v| v.strip_prefix('=')) {
                let v = v.trim().trim_matches('"').replace("$HOME", &home().to_string_lossy());
                return PathBuf::from(v);
            }
        }
    }
    home().join(fallback)
}

/// The pretty hostname ("Sam's Laptop") if set, else the plain hostname.
pub fn device_name() -> String {
    if let Ok(info) = std::fs::read_to_string("/etc/machine-info") {
        for line in info.lines() {
            if let Some(v) = line.strip_prefix("PRETTY_HOSTNAME=") {
                let v = v.trim().trim_matches('"');
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_string())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Linux computer".into())
}

/// A machine with a battery is a laptop.
pub fn device_kind() -> DeviceKind {
    let has_battery = std::fs::read_dir("/sys/class/power_supply")
        .map(|entries| entries.flatten().any(|e| e.file_name().to_string_lossy().starts_with("BAT")))
        .unwrap_or(false);
    if has_battery { DeviceKind::Laptop } else { DeviceKind::Desktop }
}
