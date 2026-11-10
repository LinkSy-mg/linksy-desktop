//! `linksy-screen`: the media side of Linksy sessions, started by `linksyd` only while a
//! session runs, so the always-on service never loads GStreamer.
//!
//! `linksy-screen share --socket PATH [--token-file PATH] [--control] [--virtual [--size WxH]] [--test]`
//!
//! Asks the desktop for a screen through the ScreenCast portal (the user picks a screen
//! the first time; the choice is remembered with a restore token), encodes it to H.264
//! and writes the Annex-B byte stream to the Unix socket `linksyd` listens on, which
//! relays it to the viewer. With `--control` it goes through the RemoteDesktop portal
//! instead, and applies the pointer and keyboard input the viewer sends back on the same
//! socket. `--virtual` shares a new virtual monitor of `--size` (default 1920x1080)
//! instead of an existing one: a second display for the device showing it. `--test`
//! streams a moving test pattern instead of the screen and logs input rather than
//! applying it, for development and tests.
//!
//! On X11, where no portal works, it reads the display itself and drives it with XTest. On
//! Hyprland and Sway, whose portals can't drive a screen, it shares one through ScreenCast
//! and applies the input with their virtual pointer and keyboard; `--output NAME` says which
//! output is shared (a headless one made for a second display).
//!
//! `linksy-screen share --camera --socket PATH [--test]` shares this device's camera
//! instead of a screen.
//!
//! `linksy-screen webcam --socket PATH [--name NAME] [--fps 30|60]` does the opposite end of a camera
//! session: decodes the stream read from the socket and offers it to every app as a
//! PipeWire camera called NAME (browsers, Meet, GNOME Snapshot, OBS…).
//!
//! `linksy-screen speaker --socket PATH [--test]` plays the sound a phone shares with its screen
//! (raw 16-bit 48 kHz stereo PCM) on this computer's speakers; `--test` plays nothing.
//!
//! `linksy-screen share --mic --socket PATH [--test]` shares this device's microphone, and
//! `linksy-screen mic --socket PATH [--name NAME]` offers a received one to every app as
//! a microphone called NAME. Audio crosses as raw 16-bit 48 kHz mono PCM (docs/protocol.md
//! §4.8): about 100 KB/s, and nothing to decode.

mod mutter;
mod outputs;
mod portal;
mod virtual_mic;
mod wlr;
mod x11;

use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use gstreamer as gst;
use gstreamer::prelude::*;
use linksy_core::protocol::read_control_frames;

struct Args {
    /// `webcam` mode: publish the incoming stream as a camera with this name.
    webcam: Option<String>,
    /// `webcam` mode: the frame rate the stream comes at (30 or 60).
    fps: u32,
    /// `mic` mode: publish the incoming audio as a microphone with this name.
    microphone: Option<String>,
    /// The video bitrate in kbit/s (16000 on a local network or cable, less over the internet).
    bitrate: u32,
    /// The most frames a second (60; 30 over the internet).
    max_fps: u32,
    /// `speaker` mode: play the incoming sound.
    speaker: bool,
    camera: bool,
    /// Share this device's microphone.
    mic: bool,
    socket: PathBuf,
    token_file: Option<PathBuf>,
    control: bool,
    /// Pointer and keyboard only, no screen (`input` sessions).
    input_only: bool,
    virtual_size: Option<(u32, u32)>,
    /// The output shared, by name, where the input goes (Hyprland, Sway).
    output: Option<String>,
    test: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut speaker = false;
    let (mut webcam, mut microphone) = match args.next().as_deref() {
        Some("share") => (None, None),
        Some("speaker") => {
            speaker = true;
            (None, None)
        }
        Some("webcam") => (Some("Linksy Camera".to_string()), None),
        Some("mic") => (None, Some("Linksy Microphone".to_string())),
        _ => {
            return Err("usage: linksy-screen share --socket PATH [--token-file PATH] [--camera | --mic | --control | --input-only] [--virtual [--size WxH]] [--output NAME] [--bitrate KBPS] [--max-fps 30|60] [--test]\n       \
                        linksy-screen webcam --socket PATH [--name NAME] [--fps 30|60]\n       \
                        linksy-screen mic --socket PATH [--name NAME]\n       \
                        linksy-screen speaker --socket PATH [--test]"
                .into());
        }
    };
    let (mut socket, mut token_file, mut control, mut test, mut camera, mut mic) = (None, None, false, false, false, false);
    let (mut input_only, mut output) = (false, None);
    let (mut bitrate, mut max_fps) = (16_000u32, MAX_FPS);
    let mut fps = 30;
    let (mut is_virtual, mut size) = (false, (1920, 1080));
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket = args.next().map(PathBuf::from),
            "--token-file" => token_file = args.next().map(PathBuf::from),
            "--control" => control = true,
            "--input-only" => input_only = true,
            "--virtual" => is_virtual = true,
            "--size" => {
                let v = args.next().unwrap_or_default();
                size = v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).ok_or("--size takes WxH")?;
            }
            "--output" => output = args.next(),
            "--test" => test = true,
            "--bitrate" => bitrate = args.next().and_then(|v| v.parse().ok()).filter(|k| (500..=60_000).contains(k)).ok_or("--bitrate takes kbit/s, 500 to 60000")?,
            "--max-fps" => max_fps = args.next().and_then(|v| v.parse().ok()).filter(|f| matches!(f, 30 | 60)).ok_or("--max-fps takes 30 or 60")?,
            "--camera" => camera = true,
            "--mic" => mic = true,
            "--fps" => fps = args.next().and_then(|v| v.parse().ok()).filter(|f| matches!(f, 30 | 60)).ok_or("--fps takes 30 or 60")?,
            "--name" if webcam.is_some() => webcam = args.next(),
            "--name" if microphone.is_some() => microphone = args.next(),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Args { webcam, fps, microphone, bitrate, max_fps, speaker, camera, mic, socket: socket.ok_or("--socket is required")?, token_file, control, input_only, virtual_size: is_virtual.then_some(size), output, test })
}

fn main() -> ExitCode {
    linksy_logs::catch_crashes("linksy-screen", env!("CARGO_PKG_VERSION"));
    // Ends with the service that started it, even if that one crashed or was killed: never
    // left behind holding a screen, a camera or memory
    #[cfg(target_os = "linux")]
    // SAFETY: a plain prctl on this process
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("linksy-screen: {e}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("linksy-screen: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    gst::init().map_err(|e| e.to_string())?;
    if let Some(name) = &args.webcam {
        return webcam(&args.socket, name, args.fps);
    }
    if let Some(name) = &args.microphone {
        return microphone(&args.socket, name);
    }
    if args.speaker {
        return speaker(&args.socket, args.test);
    }
    if args.mic {
        return share_mic(&args.socket, args.test);
    }
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().map_err(|e| e.to_string())?;
    if args.input_only {
        return input_only(&args, &rt);
    }

    // What to capture, and where input goes (nowhere for view-only sessions).
    let (source, input) = if args.camera {
        // 720p at 30 fps: what video calls use, and light on the link.
        let camera = if args.test { "videotestsrc is-live=true pattern=smpte" } else { "v4l2src ! decodebin" };
        let source = format!("{camera} ! videoconvert ! videoscale ! videorate ! video/x-raw,width=1280,height=720,framerate=30/1");
        (source, None)
    } else if args.test {
        // LINKSY_SCREEN_TEST_SIZE=720x1280 imitates a phone held upright.
        let (w, h) = std::env::var("LINKSY_SCREEN_TEST_SIZE")
            .ok()
            .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))))
            .unwrap_or((1280, 720));
        // LINKSY_SCREEN_TEST_FPS=60 imitates a screen that moves 60 times a second.
        let fps: u32 = std::env::var("LINKSY_SCREEN_TEST_FPS").ok().and_then(|f| f.parse().ok()).unwrap_or(30);
        let pattern = format!("videotestsrc is-live=true pattern=ball motion=sweep background-color=0xff2563ff ! video/x-raw,width={w},height={h},framerate={fps}/1");
        (pattern, args.control.then_some(portal::Input::Log))
    } else {
        open_capture(&args, &rt)?
    };
    // linksyd listens on the socket and relays everything written here to the viewer, and
    // the viewer's control messages back.
    let out = UnixStream::connect(&args.socket).map_err(|e| format!("{}: {e}", args.socket.display()))?;
    let control_in = out.try_clone().map_err(|e| e.to_string())?;
    let handle = rt.handle().clone();
    // Shared: a second display that stalls gets a new capture, and its input with it
    let input = std::sync::Arc::new(std::sync::Mutex::new(input));
    let shared = input.clone();
    std::thread::spawn(move || {
        // View-only sessions read and ignore input.
        read_control_frames(control_in, |msg| {
            if let Some(input) = shared.lock().unwrap().as_ref() {
                handle.block_on(input.apply(&msg));
            }
        });
    });

    // The screen through the desktop (PipeWire), not a camera or a test pattern
    let screen = source.starts_with("pipewiresrc");
    // A second display in the phone's shape: GNOME makes its virtual monitor the size asked
    // for here; KDE makes it 1920x1080 or the size chosen in its dialog, which the phone then
    // fits to its screen.
    let mut size = args.virtual_size.map(|(w, h)| format!(",width={w},height={h}")).unwrap_or_default();
    let vapostproc = gst::ElementFactory::find("vapostproc").is_some();
    // What to try, best first, each until one starts: with a GPU encoder, frames stay on
    // the GPU (the desktop hands them over in graphics memory, DMA-BUF, and the GPU converts
    // and encodes them, up to 60 a second with almost no CPU), else they're uploaded (about
    // 30 a second for a phone-shaped display). On the CPU, a big screen (2.5 million pixels)
    // can take longer to encode than a frame lasts: at most 30 a second there, converted on
    // several threads. A GPU encoder that's listed but doesn't work (a sandbox's drivers, an
    // unusual card) falls through to the next.
    let mut attempts: Vec<(String, &str, Memory)> = Vec::new();
    for (encoder, format) in encoders(args.bitrate) {
        let gpu = encoder.starts_with("va") && vapostproc;
        if gpu && screen {
            attempts.push((encoder.clone(), format, Memory::Gpu));
        }
        attempts.push((encoder, format, Memory::System));
    }
    // For Save Logs: what this computer offers, in the order tried
    let tried: Vec<String> = attempts.iter().map(|(e, _, m)| format!("{}{}", e.split_whitespace().next().unwrap_or(""), if *m == Memory::Gpu { " (graphics memory)" } else { "" })).collect();
    eprintln!("linksy-screen: encoders, in order: {}", if tried.is_empty() { "none".to_string() } else { tried.join(", ") });
    let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4));
    let max_fps = args.max_fps;
    let chain = |(encoder, format, memory): &(String, &str, Memory), size: &str| -> String {
        let (format, memory) = (*format, *memory);
        let gpu = encoder.starts_with("va") && vapostproc;
        // Copied: the desktop's few buffers go back to it at once
        let copy = if screen && memory == Memory::System { " always-copy=true" } else { "" };
        let caps = match memory {
            // Caps with a memory feature, quoted for the parser
            Memory::Gpu => format!("capsfilter caps=\"video/x-raw(memory:DMABuf){size}\""),
            Memory::System if !size.is_empty() => format!("video/x-raw{size}"),
            // Whatever the source gives (a bare "video/x-raw" isn't caps to the parser)
            Memory::System => "identity".to_string(),
        };
        // Up to 60 frames a second, however fast the screen is (144 Hz ones too): dropped here
        // where it costs nothing, and brought down to 30 by `adapt_rate` when the encoder can't
        // keep up
        let rate = format!("videorate name=rate drop-only=true max-rate={max_fps}");
        let convert = if gpu {
            format!("{rate} ! vapostproc ! capsfilter caps=\"video/x-raw(memory:VAMemory),format=NV12\"")
        } else {
            format!("{rate} ! videoconvert n-threads={threads} ! video/x-raw,format={format}")
        };
        format!("{copy} ! {caps} ! queue name=captured max-size-buffers=1 leaky=downstream ! {convert} ! queue max-size-buffers=2 leaky=downstream ! {encoder} name=encoder")
    };
    // GNOME can stop feeding a virtual monitor's stream when its layout changes (display
    // settings applied): no frame comes, not even the keepalive. Then the capture starts
    // again in the same session, the old one closed first.
    let restartable = args.virtual_size.is_some() && !args.test;
    let mut source = source;
    let mut restarts = 0;
    let mut attempt = 0;
    let result = loop {
        let Some(current) = attempts.get(attempt).cloned() else { break Err("no H.264 encoder here works (install openh264 or x264)".to_string()) };
        let (encoder, _, memory) = &current;
        let memory = *memory;
        let name = encoder.split_whitespace().next().unwrap_or("encoder").to_string();
        let name = name.as_str();
        let description = format!(
            "{source}{} ! video/x-h264,profile=constrained-baseline ! h264parse config-interval=-1 \
             ! video/x-h264,stream-format=byte-stream,alignment=au \
             ! fdsink fd={} sync=false",
            chain(&current, &size),
            out.as_raw_fd()
        );
        let pipeline = match gst::parse::launch(&description) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("linksy-screen: {name}{}: {e}", if memory == Memory::Gpu { " from graphics memory" } else { "" });
                attempt += 1;
                continue;
            }
        };
        let last_frame = count_frames(&pipeline, name);
        adapt_rate(&pipeline, max_fps);
        let outcome = match pipeline.set_state(gst::State::Playing) {
            // Why, from the element that refused (set_state only says that one did)
            Err(_) => Some(Ended(Err(bus_error(&pipeline).unwrap_or_else(|| "an element failed to start".into())))),
            Ok(_) => watch_bus(&pipeline, restartable.then_some(&last_frame)),
        };
        let _ = pipeline.set_state(gst::State::Null);
        drop(pipeline);
        let started = last_frame.load(std::sync::atomic::Ordering::Relaxed) > 0;
        match outcome {
            // The desktop keeps the second display at its own size (KDE): that size, the
            // same way again
            Some(Ended(Err(e))) if !started && !size.is_empty() && e.contains("no more input formats") => {
                eprintln!("linksy-screen: the desktop didn't make the second display in the phone's shape; using its own size");
                size.clear();
            }
            // Didn't work before the first frame: the next way
            Some(Ended(Err(e))) if !started && attempt + 1 < attempts.len() => {
                let how = if memory == Memory::Gpu { " from graphics memory" } else { "" };
                eprintln!("linksy-screen: {name}{how} didn't start ({e}); trying the next way");
                attempt += 1;
            }
            Some(Stalled) if restarts < 5 => {
                restarts += 1;
                eprintln!("linksy-screen: the second display's stream stopped, starting it again once GNOME's display settings are kept");
                rt.block_on(mutter::settled(std::time::Duration::from_secs(22)));
                if let Some(old) = input.lock().unwrap().take() {
                    rt.block_on(old.close());
                }
                let (new_source, new_input) = open_capture(&args, &rt)?;
                source = new_source;
                *input.lock().unwrap() = new_input;
            }
            Some(Stalled) => break Err("the second display's stream kept stopping".to_string()),
            None => break Ok(()),
            Some(Ended(r)) => break r,
        }
    };
    if let Some(old) = input.lock().unwrap().take() {
        rt.block_on(old.close());
    }
    drop(out);
    result
}

use Outcome::{Ended, Stalled};

/// Where frames come in: graphics memory, straight to the GPU encoder, or normal memory.
#[derive(Clone, Copy, PartialEq)]
enum Memory {
    Gpu,
    System,
}

enum Outcome {
    Ended(Result<(), String>),
    /// No frame for a while from a source that sends one every second at least.
    Stalled,
}

/// The error an element posted, with its name and its own details.
fn bus_error(pipeline: &gst::Element) -> Option<String> {
    let bus = pipeline.bus()?;
    let msg = bus.timed_pop_filtered(gst::ClockTime::from_mseconds(200), &[gst::MessageType::Error])?;
    let gst::MessageView::Error(e) = msg.view() else { return None };
    let element = e.src().map(|s| s.name().to_string()).unwrap_or_default();
    Some(match e.debug() {
        Some(debug) => format!("{element}: {} ({debug})", e.error()),
        None => format!("{element}: {}", e.error()),
    })
}

/// Run until the pipeline ends; with `last_frame`, also until frames stop coming.
fn watch_bus(pipeline: &gst::Element, last_frame: Option<&std::sync::Arc<std::sync::atomic::AtomicU64>>) -> Option<Outcome> {
    let bus = pipeline.bus()?;
    loop {
        let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(500)) else {
            // The keepalive sends the last frame 10 times a second: 3 s without any is a stall
            if let Some(last) = last_frame {
                let ms = last.load(std::sync::atomic::Ordering::Relaxed);
                if ms > 0 && now_ms() > ms + 3000 {
                    return Some(Stalled);
                }
            }
            continue;
        };
        match msg.view() {
            gst::MessageView::Eos(_) => return None,
            // The viewer went away (socket closed): a write error, and a normal end.
            gst::MessageView::Error(e) if e.src().is_some_and(|s| s.name().starts_with("fdsink")) => return None,
            gst::MessageView::Error(e) => {
                let element = e.src().map(|s| s.name().to_string()).unwrap_or_default();
                return Some(Ended(Err(format!("{element}: {} ({})", e.error(), e.debug().map(|d| d.to_string()).unwrap_or_default()))));
            }
            _ => {}
        }
    }
}

/// Ask the desktop for the screen (a new virtual monitor for a second display): the
/// pipeline's source, where input goes, and the PipeWire remote to close with it.
fn open_capture(args: &Args, rt: &tokio::runtime::Runtime) -> Result<(String, Option<portal::Input>), String> {
    if x11::session() {
        if args.virtual_size.is_some() {
            return Err("a second display needs a Wayland session".into());
        }
        let (source, control) = x11::capture(args.control)?;
        return Ok((source, control.map(|c| portal::Input::X11(Box::new(c)))));
    }
    // Hyprland, Sway: the portal shares the screen, the compositor takes the input
    let virtual_input = if args.control && args.virtual_size.is_none() { wlr::desktop() } else { None };
    if args.control && args.output.is_some() && virtual_input.is_none() {
        eprintln!("linksy-screen: this desktop takes no input from other devices; the second display only shows");
    }
    let remote = args.control && virtual_input.is_none() && args.output.is_none();
    let token = args.token_file.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let had_token = token.is_some();
    let screen = match rt.block_on(portal::screen(token, remote, args.virtual_size.is_some())) {
        Ok(screen) => screen,
        // A remembered choice can go stale (its monitor gone, say) and then restore no
        // screen at all, every time: forget it and show the dialog.
        Err(e) if had_token && !e.ends_with("cancelled") => {
            eprintln!("linksy-screen: the remembered screen didn't restore ({e}), asking again");
            rt.block_on(portal::screen(None, remote, args.virtual_size.is_some()))?
        }
        Err(e) => return Err(e),
    };
    // No token back (input refused, say): forget the old one so the dialog asks again.
    if let Some(path) = &args.token_file {
        let _ = match &screen.restore_token {
            Some(new) => std::fs::write(path, new),
            None if remote => std::fs::remove_file(path),
            None => Ok(()),
        };
    }
    let input = match virtual_input {
        Some(desktop) => Some(portal::Input::Wlr(Box::new(desktop.control(&wlr::Want { name: args.output.clone(), position: screen.position, size: screen.size })?))),
        None => screen.input,
    };
    // The PipeWire remote stays open for the life of the pipeline (pipewiresrc closes it).
    // The memory the frames come in, and a virtual monitor's size, are chosen by the chain
    // The last frame again every 100 ms while the screen is still: encoders hold one back until
    // the next comes, which left a tap's cursor a second late
    let source = format!("pipewiresrc fd={} path={} do-timestamp=true keepalive-time=100", screen.fd.as_raw_fd(), screen.node);
    std::mem::forget(screen.fd);
    Ok((source, input))
}

/// The most frames a second a shared screen is encoded at; 30 is what it falls back to.
const MAX_FPS: u32 = 60;
const SLOW_FPS: u32 = 30;

/// Encode at 60 where this computer can, and at 30 where it can't, by itself. Every two seconds
/// it looks at how many of the frames the screen gave were encoded: when a screen that moves
/// (more than 45 a second) is encoded at under 86 % of the rate wanted twice running, the rate limit goes
/// down to 30 for good: an even 30 looks better than a 60 that stutters.
fn adapt_rate(pipeline: &gst::Element, max_fps: u32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Already at the slow rate
    if max_fps <= SLOW_FPS {
        return;
    }
    let Some(bin) = pipeline.downcast_ref::<gst::Bin>() else { return };
    let Some(rate) = bin.by_name("rate") else { return };
    let counts = std::sync::Arc::new([AtomicU64::new(0), AtomicU64::new(0)]);
    // What the screen gives, and what comes out encoded
    for (name, pad, i) in [("captured", "sink", 0), ("encoder", "src", 1)] {
        if let Some(pad) = bin.by_name(name).and_then(|e| e.static_pad(pad)) {
            let counts = counts.clone();
            pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                counts[i].fetch_add(1, Ordering::Relaxed);
                gst::PadProbeReturn::Ok
            });
        }
    }
    let (counts, rate) = (std::sync::Arc::downgrade(&counts), rate.downgrade());
    std::thread::spawn(move || {
        let mut behind = 0;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let (Some(counts), Some(rate)) = (counts.upgrade(), rate.upgrade()) else { return };
            let (shown, encoded) = (counts[0].swap(0, Ordering::Relaxed) as f64 / 2.0, counts[1].swap(0, Ordering::Relaxed) as f64 / 2.0);
            let wanted = shown.min(MAX_FPS as f64);
            behind = if falls_behind(shown, encoded) { behind + 1 } else { 0 };
            if behind >= 2 {
                eprintln!("linksy-screen: encoding {encoded:.0} of {wanted:.0} frames a second; keeping to {SLOW_FPS}");
                rate.set_property("max-rate", SLOW_FPS as i32);
                return;
            }
        }
    });
}

/// Whether an encoder is too slow for 60 frames a second: the screen moves more than 45 times a
/// second, and under 86 % of the frames wanted (the screen's, up to the limit) come out. A screen
/// faster than the limit is meant to lose frames; the rest is the encoder's.
fn falls_behind(shown: f64, encoded: f64) -> bool {
    shown > 45.0 && encoded < shown.min(MAX_FPS as f64) * 0.86
}

/// Every 10 seconds, how many frames the screen gave and how many were encoded (the
/// rest were dropped because the encoder was behind), for Save Logs. Returns when the last
/// frame came (milliseconds since the epoch; 0 before the first). The report ends with the
/// pipeline.
fn count_frames(pipeline: &gst::Element, encoder: &str) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    let last = Arc::new(AtomicU64::new(0));
    let Some(bin) = pipeline.downcast_ref::<gst::Bin>() else { return last };
    // [captured, encoded]: held by the probes, so they go when the pipeline does
    let counts = Arc::new([AtomicU64::new(0), AtomicU64::new(0)]);
    for (name, pad, i) in [("captured", "sink", 0), ("encoder", "src", 1)] {
        if let Some(pad) = bin.by_name(name).and_then(|e| e.static_pad(pad)) {
            let (counts, last) = (counts.clone(), last.clone());
            pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                counts[i].fetch_add(1, Ordering::Relaxed);
                if i == 0 {
                    last.store(now_ms(), Ordering::Relaxed);
                }
                gst::PadProbeReturn::Ok
            });
        }
    }
    let (counts, encoder) = (Arc::downgrade(&counts), encoder.to_string());
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(10));
            let Some(counts) = counts.upgrade() else { return };
            let (c, e) = (counts[0].swap(0, Ordering::Relaxed), counts[1].swap(0, Ordering::Relaxed));
            if c == 0 && e == 0 {
                continue;
            }
            eprintln!("linksy-screen: {:.1} fps from the screen, {:.1} encoded ({encoder}), {} dropped", c as f64 / 10.0, e as f64 / 10.0, c.saturating_sub(e));
        }
    });
    last
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// A phone as touchpad and keyboard: apply its input until it stops. Nothing is shared.
fn input_only(args: &Args, rt: &tokio::runtime::Runtime) -> Result<(), String> {
    let input = if args.test {
        portal::Input::Log
    } else if x11::session() {
        portal::Input::X11(Box::new(x11::Control::open()?))
    } else if let Some(desktop) = wlr::desktop() {
        portal::Input::Wlr(Box::new(desktop.control(&wlr::Want::default())?))
    } else {
        let token = args.token_file.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let (input, new_token) = rt.block_on(portal::input_only(token))?;
        if let Some(path) = &args.token_file {
            let _ = match &new_token {
                Some(new) => std::fs::write(path, new),
                None => std::fs::remove_file(path),
            };
        }
        input
    };
    let stream = UnixStream::connect(&args.socket).map_err(|e| format!("{}: {e}", args.socket.display()))?;
    read_control_frames(stream, |msg| rt.block_on(input.apply(&msg)));
    // The phone went away: nothing stays pressed here
    rt.block_on(input.close());
    Ok(())
}

/// The best H.264 encoder available, and the raw format it wants: the GPU first, then
/// x264, then OpenH264 (always present on Fedora). All are asked for Constrained
/// Baseline, which every H.264 decoder handles (OpenH264's, phones' hardware ones).
fn encoders(kbps: u32) -> Vec<(String, &'static str)> {
    // LINKSY_SCREEN_ENCODERS=x264enc,openh264enc allows only those (to try the CPU ones where
    // there is a GPU, say).
    let only: Option<Vec<String>> = std::env::var("LINKSY_SCREEN_ENCODERS").ok().map(|l| l.split(',').map(|e| e.trim().to_string()).collect());
    let has = |name: &str| gst::ElementFactory::find(name).is_some() && only.as_ref().is_none_or(|o| o.iter().any(|e| e == name));
    let mut list = Vec::new();
    if has("vah264lpenc") && constant_bitrate("vah264lpenc") {
        list.push((format!("vah264lpenc rate-control=cbr bitrate={kbps} key-int-max=60 target-usage=7"), "NV12"));
    }
    if has("vah264enc") && constant_bitrate("vah264enc") {
        list.push((format!("vah264enc rate-control=cbr bitrate={kbps} key-int-max=60 target-usage=7"), "NV12"));
    }
    if has("x264enc") {
        // One slice per frame: zerolatency would cut each frame into a slice per thread,
        // which decoders fed a NAL unit at a time (phones) never assemble into a picture.
        // Two threads work on two frames at once instead: one frame later, nearly twice as fast.
        // ultrafast: a sharp desktop with windows moving on it cost superfast 10 times more
        // live than a video does (half-pixel motion search), and fell to 4 fps on a laptop on
        // battery; 16 Mbit/s keeps it sharp, which a local network carries easily.
        list.push((format!("x264enc tune=zerolatency speed-preset=ultrafast bitrate={kbps} key-int-max=60 bframes=0 sliced-threads=false threads=2"), "I420"));
    }
    if has("openh264enc") {
        // OpenH264 counts in bit/s, and is held to half what the others get: it is the CPU's last resort
        list.push((format!("openh264enc usage-type=screen complexity=low rate-control=bitrate bitrate={} gop-size=60", u64::from(kbps) * 500), "I420"));
    }
    list
}

/// Whether a VA encoder can keep a constant bitrate. Its rate controls are the GPU's: the
/// low-power one on Kaby Lake offers only constant QP, and the pipeline then never starts.
fn constant_bitrate(name: &str) -> bool {
    let Ok(encoder) = gst::ElementFactory::make(name).build() else { return false };
    encoder
        .find_property("rate-control")
        .and_then(|p| p.downcast::<gst::glib::ParamSpecEnum>().ok())
        .is_some_and(|p| p.enum_class().value_by_nick("cbr").is_some())
}

/// A decoder that hands each frame on as soon as it's decoded. libav's default frame
/// threading holds one frame per CPU thread first: about 400 ms on a 12-thread laptop,
/// against 40 ms with slice threading.
fn low_latency(decoder: &str) -> &str {
    match decoder {
        "avdec_h264" => "avdec_h264 thread-type=slice",
        other => other,
    }
}

/// The "Linksy Camera" V4L2 device (v4l2loopback, made by linksy-camera-setup), if this
/// computer has one: every app can use a camera there, browsers and Zoom included.
pub fn camera_device() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/video4linux").ok()?;
    entries.flatten().find_map(|e| {
        let name = std::fs::read_to_string(e.path().join("name")).ok()?;
        (name.trim() == "Linksy Camera").then(|| PathBuf::from("/dev").join(e.file_name()))
    })
}

/// The step that gives the decoded stream a fixed frame rate, which a camera device needs.
/// `capssetter` (gst-plugins-good's "debug" plugin) does it without touching the frames; some
/// installs lack it and the session then ended at once (desktop#85), so `videorate`, from
/// gst-plugins-base, does it there. With no timestamps to go by it passes every frame on.
fn fixed_rate(fps: u32) -> String {
    if gst::ElementFactory::find("capssetter").is_some() {
        format!("capssetter caps=video/x-raw,framerate={fps}/1")
    } else {
        eprintln!("linksy-screen: no capssetter (gst-plugins-good's debug plugin); the camera's frame rate is set with videorate");
        format!("videorate ! video/x-raw,framerate={fps}/1")
    }
}

/// The receiving end of a camera session: decode and offer it as a camera for as long as
/// the stream lasts: on the Linksy Camera device when there is one, else to PipeWire apps.
fn webcam(socket: &std::path::Path, name: &str, fps: u32) -> Result<(), String> {
    let input = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let decoder = ["avdec_h264", "vah264dec", "openh264dec"]
        .into_iter()
        .find(|d| gst::ElementFactory::make(d).build().is_ok())
        .ok_or("no H.264 decoder: install gstreamer1-plugin-openh264 or gstreamer1-plugin-libav")?;
    let decoder = low_latency(decoder);
    // Offered the way webcams are, YUY2 at a fixed rate (what the phone sends: 30 or 60):
    // apps like OBS list a camera's formats and rates from this, and skip what they can't
    // name.
    // Quotes would break the property string; keep the name plain.
    let name: String = name.chars().filter(|c| !matches!(c, '"' | '\\' | ',')).collect();
    let pipewire = format!(
        "pipewiresink mode=provide sync=false \
         stream-properties=\"props,media.class=Video/Source,media.role=Camera,node.name=linksy-camera,node.description=\\\"{name}\\\"\""
    );
    // On the V4L2 device for browsers and Zoom, and to PipeWire too: the device only says
    // it's a camera while something writes to it (what browsers need), and PipeWire looked
    // at it before that, so PipeWire apps (OBS's camera source) would list it blank.
    let sink = match camera_device() {
        Some(dev) => format!(
            "tee name=cam ! queue leaky=downstream max-size-buffers=2 ! v4l2sink device={} sync=false \
             cam. ! queue leaky=downstream max-size-buffers=2 ! {pipewire}",
            dev.display()
        ),
        None => pipewire,
    };
    let description = format!(
        "fdsrc fd={} ! h264parse ! {decoder} ! videoconvert ! video/x-raw,format=YUY2 \
         ! {} ! {sink}",
        input.as_raw_fd(),
        fixed_rate(fps)
    );
    let result = play(&description);
    drop(input);
    result
}

/// What the microphone sessions carry: raw PCM, as both ends agree (docs/protocol.md §4.8).
const MIC_CAPS: &str = "audio/x-raw,format=S16LE,rate=48000,channels=1,layout=interleaved";

/// Share this device's microphone (or a test tone) on the socket.
fn share_mic(socket: &std::path::Path, test: bool) -> Result<(), String> {
    let output = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let source = if test { "audiotestsrc is-live=true freq=440 volume=0.3" } else { "autoaudiosrc" };
    // 10 ms buffers: small enough for a call, big enough not to flood the link.
    let description = format!("{source} ! audioconvert ! audioresample ! {MIC_CAPS} ! audiobuffersplit output-buffer-duration=1/100 ! fdsink fd={} sync=false", output.as_raw_fd());
    let result = play(&description);
    drop(output);
    result
}

/// The receiving end of a microphone session: offer the audio to every app as a
/// microphone for as long as it lasts. A leaky queue keeps the delay at most 100 ms: after a
/// Wi-Fi hiccup, late audio is dropped rather than played late forever.
/// A phone's sound on this computer's speakers. Short buffers and a queue that drops the old
/// when the sound comes in bursts, so what is heard is never seconds behind the screen.
fn speaker(socket: &std::path::Path, test: bool) -> Result<(), String> {
    let input = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let sink = if test { "fakesink sync=false" } else { "autoaudiosink sync=false" };
    let description = format!(
        "fdsrc fd={} ! rawaudioparse use-sink-caps=false format=pcm pcm-format=s16le sample-rate=48000 num-channels=2 \
         ! queue max-size-time=120000000 max-size-buffers=0 max-size-bytes=0 leaky=downstream ! audioconvert ! audioresample ! {sink}",
        input.as_raw_fd()
    );
    let result = play(&description);
    drop(input);
    result
}

fn microphone(socket: &std::path::Path, name: &str) -> Result<(), String> {
    let input = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let name: String = name.chars().filter(|c| !matches!(c, '"' | '\\' | ',')).collect();
    // The service ends this with SIGTERM: the virtual microphone goes with it
    stop_on_term();
    // A virtual source that audio is played into. A node published with `mode=provide` stops
    // delivering on PipeWire 1.6 (desktop#89); that stays the way when there is no pactl.
    let (sink, _source) = match virtual_mic::VirtualMic::create(&name) {
        Ok(source) => (virtual_mic::feed(), Some(source)),
        Err(e) => {
            eprintln!("linksy-screen: {e}; the microphone is published as a PipeWire node, which stops after a moment on PipeWire 1.6");
            // Audio/Source, not Audio/Source/Virtual: PipeWire 1.6 crashes the stream with the latter.
            let node = format!(
                "pipewiresink mode=provide sync=false \
                 stream-properties=\"props,media.class=Audio/Source,node.name=linksy-mic,node.description=\\\"{name}\\\"\""
            );
            (node, None)
        }
    };
    let description = format!(
        "fdsrc fd={} ! rawaudioparse use-sink-caps=false format=pcm pcm-format=s16le sample-rate=48000 num-channels=1 \
         ! queue max-size-time=100000000 max-size-buffers=0 max-size-bytes=0 leaky=downstream ! audioconvert ! {sink}",
        input.as_raw_fd()
    );
    let result = play(&description);
    drop(input);
    result
}

/// Set by SIGTERM, so a run can end the proper way and put back what it made.
static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn stop_on_term() {
    // SAFETY: the handler only stores to an atomic
    unsafe {
        libc::signal(libc::SIGTERM, on_term as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }
}

/// Run a pipeline until it ends or fails.
fn play(description: &str) -> Result<(), String> {
    let pipeline = gst::parse::launch(description).map_err(|e| format!("pipeline: {e}"))?;
    pipeline.set_state(gst::State::Playing).map_err(|e| format!("could not start: {e}"))?;
    let bus = pipeline.bus().ok_or("pipeline has no bus")?;
    let result = loop {
        if STOP.load(Ordering::Relaxed) {
            break Ok(());
        }
        // Wakes up now and then to look at STOP
        let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(200)) else { continue };
        match msg.view() {
            gst::MessageView::Eos(_) => break Ok(()),
            gst::MessageView::Error(e) => break Err(format!("{} ({:?})", e.error(), e.debug())),
            _ => {}
        }
    };
    let _ = pipeline.set_state(gst::State::Null);
    result
}

#[cfg(test)]
mod tests {
    use super::falls_behind;

    #[test]
    fn an_encoder_that_keeps_up_stays_at_60() {
        assert!(!falls_behind(60.0, 59.8));
        assert!(!falls_behind(60.0, 53.0));
        // A 144 Hz screen is cut to 60 on purpose
        assert!(!falls_behind(144.0, 59.9));
    }

    #[test]
    fn an_encoder_that_cannot_goes_down_to_30() {
        assert!(falls_behind(60.0, 40.0));
        assert!(falls_behind(60.0, 51.0));
        assert!(falls_behind(144.0, 45.0));
    }

    #[test]
    fn a_screen_that_hardly_moves_says_nothing() {
        // A still desktop sends the keepalive 10 times a second; a video at 30 is already 30
        assert!(!falls_behind(10.0, 2.0));
        assert!(!falls_behind(30.0, 20.0));
    }
}
