//! Which way bytes go to a device, and whether a direct link (Wi-Fi Direct, docs/protocol.md
//! §4.17) pays for itself before a send (§4.22).
//!
//! A direct link takes seconds to set up and then moves bytes several times faster than the
//! network or the relay, so it is worth it only above some size. Where that size lies depends
//! on the devices: the core learns each route's speed from the transfers it has made with a
//! device and the setup time from the links it has made, and until it has learned anything it
//! falls back on [`Config::direct_min`](crate::Config::direct_min).

use serde::{Deserialize, Serialize};

use crate::node::Link;

/// How a connection to a device reaches it, from worst to best.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Through the internet, by Linksy's relay.
    Away,
    /// The same network.
    Lan,
    /// A Wi-Fi Direct link of the two devices' own.
    Direct,
    /// A USB cable.
    Usb,
}

impl From<Link> for Route {
    fn from(link: Link) -> Self {
        match link {
            Link::Away => Route::Away,
            Link::Lan => Route::Lan,
            Link::Usb => Route::Usb,
        }
    }
}

/// What a direct link looks like for a send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DirectPlan {
    /// One is up: the send goes over it at once.
    Up,
    /// One will be set up before the send starts (seconds, at most 45).
    Will,
    /// None, and why.
    Not { why: Skip },
}

/// Why a send doesn't use a direct link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Skip {
    /// A cable is faster than anything.
    Cable,
    /// It would take longer to set up than it saves.
    Small,
    /// The devices can't make one together (a computer without Wi-Fi Direct, two computers…).
    Unsupported,
    /// One was tried a moment ago and failed, for this reason.
    Failed { reason: String },
}

/// What a send of some size would do right now ([`Node::plan_send`](crate::Node::plan_send)).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendPlan {
    /// The route the connection has now.
    pub route: Route,
    pub direct: DirectPlan,
    /// A rough guess at how long it takes, in seconds, from what the core has seen the
    /// route do (or its usual speed): say "about", never promise it.
    pub seconds: Option<u32>,
}

/// Bytes a second, by route, when nothing has been measured: a home network, the relay, a
/// Wi-Fi Direct link (70 to 85 MB/s measured in 0.9.9), a cable.
pub const USUAL_LAN: f64 = 20e6;
pub const USUAL_AWAY: f64 = 3e6;
pub const USUAL_DIRECT: f64 = 75e6;
pub const USUAL_USB: f64 = 30e6;
/// Seconds a direct link takes to come up when none has yet.
pub const USUAL_SETUP: f64 = 10.0;

/// A transfer teaches the core a speed only if it was big enough that setup and latency
/// didn't dominate it.
const MIN_SAMPLE: u64 = 8 << 20;
const MIN_SAMPLE_SECS: f64 = 0.5;
/// How much a new measurement counts against what's known.
const WEIGHT: f64 = 0.4;

/// What the core has seen the routes to one device do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rates {
    lan: Option<f64>,
    away: Option<f64>,
    direct: Option<f64>,
    usb: Option<f64>,
    /// Seconds from asking for a direct link to having it.
    setup: Option<f64>,
}

impl Rates {
    pub fn is_empty(&self) -> bool {
        *self == Rates::default()
    }

    /// Bytes a second over `route`: measured, else usual.
    pub fn rate(&self, route: Route) -> f64 {
        match route {
            Route::Away => self.away.unwrap_or(USUAL_AWAY),
            Route::Lan => self.lan.unwrap_or(USUAL_LAN),
            Route::Direct => self.direct.unwrap_or(USUAL_DIRECT),
            Route::Usb => self.usb.unwrap_or(USUAL_USB),
        }
    }

    /// Seconds a direct link takes: seen, else usual.
    pub fn setup_secs(&self) -> f64 {
        self.setup.unwrap_or(USUAL_SETUP)
    }

    /// A transfer of `bytes` over `route` took `secs`.
    pub fn learn(&mut self, route: Route, bytes: u64, secs: f64) {
        if bytes < MIN_SAMPLE || secs < MIN_SAMPLE_SECS {
            return;
        }
        let slot = match route {
            Route::Away => &mut self.away,
            Route::Lan => &mut self.lan,
            Route::Direct => &mut self.direct,
            Route::Usb => &mut self.usb,
        };
        *slot = Some(blend(*slot, bytes as f64 / secs));
    }

    /// A direct link took `secs` to come up.
    pub fn learn_setup(&mut self, secs: f64) {
        self.setup = Some(blend(self.setup, secs.max(0.5)));
    }

    /// Whether setting up a direct link first gets `total` bytes there sooner than sending
    /// them over `link` as it is. `head_start` seconds of the setup are free (the user is
    /// still choosing what to send). With nothing measured yet, `min` bytes is the line.
    pub fn direct_pays(&self, link: Link, total: u64, min: u64, head_start: f64) -> bool {
        if link == Link::Usb {
            return false;
        }
        if self.is_empty() && head_start == 0.0 {
            return match link {
                Link::Lan => total >= min,
                _ => total >= min / 8,
            };
        }
        let total = total as f64;
        let now = total / self.rate(link.into());
        let direct = (self.setup_secs() - head_start).max(0.0) + total / self.rate(Route::Direct);
        now > direct
    }

    /// Seconds `bytes` take over `route`, or after a setup first.
    pub fn estimate(&self, route: Route, bytes: u64, setup: bool) -> u32 {
        let secs = bytes as f64 / self.rate(route) + if setup { self.setup_secs() } else { 0.0 };
        secs.round().clamp(0.0, u32::MAX as f64) as u32
    }
}

fn blend(old: Option<f64>, sample: f64) -> f64 {
    match old {
        Some(old) => old * (1.0 - WEIGHT) + sample * WEIGHT,
        None => sample,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 256 << 20;

    #[test]
    fn nothing_measured_the_line_is_direct_min() {
        let r = Rates::default();
        assert!(r.direct_pays(Link::Lan, MIN, MIN, 0.0));
        assert!(!r.direct_pays(Link::Lan, MIN - 1, MIN, 0.0));
        assert!(r.direct_pays(Link::Away, MIN / 8, MIN, 0.0));
        assert!(!r.direct_pays(Link::Usb, u64::MAX, MIN, 0.0));
    }

    #[test]
    fn the_usual_speeds_put_the_line_where_direct_min_does() {
        // The usual numbers are chosen so that the estimate agrees with the cold line
        let r = Rates { lan: Some(USUAL_LAN), ..Rates::default() };
        assert!(r.direct_pays(Link::Lan, 300 << 20, MIN, 0.0));
        assert!(!r.direct_pays(Link::Lan, 200 << 20, MIN, 0.0));
        let relay = Rates { away: Some(USUAL_AWAY), ..Rates::default() };
        assert!(relay.direct_pays(Link::Away, 40 << 20, MIN, 0.0));
        assert!(!relay.direct_pays(Link::Away, 20 << 20, MIN, 0.0));
    }

    #[test]
    fn a_fast_network_pushes_the_line_up() {
        let mut r = Rates::default();
        r.learn(Route::Lan, 100 << 20, 1.0);
        // Over 100 MB/s, Wi-Fi Direct at 75 MB/s never wins
        assert!(!r.direct_pays(Link::Lan, 10 << 30, MIN, 0.0));
    }

    #[test]
    fn a_slow_network_pulls_it_down() {
        let mut r = Rates::default();
        r.learn(Route::Lan, 20 << 20, 10.0);
        // 2 MB/s: 100 MB takes 50 s, against 10 s plus a second or two
        assert!(r.direct_pays(Link::Lan, 100 << 20, MIN, 0.0));
    }

    #[test]
    fn a_head_start_makes_smaller_sends_pay() {
        let mut r = Rates::default();
        r.learn(Route::Lan, 20 << 20, 1.0);
        let total = 120 << 20;
        assert!(!r.direct_pays(Link::Lan, total, MIN, 0.0));
        assert!(r.direct_pays(Link::Lan, total, MIN, 8.0));
    }

    #[test]
    fn small_transfers_teach_nothing() {
        let mut r = Rates::default();
        r.learn(Route::Lan, 1 << 20, 1.0);
        r.learn(Route::Lan, 100 << 20, 0.1);
        assert!(r.is_empty());
    }

    #[test]
    fn what_is_measured_is_blended() {
        let mut r = Rates::default();
        r.learn(Route::Direct, 100 << 20, 2.0);
        let first = r.rate(Route::Direct);
        r.learn(Route::Direct, 100 << 20, 4.0);
        assert!(r.rate(Route::Direct) < first);
        assert!(r.rate(Route::Direct) > first / 2.0);
    }

    #[test]
    fn estimates_include_the_setup() {
        let r = Rates::default();
        assert_eq!(r.estimate(Route::Direct, 750_000_000, true), 20);
        assert_eq!(r.estimate(Route::Lan, 200_000_000, false), 10);
    }

    #[test]
    fn routes_order_from_worst_to_best() {
        assert!(Route::Away < Route::Lan && Route::Lan < Route::Direct && Route::Direct < Route::Usb);
    }
}
