//! The two ends of a Handover link, over the socket the service gives the program. The
//! sender turns the keyboard and mouse it watches into a stream of events; the receiver
//! turns that stream into a virtual keyboard and mouse. The service carries the bytes
//! between the two computers and tells the person when the devices change hands.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::event::{EV_KEY, EV_REL, Event, KEY_MAX};
use crate::router::{Action, Router, State};

/// A message to the service, not an input event: the devices went to the other computer
/// (value 1) or came back (value 0).
pub const EV_MARK: u16 = 0xfe;
const MARK_HANDED: u16 = 1;

pub fn mark(handed_over: bool) -> Event {
    Event { kind: EV_MARK, code: MARK_HANDED, value: handed_over as i32 }
}

/// `Some(true)` when the devices were just handed over, `Some(false)` when they came back.
pub fn marked(event: Event) -> Option<bool> {
    (event.kind == EV_MARK && event.code == MARK_HANDED).then_some(event.value != 0)
}

const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// The longest a wheel or pointer step from the other computer may be.
const REL_LIMIT: i32 = 4096;

/// What the sender watches: the real keyboards and mice.
pub trait Source {
    /// Events within `timeout_ms`, and whether `link` became readable or hung up.
    fn poll(&mut self, link: RawFd, timeout_ms: i32) -> io::Result<(Vec<Event>, bool)>;
    fn grab(&self, on: bool) -> io::Result<()>;
    /// Nothing left to watch (every device was unplugged).
    fn empty(&self) -> bool {
        false
    }
}

impl Source for crate::evdev::Capture {
    fn poll(&mut self, link: RawFd, timeout_ms: i32) -> io::Result<(Vec<Event>, bool)> {
        self.poll_with(link, timeout_ms)
    }

    fn grab(&self, on: bool) -> io::Result<()> {
        crate::evdev::Capture::grab(self, on)
    }

    fn empty(&self) -> bool {
        self.devices().is_empty()
    }
}

/// Where the receiver puts what arrives: the virtual keyboard and mouse.
pub trait Sink {
    fn emit(&mut self, event: Event) -> io::Result<()>;
}

impl Sink for crate::uinput::VirtualDevice {
    fn emit(&mut self, event: Event) -> io::Result<()> {
        crate::uinput::VirtualDevice::emit(self, event)
    }
}

/// Watch the devices and send, over `link`, what belongs to the other computer. Ends when
/// `keep_going` says so, when the link closes, or when no device is left; then everything
/// is given back.
pub fn run_send(source: &mut impl Source, link: &mut UnixStream, mut router: Router, keep_going: impl Fn() -> bool) -> io::Result<()> {
    link.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut state = router.state();
    let mut result = Ok(());
    while keep_going() && !source.empty() {
        let (events, link_ready) = match source.poll(link.as_raw_fd(), 200) {
            Ok(polled) => polled,
            Err(e) => {
                result = Err(e);
                break;
            }
        };
        if link_ready && !drain(link) {
            break;
        }
        let mut out = Vec::new();
        for event in events {
            for action in router.on_event(event) {
                if let Err(e) = apply(action, source, &mut out) {
                    result = Err(e);
                }
            }
            if router.state() != state {
                if let Some(now_there) = handed(state, router.state()) {
                    out.extend(mark(now_there).to_bytes());
                }
                state = router.state();
            }
        }
        if result.is_err() || (!out.is_empty() && link.write_all(&out).is_err()) {
            break;
        }
    }
    let mut out = Vec::new();
    for action in router.abort() {
        let _ = apply(action, source, &mut out);
    }
    if state == State::There || state == State::Giving {
        out.extend(mark(false).to_bytes());
    }
    let _ = link.write_all(&out);
    result
}

fn handed(before: State, after: State) -> Option<bool> {
    match (before, after) {
        (State::Taking, State::There) => Some(true),
        (State::Giving, State::Here) => Some(false),
        _ => None,
    }
}

fn apply(action: Action, source: &impl Source, out: &mut Vec<u8>) -> io::Result<()> {
    match action {
        Action::Grab => source.grab(true),
        Action::Ungrab => source.grab(false),
        Action::Send(event) => {
            out.extend(event.to_bytes());
            Ok(())
        }
    }
}

/// Read and drop whatever the service sent (nothing is expected); false once it hung up.
fn drain(link: &UnixStream) -> bool {
    let mut buf = [0u8; 64];
    loop {
        // SAFETY: `buf` is valid for its length; MSG_DONTWAIT keeps it from blocking
        let n = unsafe { libc::recv(link.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT) };
        match n {
            0 => return false,
            n if n < 0 => return io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock,
            _ => {}
        }
    }
}

/// Put what arrives over `link` on `sink` until the link closes. Whatever is still down
/// is lifted at the end, so the keyboard here is never left holding a key.
pub fn run_receive(sink: &mut impl Sink, link: &mut UnixStream) -> io::Result<()> {
    let mut down = BTreeSet::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    let result = loop {
        let n = match link.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => break Err(e),
        };
        pending.extend_from_slice(&buf[..n]);
        let used = pending.len() - pending.len() % Event::SIZE;
        let mut failed = None;
        for chunk in pending[..used].chunks(Event::SIZE) {
            let Some(event) = checked(Event::from_bytes(chunk.try_into().unwrap_or_default())) else { continue };
            if event.is_key() {
                if event.value == 1 {
                    down.insert(event.code);
                } else {
                    down.remove(&event.code);
                }
            }
            if let Err(e) = sink.emit(event) {
                failed = Some(e);
                break;
            }
        }
        pending.drain(..used);
        if let Some(e) = failed {
            break Err(e);
        }
    };
    for code in std::mem::take(&mut down) {
        let _ = sink.emit(Event::key(code, false));
    }
    let _ = sink.emit(Event::sync());
    result
}

/// An event from the other computer, if it is one worth putting on a device here.
fn checked(event: Event) -> Option<Event> {
    if !event.carried() {
        return None;
    }
    match event.kind {
        EV_KEY if event.code == 0 || event.code > KEY_MAX => None,
        EV_REL => Some(Event { value: event.value.clamp(-REL_LIMIT, REL_LIMIT), ..event }),
        _ => Some(event),
    }
}

/// A sender with no devices, for trying the whole path without permission: it presses the
/// hotkey, types an H and moves the pointer, then waits for the link to close.
pub struct Demo {
    script: std::vec::IntoIter<Vec<Event>>,
}

impl Demo {
    pub fn new() -> Self {
        let (lctrl, rctrl, h) = (29, 97, 35);
        let script = vec![
            vec![Event::key(lctrl, true), Event::key(rctrl, true), Event::sync()],
            vec![Event::key(lctrl, false), Event::key(rctrl, false), Event::sync()],
            vec![Event::key(h, true), Event::sync(), Event::key(h, false), Event::sync()],
            vec![Event { kind: EV_REL, code: crate::event::REL_X, value: 5 }, Event { kind: EV_REL, code: crate::event::REL_Y, value: -3 }, Event::sync()],
        ];
        Self { script: script.into_iter() }
    }
}

impl Default for Demo {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for Demo {
    fn poll(&mut self, link: RawFd, timeout_ms: i32) -> io::Result<(Vec<Event>, bool)> {
        let mut fd = libc::pollfd { fd: link, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd
        unsafe { libc::poll(&mut fd, 1, if self.script.len() > 0 { 100 } else { timeout_ms }) };
        Ok((self.script.next().unwrap_or_default(), fd.revents != 0))
    }

    fn grab(&self, _on: bool) -> io::Result<()> {
        Ok(())
    }
}

/// A receiver that prints what it gets, one event to a line.
pub struct Print<W: Write>(pub W);

impl<W: Write> Sink for Print<W> {
    fn emit(&mut self, e: Event) -> io::Result<()> {
        writeln!(self.0, "{} {} {}", e.kind, e.code, e.value)?;
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::{Chord, DEFAULT};
    use crate::event::REL_X;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Recorder(Rc<RefCell<Vec<Event>>>);

    impl Sink for Recorder {
        fn emit(&mut self, event: Event) -> io::Result<()> {
            self.0.borrow_mut().push(event);
            Ok(())
        }
    }

    fn router() -> Router {
        Router::new(Chord::parse(DEFAULT).unwrap())
    }

    #[test]
    fn what_the_sender_sends_the_receiver_puts_on_its_device() {
        let (mut sender_end, mut service_end) = UnixStream::pair().unwrap();
        let mut demo = Demo::new();
        let reader = std::thread::spawn(move || {
            let mut got = Vec::new();
            let _ = service_end.read_to_end(&mut got);
            got
        });
        let started = std::time::Instant::now();
        run_send(&mut demo, &mut sender_end, router(), || started.elapsed() < Duration::from_millis(900)).unwrap();
        drop(sender_end);
        let bytes = reader.join().unwrap();
        let events: Vec<Event> = bytes.chunks(Event::SIZE).map(|c| Event::from_bytes(c.try_into().unwrap())).collect();
        assert!(events.contains(&mark(true)), "the takeover was announced: {events:?}");
        assert!(events.contains(&Event::key(35, true)) && events.contains(&Event::key(35, false)));
        assert!(events.contains(&Event { kind: EV_REL, code: REL_X, value: 5 }));
        // The receiver turns the same bytes into the same events on its device
        let (mut a, mut b) = UnixStream::pair().unwrap();
        a.write_all(&bytes).unwrap();
        drop(a);
        let recorded = Recorder::default();
        let seen = recorded.0.clone();
        let mut sink = recorded;
        run_receive(&mut sink, &mut b).unwrap();
        let seen = seen.borrow();
        assert!(seen.contains(&Event::key(35, true)) && seen.contains(&Event { kind: EV_REL, code: REL_X, value: 5 }));
        assert!(seen.iter().all(|e| e.kind != EV_MARK), "messages for the service never reach the device");
    }

    #[test]
    fn a_key_held_when_the_link_drops_is_lifted_on_the_receiving_side() {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        a.write_all(&[Event::key(30, true).to_bytes(), Event::sync().to_bytes(), Event::key(42, true).to_bytes()].concat()).unwrap();
        drop(a);
        let recorded = Recorder::default();
        let seen = recorded.0.clone();
        let mut sink = recorded;
        run_receive(&mut sink, &mut b).unwrap();
        let seen = seen.borrow();
        assert!(seen.contains(&Event::key(30, false)) && seen.contains(&Event::key(42, false)));
        assert_eq!(seen.last(), Some(&Event::sync()));
    }

    #[test]
    fn nonsense_from_the_other_computer_stays_off_the_device() {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let wild = Event { kind: EV_REL, code: REL_X, value: 1_000_000 };
        let bytes = [Event::key(0, true), Event::key(0x7fff, true), Event { kind: 3, code: 0, value: 9 }, wild].map(Event::to_bytes).concat();
        a.write_all(&bytes).unwrap();
        a.write_all(&[1, 2, 3]).unwrap();
        drop(a);
        let recorded = Recorder::default();
        let seen = recorded.0.clone();
        let mut sink = recorded;
        run_receive(&mut sink, &mut b).unwrap();
        let seen = seen.borrow();
        assert_eq!(seen[0], Event { kind: EV_REL, code: REL_X, value: REL_LIMIT });
        assert_eq!(seen.len(), 2, "only the clamped step and the closing sync: {seen:?}");
    }

    #[test]
    fn the_sender_gives_everything_back_when_the_link_closes() {
        struct Pressed {
            script: std::vec::IntoIter<Vec<Event>>,
            grabbed: Rc<RefCell<Vec<bool>>>,
        }
        impl Source for Pressed {
            fn poll(&mut self, link: RawFd, timeout_ms: i32) -> io::Result<(Vec<Event>, bool)> {
                let mut fd = libc::pollfd { fd: link, events: libc::POLLIN, revents: 0 };
                // SAFETY: one valid pollfd
                unsafe { libc::poll(&mut fd, 1, if self.script.len() > 0 { 20 } else { timeout_ms }) };
                Ok((self.script.next().unwrap_or_default(), fd.revents != 0))
            }
            fn grab(&self, on: bool) -> io::Result<()> {
                self.grabbed.borrow_mut().push(on);
                Ok(())
            }
        }
        let grabbed = Rc::new(RefCell::new(Vec::new()));
        let mut source = Pressed {
            script: vec![vec![Event::key(29, true), Event::key(97, true)], vec![Event::key(29, false), Event::key(97, false)], vec![Event::key(30, true), Event::sync()]].into_iter(),
            grabbed: grabbed.clone(),
        };
        let (mut a, b) = UnixStream::pair().unwrap();
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            drop(b);
        });
        run_send(&mut source, &mut a, router(), || true).unwrap();
        closer.join().unwrap();
        assert_eq!(*grabbed.borrow(), vec![true, false], "grabbed on the hotkey, given back when the link closed");
    }
}
