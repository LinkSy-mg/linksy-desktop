use std::io;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use linksy_keys::chord::{self, Chord};
use linksy_keys::evdev::{self, Capture};
use linksy_keys::router::{Action, Router, State};
use linksy_keys::uinput::VirtualDevice;

const USAGE: &str = "Usage: linksy-keys devices
       linksy-keys loopback [--hotkey KEYS] [--seconds N]

devices   lists the keyboards and mice that can be shared.
loopback  a test on this computer alone: the hotkey (both Ctrl keys by default) takes
          your keyboard and mouse over and hands them back through a virtual keyboard and
          mouse. It ends by itself after N seconds (60 by default, at most 600).

Both need permission to read /dev/input and to write /dev/uinput: run them with sudo.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("devices") => devices(),
        Some("loopback") => loopback(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("linksy-keys: {e}");
            ExitCode::FAILURE
        }
    }
}

fn describe(e: io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => format!("{e}: no permission to read the keyboard and mouse here (run it with sudo)"),
        _ => e.to_string(),
    }
}

fn devices() -> Result<(), String> {
    for d in evdev::list().map_err(describe)? {
        println!("{:<8} {}  {}", format!("{:?}", d.kind).to_lowercase(), d.path.display(), d.name);
    }
    Ok(())
}

fn loopback(args: &[String]) -> Result<(), String> {
    let (mut hotkey, mut seconds) = (chord::DEFAULT.to_string(), 60u64);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--hotkey" => hotkey = value()?,
            "--seconds" => seconds = value()?.parse::<u64>().map_err(|_| "--seconds needs a number")?.clamp(1, 600),
            other => return Err(format!("unknown option {other}\n\n{USAGE}")),
        }
    }
    let mut router = Router::new(Chord::parse(&hotkey)?);
    let devices = evdev::list().map_err(describe)?;
    if devices.is_empty() {
        return Err("no keyboard or mouse found".into());
    }
    for d in &devices {
        eprintln!("sharing {} ({})", d.name, d.path.display());
    }
    let mut capture = Capture::new(devices);
    let mut virtual_device = VirtualDevice::create().map_err(|e| format!("can't make the virtual keyboard and mouse: {}", describe(e)))?;
    eprintln!("Press {hotkey} to switch. It ends by itself in {seconds} s.");

    let end = Instant::now() + Duration::from_secs(seconds);
    let mut sent = 0u64;
    let mut state = router.state();
    while Instant::now() < end && !capture.devices().is_empty() {
        for event in capture.poll(200).map_err(|e| e.to_string())? {
            for action in router.on_event(event) {
                apply(action, &capture, &mut virtual_device, &mut sent).map_err(|e| e.to_string())?;
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
