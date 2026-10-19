//! Pairing invites, shown as a QR code: `linksy://pair?v=1&id=…&n=…&a=…&t=…`
//!
//! The invite carries the device id, so the scanner can check the certificate it reaches
//! is the one in the code, and a one-time token, so the device showing the code knows
//! the connection comes from whoever scanned it. Together they replace comparing codes.

use std::fmt::Write;
use std::net::SocketAddr;

use crate::error::{Result, invalid};
use crate::identity::Fingerprint;

pub const SCHEME: &str = "linksy://pair";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub id: Fingerprint,
    pub name: String,
    pub addrs: Vec<SocketAddr>,
    pub token: String,
}

impl Invite {
    pub fn to_uri(&self) -> String {
        let addrs: Vec<String> = self.addrs.iter().map(SocketAddr::to_string).collect();
        format!("{SCHEME}?v=1&id={}&n={}&a={}&t={}", self.id, encode(&self.name), encode(&addrs.join(",")), self.token)
    }

    pub fn parse(uri: &str) -> Result<Self> {
        let bad = || invalid("that isn't a Linksy pairing code");
        let query = uri.trim().strip_prefix(SCHEME).and_then(|r| r.strip_prefix('?')).ok_or_else(bad)?;
        let (mut v, mut id, mut name, mut addrs, mut token) = (None, None, None, None, None);
        for pair in query.split('&') {
            let (k, val) = pair.split_once('=').ok_or_else(bad)?;
            let val = decode(val).ok_or_else(bad)?;
            match k {
                "v" => v = Some(val),
                "id" => id = Some(val.parse::<Fingerprint>().map_err(|_| bad())?),
                "n" => name = Some(val),
                "a" => addrs = Some(val.split(',').filter(|a| !a.is_empty()).map(str::parse).collect::<Result<Vec<SocketAddr>, _>>().map_err(|_| bad())?),
                "t" => token = Some(val),
                _ => {} // newer fields
            }
        }
        if v.as_deref() != Some("1") {
            return Err(invalid("this pairing code needs a newer version of Linksy"));
        }
        let token = token.filter(|t| t.len() >= 16).ok_or_else(bad)?;
        let addrs = addrs.filter(|a| !a.is_empty()).ok_or_else(bad)?;
        Ok(Self { id: id.ok_or_else(bad)?, name: name.unwrap_or_default(), addrs, token })
    }
}

/// Percent-encode everything but unreserved characters (RFC 3986).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Compare tokens without leaking where they differ.
pub(crate) fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let invite = Invite {
            id: crate::identity::Identity::generate().unwrap().fingerprint,
            name: "Axel's Desk & Co ✓".into(),
            addrs: vec!["192.168.1.20:47200".parse().unwrap(), "[fd00::1]:47200".parse().unwrap()],
            token: "0123456789abcdef0123456789abcdef".into(),
        };
        let uri = invite.to_uri();
        assert!(uri.starts_with("linksy://pair?v=1&id="));
        assert!(!uri.contains(' '));
        assert_eq!(Invite::parse(&uri).unwrap(), invite);
    }

    #[test]
    fn rejects_junk() {
        for bad in ["", "https://example.com", "linksy://pair?v=1", "linksy://pair?v=2&id=00&a=1.2.3.4:1&t=0123456789abcdef"] {
            assert!(Invite::parse(bad).is_err(), "{bad}");
        }
        assert!(same_token("abc", "abc"));
        assert!(!same_token("abc", "abd"));
        assert!(!same_token("abc", "abcd"));
    }
}
