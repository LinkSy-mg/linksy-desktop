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

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    std::fs::write(path, bytes)
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
}
