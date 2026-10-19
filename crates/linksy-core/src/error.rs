use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("tls error: {0}")]
    Tls(#[from] rustls::Error),
    #[error("certificate generation failed: {0}")]
    Cert(#[from] rcgen::Error),
    #[error("could not connect: {0}")]
    Connect(#[from] quinn::ConnectError),
    #[error("connection lost: {0}")]
    Connection(#[from] quinn::ConnectionError),
    #[error("stream write failed: {0}")]
    Write(#[from] quinn::WriteError),
    #[error("stream read failed: {0}")]
    Read(#[from] quinn::ReadExactError),
    #[error("stream closed: {0}")]
    Closed(#[from] quinn::ClosedStream),
    #[error("invalid message: {0}")]
    Json(#[from] serde_json::Error),
    #[error("discovery error: {0}")]
    Discovery(#[from] mdns_sd::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("device is not paired")]
    NotPaired,
    #[error("device is not connected")]
    NotConnected,
    #[error("request was declined")]
    Declined,
    #[error("timed out waiting for an answer")]
    Timeout,
    #[error("file failed integrity check: {0}")]
    Integrity(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn protocol(msg: impl Into<String>) -> Error {
    Error::Protocol(msg.into())
}
