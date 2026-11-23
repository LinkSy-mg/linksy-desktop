//! Decides, event by event, whether the keyboard and mouse belong here or to the other
//! computer. It only says what to do; reading the devices and writing the virtual ones is
//! done by the caller.
//!
//! The hotkey is seen while the devices are shared with this computer. Taking them over
//! waits until every key and button is up, so nothing stays pressed on this side, and
//! giving them back lifts whatever is still down on the other.

use std::collections::BTreeSet;

use crate::chord::Chord;
use crate::event::Event;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The devices work here; the hotkey is watched.
    Here,
    /// The hotkey was pressed; taking over waits for every key to be up.
    Taking,
    /// The devices are taken over and everything goes to the other computer.
    There,
    /// The hotkey was pressed again; giving back waits for every key to be up.
    Giving,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Take the devices from this computer (an exclusive grab).
    Grab,
    /// Give them back.
    Ungrab,
    /// Send to the other computer.
    Send(Event),
}

pub struct Router {
    chord: Chord,
    state: State,
    down: BTreeSet<u16>,
    /// Keys and buttons the other computer was told are down.
    sent_down: BTreeSet<u16>,
}

impl Router {
    pub fn new(chord: Chord) -> Self {
        Self { chord, state: State::Here, down: BTreeSet::new(), sent_down: BTreeSet::new() }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn on_event(&mut self, ev: Event) -> Vec<Action> {
        let was = self.chord.satisfied(&self.down);
        if ev.is_key() && ev.carried() {
            if ev.value == 1 {
                self.down.insert(ev.code);
            } else {
                self.down.remove(&ev.code);
            }
        }
        let pressed = !was && ev.is_key() && ev.value == 1 && self.chord.satisfied(&self.down);
        let mut out = Vec::new();
        match self.state {
            State::Here => {
                if pressed {
                    self.state = State::Taking;
                    self.step_when_idle(&mut out);
                }
            }
            State::Taking => self.step_when_idle(&mut out),
            State::There => {
                if pressed {
                    self.lift(&mut out);
                    self.state = State::Giving;
                    self.step_when_idle(&mut out);
                } else if ev.carried() {
                    if ev.is_key() {
                        if ev.value == 1 {
                            self.sent_down.insert(ev.code);
                        } else {
                            self.sent_down.remove(&ev.code);
                        }
                    }
                    out.push(Action::Send(ev));
                }
            }
            State::Giving => self.step_when_idle(&mut out),
        }
        out
    }

    /// The link is gone, or the helper is stopping: back to this computer at once.
    pub fn abort(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        if matches!(self.state, State::There | State::Giving) {
            self.lift(&mut out);
            out.push(Action::Ungrab);
        }
        self.state = State::Here;
        self.down.clear();
        out
    }

    fn step_when_idle(&mut self, out: &mut Vec<Action>) {
        if !self.down.is_empty() {
            return;
        }
        match self.state {
            State::Taking => {
                self.state = State::There;
                out.push(Action::Grab);
            }
            State::Giving => {
                self.state = State::Here;
                out.push(Action::Ungrab);
            }
            State::Here | State::There => {}
        }
    }

    fn lift(&mut self, out: &mut Vec<Action>) {
        if self.sent_down.is_empty() {
            return;
        }
        out.extend(std::mem::take(&mut self.sent_down).into_iter().map(|code| Action::Send(Event::key(code, false))));
        out.push(Action::Send(Event::sync()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EV_REL, REL_X};

    const LCTRL: u16 = 29;
    const RCTRL: u16 = 97;
    const A: u16 = 30;
    const SHIFT: u16 = 42;

    fn router() -> Router {
        Router::new(Chord::parse(crate::chord::DEFAULT).unwrap())
    }

    fn feed(r: &mut Router, events: &[Event]) -> Vec<Action> {
        events.iter().flat_map(|e| r.on_event(*e)).collect()
    }

    fn chord_down() -> [Event; 2] {
        [Event::key(LCTRL, true), Event::key(RCTRL, true)]
    }

    fn chord_up() -> [Event; 2] {
        [Event::key(LCTRL, false), Event::key(RCTRL, false)]
    }

    fn take_over(r: &mut Router) {
        feed(r, &chord_down());
        assert_eq!(feed(r, &chord_up()), vec![Action::Grab]);
        assert_eq!(r.state(), State::There);
    }

    #[test]
    fn nothing_leaves_while_the_devices_are_here() {
        let mut r = router();
        let out = feed(&mut r, &[Event::key(A, true), Event::key(A, false), Event { kind: EV_REL, code: REL_X, value: 5 }]);
        assert!(out.is_empty());
        assert_eq!(r.state(), State::Here);
    }

    #[test]
    fn the_hotkey_takes_over_only_once_every_key_is_up() {
        let mut r = router();
        assert!(feed(&mut r, &chord_down()).is_empty());
        assert_eq!(r.state(), State::Taking);
        assert!(feed(&mut r, &[Event::key(LCTRL, false)]).is_empty());
        assert_eq!(feed(&mut r, &[Event::key(RCTRL, false)]), vec![Action::Grab]);
        assert_eq!(r.state(), State::There);
    }

    #[test]
    fn once_taken_over_everything_goes_to_the_other_computer() {
        let mut r = router();
        take_over(&mut r);
        let motion = Event { kind: EV_REL, code: REL_X, value: 7 };
        let out = feed(&mut r, &[Event::key(A, true), motion, Event::sync(), Event::key(A, false)]);
        assert_eq!(out, vec![Action::Send(Event::key(A, true)), Action::Send(motion), Action::Send(Event::sync()), Action::Send(Event::key(A, false))]);
    }

    #[test]
    fn auto_repeat_is_not_sent() {
        let mut r = router();
        take_over(&mut r);
        feed(&mut r, &[Event::key(A, true)]);
        assert!(feed(&mut r, &[Event { kind: crate::event::EV_KEY, code: A, value: 2 }]).is_empty());
    }

    #[test]
    fn the_hotkey_again_lifts_what_is_still_down_over_there_and_gives_back_when_idle() {
        let mut r = router();
        take_over(&mut r);
        feed(&mut r, &[Event::key(SHIFT, true), Event::key(A, true), Event::key(LCTRL, true)]);
        let out = r.on_event(Event::key(RCTRL, true));
        let lifted = [LCTRL, A, SHIFT].map(|k| Action::Send(Event::key(k, false)));
        assert_eq!(out, [lifted.to_vec(), vec![Action::Send(Event::sync())]].concat());
        assert_eq!(r.state(), State::Giving);
        assert!(feed(&mut r, &[Event::key(A, false), Event::key(SHIFT, false), Event::key(LCTRL, false)]).is_empty());
        assert_eq!(feed(&mut r, &[Event::key(RCTRL, false)]), vec![Action::Ungrab]);
        assert!(r.abort().is_empty(), "nothing is left to lift or give back");
        assert_eq!(r.state(), State::Here);
    }

    #[test]
    fn while_giving_back_nothing_more_is_sent() {
        let mut r = router();
        take_over(&mut r);
        feed(&mut r, &chord_down());
        assert!(feed(&mut r, &[Event { kind: EV_REL, code: REL_X, value: 3 }, Event::sync()]).is_empty());
    }

    #[test]
    fn an_abort_lifts_everything_and_gives_the_devices_back_at_once() {
        let mut r = router();
        take_over(&mut r);
        feed(&mut r, &[Event::key(A, true)]);
        let out = r.abort();
        assert_eq!(out, vec![Action::Send(Event::key(A, false)), Action::Send(Event::sync()), Action::Ungrab]);
        assert_eq!(r.state(), State::Here);
        assert!(r.abort().is_empty());
    }

    #[test]
    fn an_abort_before_the_takeover_has_nothing_to_undo() {
        let mut r = router();
        feed(&mut r, &chord_down());
        assert_eq!(r.state(), State::Taking);
        assert!(r.abort().is_empty());
        assert_eq!(r.state(), State::Here);
    }

    #[test]
    fn it_can_be_switched_back_and_forth() {
        let mut r = router();
        for _ in 0..3 {
            take_over(&mut r);
            feed(&mut r, &chord_down());
            assert_eq!(feed(&mut r, &chord_up()), vec![Action::Ungrab]);
            assert_eq!(r.state(), State::Here);
        }
    }
}
