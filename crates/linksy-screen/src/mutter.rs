//! GNOME's display layout, for a second display: GNOME can stop feeding a virtual
//! monitor's stream when its layout changes, and the capture then starts again, but only
//! once GNOME's "Keep these display settings?" is answered, which a change of monitors
//! would otherwise revert. (Known issue: settings changed while a second display runs can
//! still revert.)

use std::time::Duration;

const DEST: &str = "org.gnome.Mutter.DisplayConfig";
const PATH: &str = "/org/gnome/Mutter/DisplayConfig";

/// The layout's serial: it changes with every layout applied. None where it isn't GNOME.
async fn serial() -> Option<u32> {
    let bus = zbus::Connection::session().await.ok()?;
    let reply = bus.call_method(Some(DEST), PATH, Some(DEST), "GetCurrentState", &()).await.ok()?;
    // Only the serial; the rest of the reply (monitors, layout) isn't needed
    let body = reply.body();
    let structure: zbus::zvariant::Structure = body.deserialize().ok()?;
    structure.fields().first().and_then(|v| u32::try_from(v).ok())
}

/// After a layout change, wait for the next one, at most `limit` (GNOME's answer: Keep,
/// Revert, or its 20-second timeout).
pub async fn settled(limit: Duration) {
    let Some(start) = serial().await else { return };
    let until = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if serial().await.is_some_and(|s| s != start) {
            // The answer's own layout lands too: a moment for it
            tokio::time::sleep(Duration::from_millis(500)).await;
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    /// Needs a GNOME session.
    #[tokio::test]
    #[ignore = "needs GNOME"]
    async fn reads_the_layout_serial() {
        assert!(super::serial().await.is_some());
    }
}
