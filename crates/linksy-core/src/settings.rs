//! User settings, changeable while the node runs. Only values the user changed are
//! written to `settings.json`; everything else follows the [`Config`](crate::Config)
//! defaults, so a new default (say, a renamed host) still applies.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, invalid};
use crate::identity::DeviceInfo;
use crate::node::Config;

/// Longest device name; it has to fit in an mDNS instance name with the short id.
pub const MAX_NAME: usize = 48;

/// A name a peer sent, made safe to show, store and put in a prompt: control, bidi and
/// invisible characters dropped, whitespace collapsed, capped at [`MAX_NAME`]. May come back empty.
pub(crate) fn clean_name(name: &str) -> String {
    let visible = name.chars().filter(|c| c.is_whitespace() || !(c.is_control() || matches!(c, '\u{061c}' | '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')));
    visible.collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_NAME).collect::<String>().trim_end().to_string()
}

/// A peer's handshake device, with its name cleaned (see [`clean_name`]) so a
/// hostile one can't crowd the pairing code out of a prompt. A name left empty becomes the short id.
pub(crate) fn tidy_peer(mut device: DeviceInfo) -> DeviceInfo {
    device.name = clean_name(&device.name);
    if device.name.is_empty() {
        device.name = device.id.short();
    }
    device
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Name other devices see.
    pub name: String,
    /// Where received files are saved.
    pub download_dir: PathBuf,
    /// Keep paired devices connected instead of connecting on demand.
    pub stay_connected: bool,
    /// Accept files from paired devices without asking.
    pub auto_accept: bool,
    /// Reach paired devices away from home, over the internet (docs/protocol.md §4.13).
    #[serde(default = "yes")]
    pub away: bool,
    /// A relay the user hosts (`https://relay.example.com`), instead of the one the
    /// service lists (docs/protocol.md §4.12). None: Linksy's own.
    #[serde(default)]
    pub relay: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    download_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stay_connected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_accept: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    away: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relay: Option<String>,
}

impl Settings {
    pub(crate) fn defaults(config: &Config) -> Self {
        Self {
            name: config.name.clone(),
            download_dir: config.download_dir.clone(),
            stay_connected: config.stay_connected,
            auto_accept: false,
            away: true,
            relay: None,
        }
    }

    pub(crate) fn load(path: &Path, config: &Config) -> Result<Self> {
        let d = Self::defaults(config);
        let stored: Stored = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Stored::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            name: stored.name.unwrap_or(d.name),
            download_dir: stored.download_dir.unwrap_or(d.download_dir),
            stay_connected: stored.stay_connected.unwrap_or(d.stay_connected),
            auto_accept: stored.auto_accept.unwrap_or(d.auto_accept),
            away: stored.away.unwrap_or(d.away),
            relay: stored.relay.or(d.relay),
        })
    }

    pub(crate) fn save(&self, path: &Path, config: &Config) -> Result<()> {
        let d = Self::defaults(config);
        let stored = Stored {
            name: (self.name != d.name).then(|| self.name.clone()),
            download_dir: (self.download_dir != d.download_dir).then(|| self.download_dir.clone()),
            stay_connected: (self.stay_connected != d.stay_connected).then_some(self.stay_connected),
            auto_accept: (self.auto_accept != d.auto_accept).then_some(self.auto_accept),
            away: (self.away != d.away).then_some(self.away),
            relay: (self.relay != d.relay).then(|| self.relay.clone()).flatten(),
        };
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&stored)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// Trim and check what the user typed.
    pub(crate) fn validated(mut self) -> Result<Self> {
        self.name = self.name.split_whitespace().collect::<Vec<_>>().join(" ");
        if self.name.is_empty() {
            return Err(invalid("the device name can't be empty"));
        }
        if self.name.chars().count() > MAX_NAME {
            return Err(invalid(format!("the device name can be at most {MAX_NAME} characters")));
        }
        if !self.download_dir.is_absolute() {
            return Err(invalid("the download folder must be an absolute path"));
        }
        std::fs::create_dir_all(&self.download_dir)?;
        // The relay: an https URL, as iroh reads it; plain http only for a relay on this
        // machine (localhost, 127.0.0.1, ::1), in development. Blank means Linksy's own.
        self.relay = match self.relay.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
            None => None,
            Some(r) => {
                let url: iroh::RelayUrl = r.parse().map_err(|e| invalid(format!("the relay address doesn't look right: {e}")))?;
                let on_this_machine = url.host_str().is_some_and(|h| h == "localhost" || h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()));
                match url.scheme() {
                    "https" => {}
                    "http" if on_this_machine => {}
                    _ => return Err(invalid("the relay address must start with https:// (http:// only for a relay on this machine)")),
                }
                Some(url.to_string())
            }
        };
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_only_changes() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().into(), dir.path().join("dl"), "Host");
        let path = dir.path().join("settings.json");
        let mut s = Settings::load(&path, &config).unwrap();
        assert_eq!(s, Settings::defaults(&config));
        s.auto_accept = true;
        s.save(&path, &config).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().replace(char::is_whitespace, ""), r#"{"auto_accept":true}"#);
        // A changed default still applies to fields the user never touched.
        let renamed = Config::new(dir.path().into(), dir.path().join("dl"), "New Host");
        let s = Settings::load(&path, &renamed).unwrap();
        assert_eq!((s.name.as_str(), s.auto_accept), ("New Host", true));
    }

    #[test]
    fn validation_tidies_names() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().into(), dir.path().join("dl"), "Host");
        let mut s = Settings::defaults(&config);
        s.name = "  Living   Room  ".into();
        assert_eq!(s.clone().validated().unwrap().name, "Living Room");
        s.name = "   ".into();
        assert!(s.clone().validated().is_err());
        s.name = "x".repeat(MAX_NAME + 1);
        assert!(s.validated().is_err());
    }

    #[test]
    fn peer_names_are_cleaned() {
        assert_eq!(clean_name("Pixel 9"), "Pixel 9");
        assert_eq!(clean_name("  Living \n\t Room  "), "Living Room");
        assert_eq!(clean_name("Evil\u{202e}\u{2066}\u{200f}\u{7}\u{0}Name"), "EvilName");
        assert_eq!(clean_name("Pixel\u{200b} 9\u{feff}\u{061c}"), "Pixel 9");
        assert_eq!(clean_name("👩\u{200d}💻 فرزانه\u{200c}ها"), "👩\u{200d}💻 فرزانه\u{200c}ها");
        assert_eq!(clean_name(&"x".repeat(MAX_NAME * 100)).chars().count(), MAX_NAME);
        assert_eq!(clean_name("\n\u{202e} "), "");
        // Cut exactly at a space: no trailing one is left.
        assert_eq!(clean_name(&format!("{} tail", "x".repeat(MAX_NAME - 1))), "x".repeat(MAX_NAME - 1));
    }

    #[test]
    fn relay_is_checked_and_tidied() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().into(), dir.path().join("dl"), "Host");
        let mut s = Settings::defaults(&config);
        s.relay = Some("  https://relay.example.com  ".into());
        assert_eq!(s.clone().validated().unwrap().relay.as_deref(), Some("https://relay.example.com/"));
        s.relay = Some("   ".into());
        assert_eq!(s.clone().validated().unwrap().relay, None);
        s.relay = Some("relay.example.com".into());
        assert!(s.clone().validated().is_err());
        s.relay = Some("https://".into());
        assert!(s.clone().validated().is_err());
        // Plain http only on this machine.
        for local in ["http://localhost:3340", "http://127.0.0.1:3340", "http://[::1]:3340"] {
            s.relay = Some(local.into());
            assert!(s.clone().validated().is_ok(), "{local}");
        }
        for remote in ["http://relay.example.com", "http://10.0.0.5:3340", "ftp://relay.example.com"] {
            s.relay = Some(remote.into());
            assert!(s.clone().validated().is_err(), "{remote}");
        }
        // Saved only when set: a cleared relay leaves no trace.
        s.relay = Some("https://relay.example.com".into());
        let path = dir.path().join("settings.json");
        s.clone().validated().unwrap().save(&path, &config).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("relay.example.com"));
        s.relay = None;
        s.save(&path, &config).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("relay"));
    }
}
