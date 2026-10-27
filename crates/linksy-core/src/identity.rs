//! Persistent device identity: a self-signed certificate whose SHA-256 fingerprint is the
//! device id. Peers pin this fingerprint when pairing, so no CA is involved.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Result;

/// SHA-256 of a device certificate. Stable for the life of an install.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn of(cert: &[u8]) -> Self {
        Self(Sha256::digest(cert).into())
    }

    /// Short form for display ("A1B2-C3D4").
    pub fn short(&self) -> String {
        let h = hex::encode_upper(&self.0[..4]);
        format!("{}-{}", &h[..4], &h[4..])
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.short())
    }
}

impl FromStr for Fingerprint {
    type Err = hex::FromHexError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut out = [0u8; 32];
        hex::decode_to_slice(s, &mut out)?;
        Ok(Self(out))
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Desktop,
    Laptop,
    Phone,
    Tablet,
}

/// What a device tells peers about itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: Fingerprint,
    pub name: String,
    pub kind: DeviceKind,
    pub os: String,
    /// What this device can do for others (`"files"`, `"sms"`, `"control"`…), so they can
    /// hide what it can't. Empty: not said (older versions, or seen only on the network).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// How to reach it away from home (docs/protocol.md §4.13); only in the handshake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub away: Option<crate::away::AwayAddr>,
}

pub struct Identity {
    pub cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    pub fingerprint: Fingerprint,
}

impl Identity {
    /// Load `cert.der`/`key.der` from `dir`, creating a new identity on first run.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let cert_path = dir.join("cert.der");
        let key_path = dir.join("key.der");
        if cert_path.exists() && key_path.exists() {
            let cert = std::fs::read(&cert_path)?;
            let key = std::fs::read(&key_path)?;
            return Ok(Self::from_der(cert, key));
        }
        let id = Self::generate()?;
        std::fs::create_dir_all(dir)?;
        write_private(&key_path, id.key.secret_pkcs8_der())?;
        std::fs::write(&cert_path, id.cert.as_ref())?;
        Ok(id)
    }

    pub fn generate() -> Result<Self> {
        let ck = rcgen::generate_simple_self_signed(vec!["linksy".to_string()])?;
        Ok(Self::from_der(ck.cert.der().to_vec(), ck.signing_key.serialize_der()))
    }

    fn from_der(cert: Vec<u8>, key: Vec<u8>) -> Self {
        let fingerprint = Fingerprint::of(&cert);
        Self { cert: CertificateDer::from(cert), key: PrivatePkcs8KeyDer::from(key), fingerprint }
    }

    pub fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(self.key.clone_key())
    }
}

/// Write a file that only this user can read, private from the moment it is created.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        // The mode above only applies when the file is new; a leftover one keeps its old mode.
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    std::fs::write(path, bytes)
}

/// State files that hold keys, tokens, paired devices or history; made private on start.
const PRIVATE_FILES: [&str; 12] = [
    "key.der",
    "discovery.key",
    "away.key",
    "trusted.json",
    "history.json",
    "files.json",
    "terminal.json",
    "notifications.json",
    "photos.json",
    "lock.json",
    "usb-auto.json",
    "update.json",
];

/// Create the data directory private to this user (0700), and tighten the state files in
/// it if they already exist with looser permissions. An existing directory is tightened
/// too, unless `keep_dir_mode` (the user chose it with `--data-dir`) says to leave it.
/// Best effort past the create: a failed tighten is logged, never fatal.
pub fn create_private_dir(dir: &Path, keep_dir_mode: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let existed = dir.is_dir();
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        if existed && keep_dir_mode {
            tracing::info!("left the mode of {} as it is", dir.display());
        } else {
            tighten(dir, 0o700);
        }
        for name in PRIVATE_FILES {
            tighten(&dir.join(name), 0o600);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = keep_dir_mode;
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

#[cfg(unix)]
fn tighten(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => tracing::warn!("could not restrict {}: {e}", path.display()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_persists() {
        let dir = tempfile::tempdir().unwrap();
        let a = Identity::load_or_create(dir.path()).unwrap();
        let b = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn fingerprint_roundtrip() {
        let fp = Identity::generate().unwrap().fingerprint;
        assert_eq!(fp.to_string().parse::<Fingerprint>().unwrap(), fp);
        assert_eq!(fp.short().len(), 9);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_and_dir_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let root = tempfile::tempdir().unwrap();

        let fresh = root.path().join("a/b");
        create_private_dir(&fresh, false).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        write_private(&fresh.join("away.key"), b"k").unwrap();
        assert_eq!(mode(&fresh.join("away.key")), 0o600);

        // An older install left them readable by everyone.
        let old = root.path().join("old");
        std::fs::create_dir(&old).unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        for name in PRIVATE_FILES {
            std::fs::write(old.join(name), b"x").unwrap();
        }
        create_private_dir(&old, false).unwrap();
        assert_eq!(mode(&old), 0o700);
        for name in PRIVATE_FILES {
            assert_eq!(mode(&old.join(name)), 0o600, "{name}");
        }

        // A directory the user chose keeps its mode; the files in it are still tightened.
        let shared = root.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(shared.join("trusted.json"), b"x").unwrap();
        std::fs::set_permissions(shared.join("trusted.json"), std::fs::Permissions::from_mode(0o644)).unwrap();
        create_private_dir(&shared, true).unwrap();
        assert_eq!(mode(&shared), 0o755);
        assert_eq!(mode(&shared.join("trusted.json")), 0o600);
        // ...but one we make ourselves is private either way.
        let made = root.path().join("made");
        create_private_dir(&made, true).unwrap();
        assert_eq!(mode(&made), 0o700);

        // A stale temp file from a crash does not keep its old mode.
        let stale = root.path().join("trusted.json.tmp");
        std::fs::write(&stale, b"old").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&stale, b"new").unwrap();
        assert_eq!(mode(&stale), 0o600);
    }
}
