//! Apps between phones, and what to ask the other phone before sending anything
//! (docs/protocol.md §4.22).
//!
//! An app travels as the APK files it is made of (a base and, for most apps from Play,
//! splits), in a folder of its own inside an ordinary offer, so it gets the same resume,
//! hashing and progress as any file. [`AppInfo`] says which folder is which app, so the
//! receiving phone can show "Firefox 129, an update" before it asks and install the folder
//! afterwards. It is only a description: Android checks what it installs itself, and
//! asks the user.
//!
//! [`PreflightMsg`] is one round trip before sending: room for it on the other device,
//! whether it asks first, whether it can do a direct link now, and which of the apps about
//! to be sent it has already.

use serde::{Deserialize, Serialize};

use crate::protocol::FileEntry;

/// Most apps in one offer, and most to ask about at once.
pub const MAX_APPS: usize = 100;
pub const MAX_PROBES: usize = 256;
/// Most files one app may have: a base and its splits.
pub const MAX_APKS: usize = 64;
/// An app's icon (a PNG of 96 px is plenty), raw bytes before base64.
pub const MAX_APP_ICON: usize = 16 * 1024;
const MAX_LABEL: usize = 128;
const MAX_VERSION_NAME: usize = 64;

/// An app inside an offer: its APKs are the files in the folder `dir` of the offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    /// Android's application id, `org.mozilla.firefox`.
    pub package: String,
    /// What the user knows it as.
    pub label: String,
    /// Android's `longVersionCode`; a higher one is newer.
    pub version_code: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    /// SHA-256 of the certificate the app is signed with, 64 hex characters: what lets the
    /// other phone say "same developer" and tell an update from a clash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
    /// The folder of the offer holding its APKs: one path component, set by the sender
    /// ([`Node::send_bundle`](crate::Node::send_bundle) makes it `<package>-<version_code>`).
    pub dir: String,
    /// A PNG, base64, at most [`MAX_APP_ICON`] bytes before that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl AppInfo {
    /// Whether what it says looks right; anything else is dropped from an offer (the files
    /// still arrive, as files).
    pub fn is_valid(&self) -> bool {
        is_package(&self.package)
            && !self.label.trim().is_empty()
            && self.label.len() <= MAX_LABEL
            && !self.label.chars().any(char::is_control)
            && self.version_name.as_ref().is_none_or(|v| v.len() <= MAX_VERSION_NAME && !v.chars().any(char::is_control))
            && self.signer.as_deref().is_none_or(is_signer)
            && is_folder(&self.dir)
            && self.icon.as_deref().is_none_or(|i| i.len() <= MAX_APP_ICON * 4 / 3 + 4 && i.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=')))
    }
}

/// An app to send: what it says about itself and its APK files on this device (the base
/// and every split, each a file that exists), as Android keeps them.
#[derive(Clone, Debug)]
pub struct AppSend {
    pub info: AppInfo,
    pub apks: Vec<std::path::PathBuf>,
}

/// An app that arrived: its description and the APK files it came in, saved, to install
/// together in one session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceivedApp {
    pub info: AppInfo,
    pub apks: Vec<std::path::PathBuf>,
}

/// The apps of an offer that arrived, with where their files were saved (`saved` is in the
/// order of `files`, as the transfer returns it).
pub fn received_apps(apps: &[AppInfo], files: &[FileEntry], saved: &[std::path::PathBuf]) -> Vec<ReceivedApp> {
    apps.iter()
        .map(|info| {
            let prefix = format!("{}/", info.dir);
            let apks = files.iter().zip(saved).filter(|(f, _)| f.path.starts_with(&prefix)).map(|(_, path)| path.clone()).collect();
            ReceivedApp { info: info.clone(), apks }
        })
        .collect()
}

/// An app on the sender's phone, to ask the other about before sending it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppProbe {
    pub package: String,
    pub version_code: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
}

impl AppProbe {
    pub fn is_valid(&self) -> bool {
        is_package(&self.package) && self.signer.as_deref().is_none_or(is_signer)
    }
}

/// How the other phone's copy of an app compares with the one to be sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppHave {
    /// Not installed there.
    #[serde(rename = "none")]
    Missing,
    /// The same version, from the same developer.
    Same,
    /// Installed, older: sending it is an update.
    Older,
    /// Installed, newer than the one to be sent.
    Newer,
    /// Installed but signed by someone else: Android won't update it, only replace it after
    /// the other one is removed, and that loses its data. Don't offer it.
    OtherSigner,
    /// Something a newer Linksy says.
    #[serde(other)]
    Unknown,
}

/// What the other phone has of one probed app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppState {
    pub package: String,
    pub have: AppHave,
    /// The version it has, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<u64>,
}

/// Whether a device could set up a direct link (§4.17) right now, as only its app knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectReady {
    Ready,
    /// Wi-Fi is switched off.
    WifiOff,
    /// Linksy may not use Wi-Fi Direct (Nearby devices permission).
    NoPermission,
    /// This device has no Wi-Fi Direct.
    Unsupported,
    #[serde(other)]
    Unknown,
}

/// The messages of the preflight stream: the first message of a bidirectional stream, and
/// its answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PreflightMsg {
    /// About to send `bytes` in `files` files, and perhaps these apps.
    Preflight {
        bytes: u64,
        #[serde(default)]
        files: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        apps: Vec<AppProbe>,
    },
    PreflightReply(PreflightReply),
}

/// What a device says before it is sent something.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightReply {
    /// Bytes free where received files go, when known. A phone that copies what it receives
    /// to another place counts that too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free: Option<u64>,
    /// Files from the sender are accepted without asking.
    #[serde(default)]
    pub auto: bool,
    /// Could it set up a direct link now.
    pub direct: DirectReady,
    /// Whether it can install apps (its Linksy may), when it is a phone that does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_install: Option<bool>,
    /// What it has of the probed apps, one per probe it understood.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<AppState>,
}

impl PreflightReply {
    /// Whether `bytes` fit, as far as it knows.
    pub fn has_room_for(&self, bytes: u64) -> bool {
        self.free.is_none_or(|free| free >= bytes)
    }
}

/// The apps of an offer that describe themselves truthfully: valid, each with files in its
/// folder and nothing but APKs there, none claiming a folder another one has.
pub fn valid_apps(apps: Vec<AppInfo>, files: &[FileEntry]) -> Vec<AppInfo> {
    let mut seen = std::collections::HashSet::new();
    apps.into_iter()
        .take(MAX_APPS)
        .filter(|a| {
            if !a.is_valid() || !seen.insert(a.dir.clone()) {
                return false;
            }
            let prefix = format!("{}/", a.dir);
            let mine: Vec<&FileEntry> = files.iter().filter(|f| f.path.starts_with(&prefix)).collect();
            !mine.is_empty() && mine.len() <= MAX_APKS && mine.iter().all(|f| is_apk(&f.path[prefix.len()..]))
        })
        .collect()
}

/// `base.apk`, `split_config.arm64_v8a.apk`: a name without folders, ending in `.apk`.
pub fn is_apk(name: &str) -> bool {
    name.len() <= 128 && !name.contains('/') && name.to_ascii_lowercase().ends_with(".apk") && name.len() > 4 && !name.chars().any(char::is_control)
}

/// Android's rules for an application id: dot-separated, at least two parts, each starting
/// with a letter and made of letters, digits and underscores.
pub fn is_package(s: &str) -> bool {
    s.len() <= 255
        && s.split('.').count() >= 2
        && s.split('.').all(|part| part.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

fn is_signer(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

/// One path component, safe to make a folder of.
fn is_folder(s: &str) -> bool {
    !s.is_empty() && s.len() <= 160 && s != "." && s != ".." && !s.contains(['/', '\\']) && !s.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn received_apps_get_their_own_files_whatever_they_were_renamed_to() {
        let files = [file("fx-1/base.apk"), file("photo.jpg"), file("fx-1/split_a.apk"), file("zz-1/base.apk")];
        let saved: Vec<std::path::PathBuf> = ["/d/fx-1/base (1).apk", "/d/photo.jpg", "/d/fx-1/split_a.apk", "/d/zz-1/base.apk"].iter().map(Into::into).collect();
        let got = received_apps(&[app("org.mozilla.firefox", "fx-1")], &files, &saved);
        assert_eq!(got[0].apks, vec![std::path::PathBuf::from("/d/fx-1/base (1).apk"), "/d/fx-1/split_a.apk".into()]);
    }

    fn app(package: &str, dir: &str) -> AppInfo {
        AppInfo { package: package.into(), label: "Firefox".into(), version_code: 2_015_000_001, version_name: Some("129.0".into()), signer: Some("ab".repeat(32)), dir: dir.into(), icon: None }
    }

    fn file(path: &str) -> FileEntry {
        FileEntry { path: path.into(), size: 1000 }
    }

    #[test]
    fn package_names_follow_androids_rules() {
        assert!(is_package("org.mozilla.firefox"));
        assert!(is_package("com.example.app_2"));
        assert!(!is_package("firefox"));
        assert!(!is_package("org..firefox"));
        assert!(!is_package("org.1mozilla"));
        assert!(!is_package("org.mozilla.fire-fox"));
        assert!(!is_package("../../etc"));
    }

    #[test]
    fn what_an_app_says_is_checked() {
        assert!(app("org.mozilla.firefox", "org.mozilla.firefox-1").is_valid());
        for bad in [
            AppInfo { package: "firefox".into(), ..app("a.b", "d") },
            AppInfo { label: " ".into(), ..app("a.b", "d") },
            AppInfo { label: "x\u{7}".into(), ..app("a.b", "d") },
            AppInfo { signer: Some("nothex".into()), ..app("a.b", "d") },
            AppInfo { dir: "../x".into(), ..app("a.b", "d") },
            AppInfo { dir: "a/b".into(), ..app("a.b", "d") },
            AppInfo { dir: "..".into(), ..app("a.b", "d") },
            AppInfo { icon: Some("not base64!".into()), ..app("a.b", "d") },
            AppInfo { icon: Some("A".repeat(MAX_APP_ICON * 2)), ..app("a.b", "d") },
        ] {
            assert!(!bad.is_valid(), "{bad:?}");
        }
    }

    #[test]
    fn an_app_needs_apks_in_its_folder_and_nothing_else() {
        let files = [file("fx-1/base.apk"), file("fx-1/split_config.arm64_v8a.apk"), file("photo.jpg"), file("evil-1/base.apk"), file("evil-1/run.sh"), file("nothing-1/readme")];
        let apps = vec![app("org.mozilla.firefox", "fx-1"), app("org.evil.app", "evil-1"), app("org.empty.app", "missing-1"), app("org.nothing.app", "nothing-1")];
        let kept = valid_apps(apps, &files);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].package, "org.mozilla.firefox");
    }

    #[test]
    fn two_apps_cannot_claim_one_folder() {
        let files = [file("shared/base.apk")];
        let kept = valid_apps(vec![app("org.one.app", "shared"), app("org.two.app", "shared")], &files);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].package, "org.one.app");
    }

    #[test]
    fn apk_names_are_plain() {
        assert!(is_apk("base.apk"));
        assert!(is_apk("split_config.xxhdpi.APK"));
        assert!(!is_apk(".apk"));
        assert!(!is_apk("a/b.apk"));
        assert!(!is_apk("base.apk.sh"));
    }

    #[test]
    fn what_a_newer_linksy_says_is_not_an_error() {
        let reply: PreflightMsg = serde_json::from_str(r#"{"type":"preflight_reply","direct":"quantum","apps":[{"package":"a.b","have":"soon"}]}"#).unwrap();
        let PreflightMsg::PreflightReply(r) = reply else { panic!() };
        assert_eq!(r.direct, DirectReady::Unknown);
        assert_eq!(r.apps[0].have, AppHave::Unknown);
        assert!(r.has_room_for(u64::MAX));
    }

    #[test]
    fn the_wire_is_as_documented() {
        let ask = PreflightMsg::Preflight { bytes: 1 << 31, files: 3, apps: vec![AppProbe { package: "org.mozilla.firefox".into(), version_code: 7, signer: None }] };
        assert_eq!(serde_json::to_string(&ask).unwrap(), r#"{"type":"preflight","bytes":2147483648,"files":3,"apps":[{"package":"org.mozilla.firefox","version_code":7}]}"#);
        let reply = PreflightMsg::PreflightReply(PreflightReply {
            free: Some(1_400_000_000),
            auto: false,
            direct: DirectReady::WifiOff,
            can_install: Some(true),
            apps: vec![AppState { package: "org.mozilla.firefox".into(), have: AppHave::Missing, version_code: None }],
        });
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"type":"preflight_reply","free":1400000000,"auto":false,"direct":"wifi_off","can_install":true,"apps":[{"package":"org.mozilla.firefox","have":"none"}]}"#
        );
    }
}
