//! Away from home (docs/protocol.md §4.13): paired devices reach each other over the internet.
//!
//! iroh finds the path (straight between the two devices when the networks allow it,
//! else through a relay) and carries Linksy's own QUIC packets, unchanged and still
//! encrypted end to end with the keys pinned at pairing, exactly like the USB bridge: each
//! packet as a 2-byte big-endian length and the packet, on one iroh stream. On each side
//! the packets reach the node from `127.0.0.2`, which is how both know the connection is
//! away (and prefer the cable or the network to it).
//!
//! Each device learns the other's iroh address over a connection it already trusts (the
//! handshake, on the same network), and only takes away connections from paired devices
//! it met on the same network or cable in the last [`LEASE`].

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl, SecretKey, TransportAddr};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tracing::{debug, info};

use crate::error::{Error, Result};

/// The protocol name on iroh connections.
pub const ALPN: &[u8] = b"linksy/away/1";
/// How long after two devices last met on the same network or cable they may still reach
/// each other from away.
pub const LEASE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// The address away bridges use, so the node tells them from the cable (127.0.0.1).
pub const AWAY_IP: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
/// Largest packet the bridge carries: QUIC's are far smaller.
const MAX_PACKET: usize = 65_535;
const DIAL_TIMEOUT: Duration = Duration::from_secs(15);

/// How to reach a device away from home: its iroh id, its relay and the addresses it has
/// (sent in the handshake, so only to devices it's connected to).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwayAddr {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addrs: Vec<SocketAddr>,
}

pub(crate) struct Away {
    endpoint: Endpoint,
    relay: Option<RelayUrl>,
}

impl Away {
    /// Start the iroh endpoint with this device's away key (`away.key`, made once), using
    /// `relay` (none: direct paths only).
    pub async fn start(data_dir: &Path, relay: Option<&str>) -> Result<Arc<Self>> {
        let key = load_or_create_key(&data_dir.join("away.key"))?;
        let relay = relay.map(RelayUrl::from_str).transpose().map_err(|e| Error::Protocol(format!("relay address: {e}")))?;
        let mode = match &relay {
            Some(url) => RelayMode::Custom(RelayMap::from(url.clone())),
            None => RelayMode::Disabled,
        };
        // No address lookup service: devices tell each other their addresses themselves.
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(key)
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(mode)
            .bind()
            .await
            .map_err(|e| Error::Protocol(format!("away endpoint: {e}")))?;
        info!(id = %endpoint.id().fmt_short(), relay = ?relay.as_ref().map(|r| r.to_string()), "away from home ready");
        Ok(Arc::new(Self { endpoint, relay }))
    }

    /// This device's away address, as it is now.
    pub fn addr(&self) -> AwayAddr {
        let current = self.endpoint.addr();
        AwayAddr {
            id: self.endpoint.id().to_string(),
            relay: self.relay.as_ref().map(|r| r.to_string()),
            addrs: current.ip_addrs().copied().filter(|a| !a.ip().is_loopback()).collect(),
        }
    }

    /// Reach `to` and bridge it: returns the local address to dial (Linksy's own QUIC
    /// handshake, with the pinned key, runs through it).
    pub async fn dial(&self, to: &AwayAddr) -> Result<SocketAddr> {
        let id = EndpointId::from_str(&to.id).map_err(|e| Error::Protocol(format!("away id: {e}")))?;
        let mut parts: Vec<TransportAddr> = to.addrs.iter().copied().map(TransportAddr::Ip).collect();
        if let Some(relay) = to.relay.as_deref().and_then(|r| RelayUrl::from_str(r).ok()) {
            parts.push(TransportAddr::Relay(relay));
        }
        let addr = EndpointAddr::from_parts(id, parts);
        let conn = tokio::time::timeout(DIAL_TIMEOUT, self.endpoint.connect(addr, ALPN))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|e| Error::Protocol(format!("away: {e}")))?;
        let (send, recv) = conn.open_bi().await.map_err(|e| Error::Protocol(format!("away: {e}")))?;
        let udp = UdpSocket::bind((AWAY_IP, 0)).await?;
        let local = udp.local_addr()?;
        tokio::spawn(async move {
            carry(recv, send, udp, None).await;
            conn.close(0u32.into(), b"done");
        });
        Ok(local)
    }

    /// Take away connections from devices `allowed` says yes to, bridging each to the node
    /// listening on `node_port`.
    pub async fn accept_loop(self: Arc<Self>, node_port: u16, allowed: impl Fn(&str) -> bool + Send + Sync + 'static) {
        let allowed = Arc::new(allowed);
        while let Some(incoming) = self.endpoint.accept().await {
            let allowed = allowed.clone();
            tokio::spawn(async move {
                let Ok(accepting) = incoming.accept() else { return };
                let Ok(conn) = accepting.await else { return };
                let who = conn.remote_id().to_string();
                if !allowed(&who) {
                    debug!("away connection from an unknown or expired device refused");
                    conn.close(1u32.into(), b"not allowed");
                    return;
                }
                let Ok((send, recv)) = conn.accept_bi().await else { return };
                let Ok(udp) = UdpSocket::bind((AWAY_IP, 0)).await else { return };
                carry(recv, send, udp, Some(SocketAddr::from((Ipv4Addr::LOCALHOST, node_port)))).await;
                conn.close(0u32.into(), b"done");
            });
        }
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    if let Ok(bytes) = std::fs::read(path)
        && let Ok(raw) = <[u8; 32]>::try_from(bytes.as_slice())
    {
        return Ok(SecretKey::from_bytes(&raw));
    }
    let key = SecretKey::generate();
    std::fs::write(path, key.to_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

/// Move packets between the iroh stream and `udp` until either side stops. `node`: where
/// the node listens (the accepting side); else it's learned from the node's first packet
/// (the dialing side, whose node sends first). Each direction runs whole in its own loop,
/// so a length prefix is never read halfway.
async fn carry(mut rx: RecvStream, mut tx: SendStream, udp: UdpSocket, node: Option<SocketAddr>) {
    let learned = std::sync::OnceLock::new();
    if let Some(n) = node {
        let _ = learned.set(n);
    }
    let up = async {
        let mut buf = vec![0u8; MAX_PACKET];
        while let Ok((len, from)) = udp.recv_from(&mut buf).await {
            let _ = learned.set(from);
            if tx.write_u16(len as u16).await.is_err() || tx.write_all(&buf[..len]).await.is_err() {
                break;
            }
        }
    };
    let down = async {
        let mut packet = vec![0u8; MAX_PACKET];
        while let Ok(len) = rx.read_u16().await {
            let len = len as usize;
            if rx.read_exact(&mut packet[..len]).await.is_err() {
                break;
            }
            // The node may be busy; a lost packet is QUIC's to resend.
            if let Some(to) = learned.get() {
                let _ = udp.send_to(&packet[..len], to).await;
            }
        }
    };
    tokio::select! {
        _ = up => {}
        _ = down => {}
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use crate::identity::{DeviceInfo, DeviceKind, Fingerprint};
    use crate::node::{Link, link_of};
    use crate::trust::TrustedDevice;

    #[test]
    fn links_by_address() {
        assert_eq!(link_of("127.0.0.2:5000".parse::<SocketAddr>().unwrap()), Link::Away);
        assert_eq!(link_of("[::ffff:127.0.0.2]:5000".parse::<SocketAddr>().unwrap()), Link::Away);
        assert_eq!(link_of("127.0.0.1:5000".parse::<SocketAddr>().unwrap()), Link::Usb);
        assert_eq!(link_of("192.168.1.20:47200".parse::<SocketAddr>().unwrap()), Link::Lan);
        // The cable wins over the network, which wins over away.
        assert!(Link::Usb > Link::Lan && Link::Lan > Link::Away);
    }

    #[test]
    fn the_lease_lasts_a_week_from_the_last_meeting() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let device = DeviceInfo { id: Fingerprint::of(b"x"), name: "Pixel".into(), kind: DeviceKind::Phone, os: "android".into(), features: vec![], away: None };
        let t = |met: u64| TrustedDevice { device: device.clone(), last_addr: None, paired_at: 0, last_met: Some(met), away_all: false };
        assert!(t(now - 6 * 86_400).lease_ok());
        assert!(!t(now - 8 * 86_400).lease_ok());
    }
}
