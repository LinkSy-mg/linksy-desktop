//! QUIC/TLS configuration. Both sides present their device certificate (mutual TLS).
//! Certificates are self-signed, so the handshake accepts any certificate with a valid
//! signature; trust is decided afterwards by comparing the peer fingerprint against the
//! trust store (or by the pairing code for new devices).

use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};

use crate::error::{Error, Result};
use crate::identity::{Fingerprint, Identity};

pub const ALPN: &[u8] = b"linksy/1";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn transport() -> Arc<quinn::TransportConfig> {
    let mut t = quinn::TransportConfig::default();
    // LAN links are fast and short: large windows keep a single stream near line rate.
    t.stream_receive_window((16u32 << 20).into());
    t.receive_window((32u32 << 20).into());
    t.send_window(32 << 20);
    t.keep_alive_interval(Some(Duration::from_secs(10)));
    t.max_idle_timeout(Some(Duration::from_secs(30).try_into().expect("valid idle timeout")));
    Arc::new(t)
}

pub fn server_config(id: &Identity) -> Result<quinn::ServerConfig> {
    let p = provider();
    let mut tls = rustls::ServerConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(AnyCert(p.signature_verification_algorithms)))
        .with_single_cert(vec![id.cert.clone()], id.private_key())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = QuicServerConfig::try_from(tls).map_err(|e| Error::Protocol(e.to_string()))?;
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(quic));
    cfg.transport_config(transport());
    Ok(cfg)
}

pub fn client_config(id: &Identity) -> Result<quinn::ClientConfig> {
    let p = provider();
    let mut tls = rustls::ClientConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert(p.signature_verification_algorithms)))
        .with_client_auth_cert(vec![id.cert.clone()], id.private_key())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = QuicClientConfig::try_from(tls).map_err(|e| Error::Protocol(e.to_string()))?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(quic));
    cfg.transport_config(transport());
    Ok(cfg)
}

/// Fingerprint of the certificate the peer presented during the handshake.
pub fn peer_fingerprint(conn: &quinn::Connection) -> Result<Fingerprint> {
    let certs = conn
        .peer_identity()
        .and_then(|id| id.downcast::<Vec<CertificateDer<'static>>>().ok())
        .ok_or_else(|| Error::Protocol("peer sent no certificate".into()))?;
    let cert = certs.first().ok_or_else(|| Error::Protocol("peer sent no certificate".into()))?;
    Ok(Fingerprint::of(cert))
}

/// Six-digit code both users compare when pairing. It is derived from the TLS session
/// itself, so a man in the middle (who would hold two different sessions) cannot make
/// both screens show the same code.
pub fn pairing_code(conn: &quinn::Connection) -> Result<String> {
    let mut out = [0u8; 8];
    conn.export_keying_material(&mut out, b"linksy pairing code v1", b"")
        .map_err(|_| Error::Protocol("could not derive pairing code".into()))?;
    let n = u64::from_be_bytes(out) % 1_000_000;
    Ok(format!("{n:06}"))
}

/// Accepts any certificate whose handshake signature verifies. Identity is checked by
/// fingerprint after the handshake.
#[derive(Debug)]
struct AnyCert(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOffered))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

impl ClientCertVerifier for AnyCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOffered))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}
