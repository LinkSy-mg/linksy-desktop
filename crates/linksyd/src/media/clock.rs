//! Where a player is, for the players whose MPRIS `Position` can't be believed.
//!
//! Browsers are the usual case: Firefox and its forks keep `Position` at 0 for as long as
//! the page doesn't tell them where it is (YouTube doesn't), while the player plays on.
//! Sent as it is, a phone's bar sits at 0:00 and a seek lands in the wrong place. So each
//! player gets a clock that is started from what is certain (a new track begins at 0, a
//! `Seeked` signal says where it went) and runs while the player plays. A player that does
//! report its position is believed as before.

use std::time::{Duration, Instant};

/// How long a player plays with its `Position` still 0 before it stops being believed.
const STUCK_AFTER: Duration = Duration::from_secs(2);

/// What tells one track from the next.
#[derive(Clone, PartialEq, Debug)]
pub struct Track {
    pub title: String,
    pub length_ms: Option<u64>,
}

#[derive(Debug)]
pub struct Clock {
    track: Track,
    /// Where the player was at `since`, in milliseconds.
    base_ms: u64,
    since: Instant,
    playing: bool,
    /// Its own `Position` stayed at 0 while it played: use this clock instead.
    stuck: bool,
}

impl Clock {
    pub fn new(track: Track, playing: bool, now: Instant) -> Self {
        Self { track, base_ms: 0, since: now, playing, stuck: false }
    }

    /// Where the player is now, by this clock.
    pub fn at(&self, now: Instant) -> u64 {
        let ran = if self.playing { now.saturating_duration_since(self.since).as_millis() as u64 } else { 0 };
        let here = self.base_ms + ran;
        self.track.length_ms.map_or(here, |l| here.min(l))
    }

    /// Whether this clock, not the player, says where it is.
    pub fn stuck(&self) -> bool {
        self.stuck
    }

    /// A seek just made, or a `Seeked` signal: the player is at `ms` from now on.
    pub fn seeked(&mut self, ms: u64, now: Instant) {
        self.base_ms = ms;
        self.since = now;
    }

    /// One look at the player: what it says it is playing, whether it plays, its own
    /// `Position`, and the place and time of a `Seeked` signal heard since the last look.
    /// Returns where it is.
    pub fn look(&mut self, track: Track, playing: bool, raw_ms: Option<u64>, seeked: Option<(u64, Instant)>, now: Instant) -> Option<u64> {
        if track != self.track {
            // A new track begins at its start, whatever the last one was doing
            *self = Self::new(track, playing, now);
            if let Some(r) = raw_ms.filter(|r| *r > 0) {
                self.base_ms = r;
            }
            return raw_ms;
        }
        if let Some((ms, at)) = seeked {
            self.seeked(ms, at);
        } else if playing != self.playing {
            self.base_ms = self.at(now);
            self.since = now;
        }
        self.playing = playing;
        if let Some(r) = raw_ms.filter(|r| *r > 0) {
            // It knows where it is: believe it, and keep up for the day it stops
            self.stuck = false;
            self.base_ms = r;
            self.since = now;
            return raw_ms;
        }
        if playing && self.at(now) >= STUCK_AFTER.as_millis() as u64 {
            self.stuck = true;
        }
        if self.stuck { Some(self.at(now)) } else { raw_ms }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str) -> Track {
        Track { title: title.into(), length_ms: Some(250_000) }
    }

    fn secs(start: Instant, s: u64) -> Instant {
        start + Duration::from_secs(s)
    }

    #[test]
    fn a_player_that_knows_where_it_is_is_believed() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        assert_eq!(clock.look(track("a"), true, Some(61_000), None, secs(t0, 5)), Some(61_000));
        assert_eq!(clock.look(track("a"), true, Some(66_000), None, secs(t0, 10)), Some(66_000));
        assert!(!clock.stuck());
    }

    #[test]
    fn a_position_stuck_at_0_is_replaced_by_a_clock() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        // Just started: 0 is believable
        assert_eq!(clock.look(track("a"), true, Some(0), None, secs(t0, 1)), Some(0));
        assert!(!clock.stuck());
        // Still 0 after 3 s of playing: it isn't
        assert_eq!(clock.look(track("a"), true, Some(0), None, secs(t0, 3)), Some(3000));
        assert!(clock.stuck());
        assert_eq!(clock.look(track("a"), true, Some(0), None, secs(t0, 40)), Some(40_000));
    }

    #[test]
    fn a_player_without_a_position_gets_a_clock_too() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        assert_eq!(clock.look(track("a"), true, None, None, secs(t0, 10)), Some(10_000));
    }

    #[test]
    fn a_seek_moves_the_clock_to_where_it_went() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        clock.look(track("a"), true, Some(0), None, secs(t0, 5));
        // Seeked(100 s), and the player goes on saying 0
        assert_eq!(clock.look(track("a"), true, Some(0), Some((100_000, secs(t0, 6))), secs(t0, 6)), Some(100_000));
        assert_eq!(clock.look(track("a"), true, Some(0), None, secs(t0, 11)), Some(105_000));
    }

    #[test]
    fn a_seek_counts_from_when_it_was_heard() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        assert_eq!(clock.look(track("a"), true, Some(0), Some((100_000, secs(t0, 6))), secs(t0, 8)), Some(102_000));
    }

    #[test]
    fn a_seek_we_made_counts_the_same() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        clock.seeked(30_000, secs(t0, 2));
        assert_eq!(clock.look(track("a"), true, Some(0), None, secs(t0, 4)), Some(32_000));
    }

    #[test]
    fn pausing_stops_the_clock_and_playing_starts_it_again() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        clock.look(track("a"), true, Some(0), None, secs(t0, 10));
        assert_eq!(clock.look(track("a"), false, Some(0), None, secs(t0, 20)), Some(20_000));
        assert_eq!(clock.look(track("a"), false, Some(0), None, secs(t0, 90)), Some(20_000));
        assert_eq!(clock.look(track("a"), true, Some(0), None, secs(t0, 100)), Some(20_000));
        assert_eq!(clock.look(track("a"), true, Some(0), None, secs(t0, 110)), Some(30_000));
    }

    #[test]
    fn a_new_track_starts_at_0() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        clock.look(track("a"), true, Some(0), Some((100_000, secs(t0, 1))), secs(t0, 1));
        assert_eq!(clock.look(track("b"), true, Some(0), None, secs(t0, 50)), Some(0));
        assert_eq!(clock.look(track("b"), true, Some(0), None, secs(t0, 58)), Some(8000));
    }

    #[test]
    fn paused_at_the_start_is_just_at_the_start() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), false, t0);
        assert_eq!(clock.look(track("a"), false, Some(0), None, secs(t0, 60)), Some(0));
        assert!(!clock.stuck());
    }

    #[test]
    fn a_position_that_comes_back_is_believed_again() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        clock.look(track("a"), true, Some(0), None, secs(t0, 10));
        assert!(clock.stuck());
        assert_eq!(clock.look(track("a"), true, Some(44_000), None, secs(t0, 12)), Some(44_000));
        assert!(!clock.stuck());
    }

    #[test]
    fn the_clock_never_runs_past_the_end() {
        let t0 = Instant::now();
        let mut clock = Clock::new(track("a"), true, t0);
        assert_eq!(clock.look(track("a"), true, Some(0), None, secs(t0, 9999)), Some(250_000));
    }
}
