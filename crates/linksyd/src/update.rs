//! "Linksy X is available": once at start-up and once a day, read the service information
//! (docs/protocol.md §4.12) from the website and compare its latest Linux version with this one.
//! A newer one is announced once (a notification, and the app's sidebar), and the site's
//! notices for Linux are shown once each. Nothing about the user is sent: a plain GET with
//! the version in the User-Agent. Off in Preferences ("Check for Updates").

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use linksy_i18n::t;
use linksy_ipc::{UpdateInfo, UpdatePrefs};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

pub static UPDATES: OnceLock<Updates> = OnceLock::new();

/// Where the service information lives; LINKSY_SERVICE_URL overrides it (tests).
const SERVICE_URL: &str = "https://linksy.diary.mg/linksy-relay";
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// Larger than this isn't the service information.
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Clone)]
pub struct Updates {
    path: PathBuf,
    state: Arc<Mutex<State>>,
    notifications: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    prefs: UpdatePrefs,
    /// The newest version already announced, and notices already shown.
    announced: Option<String>,
    seen_notices: HashSet<String>,
    /// The relays the website lists, preferred first: used from the next start.
    relays: Vec<String>,
    #[serde(skip)]
    latest: Option<Latest>,
}

#[derive(Clone)]
struct Latest {
    version: String,
    notes: Option<String>,
}

/// The parts of the service information this reads; everything else is ignored.
#[derive(Deserialize)]
struct Service {
    schema: u32,
    #[serde(default)]
    relays: Vec<RelayInfo>,
    #[serde(default)]
    latest: Option<Releases>,
    #[serde(default)]
    notices: Vec<Notice>,
}

#[derive(Deserialize)]
struct RelayInfo {
    url: String,
}

#[derive(Deserialize)]
struct Releases {
    linux: Option<Release>,
}

#[derive(Deserialize)]
struct Release {
    version: String,
    notes: Option<String>,
}

#[derive(Deserialize)]
struct Notice {
    id: String,
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    platforms: Option<Vec<String>>,
}

impl Updates {
    pub fn load(path: PathBuf, notifications: bool) -> Self {
        let state = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Updates { path, state: Arc::new(Mutex::new(state)), notifications }
    }

    pub fn info(&self) -> UpdateInfo {
        let s = self.state.lock().unwrap();
        let latest = s.latest.clone().filter(|l| newer(&l.version, env!("CARGO_PKG_VERSION")));
        UpdateInfo {
            current: env!("CARGO_PKG_VERSION").into(),
            latest: latest.as_ref().map(|l| l.version.clone()),
            notes: latest.and_then(|l| l.notes),
            prefs: s.prefs.clone(),
        }
    }

    pub fn set_prefs(&self, prefs: UpdatePrefs) {
        let check = {
            let mut s = self.state.lock().unwrap();
            let turned_on = prefs.check && !s.prefs.check;
            s.prefs = prefs;
            self.save(&s);
            turned_on
        };
        if check {
            let this = self.clone();
            tokio::spawn(async move { this.check().await });
        }
    }

    pub async fn run(self) {
        // Not in the way of start-up.
        tokio::time::sleep(Duration::from_secs(if std::env::var_os("LINKSY_SERVICE_URL").is_some() { 1 } else { 20 })).await;
        loop {
            self.check().await;
            tokio::time::sleep(EVERY).await;
        }
    }

    async fn check(&self) {
        if !self.state.lock().unwrap().prefs.check {
            return;
        }
        let url = std::env::var("LINKSY_SERVICE_URL").unwrap_or_else(|_| SERVICE_URL.into());
        let fetched = tokio::task::spawn_blocking(move || fetch(&url)).await;
        let service = match fetched {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return debug!("update check: {e}"),
            Err(_) => return,
        };
        let mut announce = None;
        let mut notices = Vec::new();
        {
            let mut s = self.state.lock().unwrap();
            let relays: Vec<String> = service.relays.into_iter().map(|r| r.url).filter(|u| u.starts_with("https://")).collect();
            if !relays.is_empty() {
                s.relays = relays;
            }
            if let Some(r) = service.latest.and_then(|l| l.linux) {
                if newer(&r.version, env!("CARGO_PKG_VERSION")) && s.announced.as_deref() != Some(r.version.as_str()) {
                    s.announced = Some(r.version.clone());
                    announce = Some(r.version.clone());
                }
                s.latest = Some(Latest { version: r.version, notes: r.notes.filter(|n| n.starts_with("https://")) });
            }
            for n in service.notices {
                let for_linux = n.platforms.as_ref().is_none_or(|p| p.iter().any(|p| p == "linux"));
                if for_linux && s.seen_notices.insert(n.id.clone()) {
                    notices.push(n);
                }
            }
            self.save(&s);
        }
        if let Some(version) = announce {
            info!("Linksy {version} is available");
            self.announce(&version);
        }
        for n in notices {
            #[cfg(target_os = "linux")]
            if self.notifications {
                crate::notify::show(plain(&n.title, 120), plain(&n.body, 600));
            }
        }
    }

    fn announce(&self, version: &str) {
        #[cfg(target_os = "linux")]
        if self.notifications {
            let notes = self.state.lock().unwrap().latest.as_ref().and_then(|l| l.notes.clone());
            let summary = t!("Linksy {version} is available", version);
            tokio::task::spawn_blocking(move || {
                let mut n = crate::notify::base(&summary, t!("See what's new and how to update."));
                if notes.is_some() {
                    n.action("default", t!("What's New")).action("notes", t!("What's New"));
                }
                let Ok(handle) = n.show() else { return };
                if let Some(url) = notes {
                    handle.wait_for_action(|action| {
                        if matches!(action, "default" | "notes") {
                            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                        }
                    });
                }
            });
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (version, self.notifications);
    }

    fn save(&self, s: &State) {
        if let Err(e) = serde_json::to_vec_pretty(s).map_err(std::io::Error::other).and_then(|b| std::fs::write(&self.path, b)) {
            warn!("could not save {}: {e}", self.path.display());
        }
    }
}

/// The relay to use away from home: LINKSY_RELAY, else the first the website listed last
/// time (saved in `update.json`), else Linksy's own.
pub fn relay(path: &std::path::Path) -> String {
    if let Ok(r) = std::env::var("LINKSY_RELAY") {
        return r;
    }
    let saved: Option<State> = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    saved.and_then(|s| s.relays.into_iter().next()).unwrap_or_else(|| DEFAULT_RELAY.into())
}

const DEFAULT_RELAY: &str = "https://linksy-relay.diary.mg";

fn fetch(url: &str) -> Result<Service, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).user_agent(format!("Linksy/{} (Linux)", env!("CARGO_PKG_VERSION"))).build().into();
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    let body = response.body_mut().with_config().limit(MAX_BYTES).read_to_string().map_err(|e| e.to_string())?;
    let service: Service = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    if service.schema != 1 {
        return Err(format!("unknown schema {}", service.schema));
    }
    Ok(service)
}

/// Whether version `a` ("0.5.10") is newer than `b`, part by part.
// Semantic versioning, the home-made edition.
fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

/// Notice text as plain text, cut to `max` characters.
fn plain(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.5.10", "0.5.9"));
        assert!(newer("0.6.0", "0.5.7"));
        assert!(!newer("0.5.7", "0.5.7"));
        assert!(!newer("0.5.6", "0.5.7"));
    }
}
