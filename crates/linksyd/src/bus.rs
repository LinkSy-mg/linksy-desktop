//! One session bus connection for the one-off calls: reading Do Not Disturb, locking the
//! screen, closing a notification. Each of those used to connect afresh, so a phone that
//! follows Do Not Disturb (read every 2 seconds) made a new connection every 2 seconds,
//! and a few of them were never given back: in a Flatpak the session bus runs out of them
//! in hours (linksy-releases#12). Whatever needs a connection of its own (a bus name, an
//! inhibitor, a signal stream) still makes one.

use tokio::sync::Mutex;
use zbus::Connection;

static SHARED: Mutex<Option<Connection>> = Mutex::const_new(None);

/// The shared session bus connection, made on first use and again if the bus closed it.
pub(crate) async fn session() -> zbus::Result<Connection> {
    let mut shared = SHARED.lock().await;
    if let Some(bus) = shared.as_ref().filter(|bus| !bus.is_closed()) {
        return Ok(bus.clone());
    }
    let bus = Connection::session().await?;
    *shared = Some(bus.clone());
    Ok(bus)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calls reuse one connection: the session bus sees one client, not one per call.
    #[tokio::test]
    async fn calls_share_one_connection() {
        let Ok(first) = session().await else { return }; // no session bus here (CI containers)
        for _ in 0..25 {
            let again = session().await.unwrap();
            assert_eq!(again.unique_name(), first.unique_name());
        }
    }
}
