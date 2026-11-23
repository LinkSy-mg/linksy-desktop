use std::io;
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use linksy_handover::chord::{self, Chord};
use linksy_handover::evdev::{self, Capture};
use linksy_handover::link::{self, Demo, Print};
use linksy_handover::router::{Action, Router, State};
use linksy_handover::uinput::VirtualDevice;

const USAGE: &str = "Usage: linksy-handover send --socket PATH [--hotkey KEYS]
       linksy-handover receive --socket PATH
       linksy-handover devices
       linksy-handover loopback [--hotkey KEYS] [--seconds N]

send      watches this computer's keyboard and mouse and, while the hotkey has handed
          them over, sends them to the service on PATH (both Ctrl keys by default).
receive   makes a virtual keyboard and mouse from what the service on PATH sends.
devices   lists the keyboards and mice that can be handed over.
loopback  a test on this computer alone: the hotkey hands the devices to a virtual
          keyboard and mouse here, and back. It ends by itself after N seconds (60 by
          default, at most 600).

Reading /dev/input and writing /dev/uinput need permission: without it these exit with 77.
--demo (send) and --print (receive) try the link without any device.";

/// The exit status for "no permission", as sysexits names it.
const NO_PERMISSION: u8 = 77;

struct Fail(u8, String);

impl From<String> for Fail {
    fn from(message: String) -> Self {
        Fail(1, message)
    }
}

fn describe(what: &str, e: io::Error) -> Fail {
    match e.kind() {
        io::ErrorKind::PermissionDenied => Fail(NO_PERMISSION, format!("{what}: no permission ({e})")),
        _ => Fail(1, format!("{what}: {e}")),
    }
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn stop_on_signals() {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: the handler only stores to an atomic
        unsafe { libc::signal(signal, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    }
}

fn running() -> bool {
    !STOP.load(Ordering::Relaxed)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("send") => send(&args[1..]),
        Some("receive") => receive(&args[1..]),
        Some("devices") => devices(),
        Some("loopback") => loopback(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail(code, message)) => {
            eprintln!("linksy-handover: {message}");
            ExitCode::from(code)
        }
    }
}

#[derive(Default)]
struct Options {
    socket: Option<String>,
    hotkey: Option<String>,
    seconds: Option<u64>,
    demo: bool,
    print: bool,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut out = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--socket" => out.socket = Some(value()?),
            "--hotkey" => out.hotkey = Some(value()?),
            "--seconds" => out.seconds = Some(value()?.parse::<u64>().map_err(|_| "--seconds needs a number")?.clamp(1, 600)),
            "--demo" => out.demo = true,
            "--print" => out.print = true,
            other => return Err(format!("unknown option {other}\n\n{USAGE}")),
        }
    }
    Ok(out)
}

fn connect(opts: &Options) -> Result<UnixStream, Fail> {
    let path = opts.socket.as_deref().ok_or_else(|| String::from("--socket is needed"))?;
    UnixStream::connect(path).map_err(|e| describe(path, e))
}

fn router(opts: &Options) -> Result<Router, String> {
    Ok(Router::new(Chord::parse(opts.hotkey.as_deref().unwrap_or(chord::DEFAULT))?))
}

fn send(args: &[String]) -> Result<(), Fail> {
    let opts = options(args)?;
    let router = router(&opts)?;
    if opts.demo {
        let mut link = connect(&opts)?;
        stop_on_signals();
        return link::run_send(&mut Demo::new(), &mut link, router, running).map_err(|e| describe("the link", e));
    }
    let devices = evdev::list().map_err(|e| describe("the keyboard and mouse", e))?;
    if devices.is_empty() {
        return Err(Fail(1, "no keyboard or mouse found".into()));
    }
    let mut link = connect(&opts)?;
    stop_on_signals();
    let mut capture = Capture::new(devices);
    link::run_send(&mut capture, &mut link, router, running).map_err(|e| describe("the keyboard and mouse", e))
}

fn receive(args: &[String]) -> Result<(), Fail> {
    let opts = options(args)?;
    if opts.print {
        let mut link = connect(&opts)?;
        stop_on_signals();
        return link::run_receive(&mut Print(io::stdout()), &mut link).map_err(|e| describe("the link", e));
    }
    let mut device = VirtualDevice::create().map_err(|e| describe("the virtual keyboard and mouse", e))?;
    let mut link = connect(&opts)?;
    stop_on_signals();
    link::run_receive(&mut device, &mut link).map_err(|e| describe("the virtual keyboard and mouse", e))
}

fn devices() -> Result<(), Fail> {
    for d in evdev::list().map_err(|e| describe("the keyboard and mouse", e))? {
        println!("{:<8} {}  {}", format!("{:?}", d.kind).to_lowercase(), d.path.display(), d.name);
    }
    Ok(())
}

fn loopback(args: &[String]) -> Result<(), Fail> {
    let opts = options(args)?;
    let hotkey = opts.hotkey.clone().unwrap_or_else(|| chord::DEFAULT.to_string());
    let seconds = opts.seconds.unwrap_or(60);
    let mut router = router(&opts)?;
    let devices = evdev::list().map_err(|e| describe("the keyboard and mouse", e))?;
    if devices.is_empty() {
        return Err(Fail(1, "no keyboard or mouse found".into()));
    }
    for d in &devices {
        eprintln!("sharing {} ({})", d.name, d.path.display());
    }
    let mut capture = Capture::new(devices);
    let mut virtual_device = VirtualDevice::create().map_err(|e| describe("the virtual keyboard and mouse", e))?;
    stop_on_signals();
    eprintln!("Press {hotkey} to switch. It ends by itself in {seconds} s.");

    let end = Instant::now() + Duration::from_secs(seconds);
    let mut sent = 0u64;
    let mut state = router.state();
    while Instant::now() < end && running() && !capture.devices().is_empty() {
        for event in capture.poll(200).map_err(|e| describe("the keyboard and mouse", e))? {
            for action in router.on_event(event) {
                apply(action, &capture, &mut virtual_device, &mut sent).map_err(|e| describe("the devices", e))?;
            }
            if router.state() != state {
                state = router.state();
                eprintln!("{}", if state == State::There { "now on the virtual keyboard and mouse" } else if state == State::Here { "back on the real ones" } else { "waiting for every key to be up" });
            }
        }
    }
    for action in router.abort() {
        let _ = apply(action, &capture, &mut virtual_device, &mut sent);
    }
    eprintln!("done, {sent} events went through the virtual device");
    Ok(())
}

fn apply(action: Action, capture: &Capture, virtual_device: &mut VirtualDevice, sent: &mut u64) -> io::Result<()> {
    match action {
        Action::Grab => capture.grab(true),
        Action::Ungrab => capture.grab(false),
        Action::Send(event) => {
            *sent += 1;
            virtual_device.emit(event)
        }
    }
}
