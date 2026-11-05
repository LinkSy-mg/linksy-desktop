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
    /// What was asked for isn't there (a file on another device); for the user.
    #[error("{0}")]
    NotFound(String),
    /// A request that can't be carried out as asked; the message is for the user.
    #[error("{0}")]
    Invalid(String),
    #[error("device is not paired")]
    NotPaired,
    #[error("device isn't reachable right now")]
    Unreachable,
    #[error("request was declined")]
    Declined,
    /// The other device can't do this kind of session (yet), whatever its user wants.
    #[error("the other device can't do this yet")]
    Unsupported,
    /// Asked from away (over the internet), where the other device only allows it if
    /// its user said so for this device.
    #[error("the other device allows this only on the same network. To use it from away, open Linksy on that device and turn on Allow Everything From Away for this one")]
    NotFromAway,
    /// A terminal turned down, with why (`off`, `code`, `locked`).
    #[error("terminal refused: {reason}")]
    Refused { reason: String, tries_left: Option<u32> },
    #[error("timed out waiting for an answer")]
    Timeout,
    #[error("file failed integrity check: {0}")]
    Integrity(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn protocol(msg: impl Into<String>) -> Error {
    Error::Protocol(msg.into())
}

pub(crate) fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

/// What a device said when it turned a request down: its words, except that a refusal because
/// the asker is away from home is the error for that, which says what to do about it.
pub(crate) fn refusal(message: String) -> Error {
    if message.contains("only on the same network") { Error::NotFromAway } else { Error::Invalid(message) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_for_being_away_says_what_to_do() {
        let e = refusal("this device shows its photos only on the same network".into());
        assert!(matches!(e, Error::NotFromAway));
        assert!(e.to_string().contains("Allow Everything From Away"));
        assert!(matches!(refusal("this device shares its files only on the same network".into()), Error::NotFromAway));
    }

    #[test]
    fn any_other_refusal_is_kept_as_it_was_said() {
        assert!(matches!(refusal("this device doesn't share its photos".into()), Error::Invalid(m) if m == "this device doesn't share its photos"));
    }
}
