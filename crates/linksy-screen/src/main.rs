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
//! `linksy-screen share --camera --socket PATH [--test]` shares this device's camera
//! instead of a screen.
//!
//! `linksy-screen webcam --socket PATH [--name NAME] [--fps 30|60]` does the opposite end of a camera
//! session: decodes the stream read from the socket and offers it to every app as a
//! PipeWire camera called NAME (browsers, Meet, GNOME Snapshot, OBS…).
//!
//! `linksy-screen share --mic --socket PATH [--test]` shares this device's microphone, and
//! `linksy-screen mic --socket PATH [--name NAME]` offers a received one to every app as
//! a microphone called NAME. Audio crosses as raw 16-bit 48 kHz mono PCM (docs/protocol.md
//! §4.8): about 100 KB/s, and nothing to decode.

mod mutter;
mod portal;

use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;

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
    camera: bool,
    /// Share this device's microphone.
    mic: bool,
    socket: PathBuf,
    token_file: Option<PathBuf>,
    control: bool,
    /// Pointer and keyboard only, no screen (`input` sessions).
    input_only: bool,
    virtual_size: Option<(u32, u32)>,
    test: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let (mut webcam, mut microphone) = match args.next().as_deref() {
        Some("share") => (None, None),
        Some("webcam") => (Some("Linksy Camera".to_string()), None),
        Some("mic") => (None, Some("Linksy Microphone".to_string())),
        _ => {
            return Err("usage: linksy-screen share --socket PATH [--token-file PATH] [--camera | --mic | --control | --input-only] [--virtual [--size WxH]] [--test]\n       \
                        linksy-screen webcam --socket PATH [--name NAME] [--fps 30|60]\n       \
                        linksy-screen mic --socket PATH [--name NAME]"
                .into());
        }
    };
    let (mut socket, mut token_file, mut control, mut test, mut camera, mut mic) = (None, None, false, false, false, false);
    let mut input_only = false;
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
            "--test" => test = true,
            "--camera" => camera = true,
            "--mic" => mic = true,
            "--fps" => fps = args.next().and_then(|v| v.parse().ok()).filter(|f| matches!(f, 30 | 60)).ok_or("--fps takes 30 or 60")?,
            "--name" if webcam.is_some() => webcam = args.next(),
            "--name" if microphone.is_some() => microphone = args.next(),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Args { webcam, fps, microphone, camera, mic, socket: socket.ok_or("--socket is required")?, token_file, control, input_only, virtual_size: is_virtual.then_some(size), test })
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
        let pattern = format!("videotestsrc is-live=true pattern=ball motion=sweep background-color=0xff2563ff ! video/x-raw,width={w},height={h},framerate=30/1");
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

    let (encoder, format) = encoder();
    let gpu = encoder.starts_with("va") && gst::ElementFactory::find("vapostproc").is_some();
    // The screen through the desktop (PipeWire), not a camera or a test pattern
    let screen = source.starts_with("pipewiresrc");
    let size = args.virtual_size.map(|(w, h)| format!(",width={w},height={h}")).unwrap_or_default();
    // With a GPU encoder, frames stay on the GPU: the desktop hands them over in graphics
    // memory (DMA-BUF), and the GPU converts and encodes them, up to 60 a second with almost
    // no CPU. Where the desktop can't, they're uploaded (about 30 a second for a phone-shaped
    // display). On the CPU, a big screen (2.5 million pixels) can take longer to encode than
    // a frame lasts: at most 30 a second there, converted on several threads.
    let mut memory = if gpu && screen { Memory::Gpu } else { Memory::System };
    let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4));
    let chain = |memory: Memory| -> String {
        // Copied: the desktop's few buffers go back to it at once
        let copy = if screen && memory == Memory::System { " always-copy=true" } else { "" };
        let caps = match memory {
            // Caps with a memory feature, quoted for the parser
            Memory::Gpu => format!("capsfilter caps=\"video/x-raw(memory:DMABuf){size}\""),
            Memory::System if !size.is_empty() => format!("video/x-raw{size}"),
            // Whatever the source gives (a bare "video/x-raw" isn't caps to the parser)
            Memory::System => "identity".to_string(),
        };
        let convert = if gpu {
            "vapostproc ! capsfilter caps=\"video/x-raw(memory:VAMemory),format=NV12\"".to_string()
        } else {
            format!("videorate drop-only=true max-rate=30 ! videoconvert n-threads={threads} ! video/x-raw,format={format}")
        };
        format!("{copy} ! {caps} ! queue name=captured max-size-buffers=1 leaky=downstream ! {convert} ! queue max-size-buffers=2 leaky=downstream ! {encoder} name=encoder")
    };
    // GNOME can stop feeding a virtual monitor's stream when its layout changes (display
    // settings applied): no frame comes, not even the keepalive. Then the capture starts
    // again in the same session, the old one closed first.
    let restartable = args.virtual_size.is_some() && !args.test;
    let mut source = source;
    let mut restarts = 0;
    let result = loop {
        let description = format!(
            "{source}{} ! video/x-h264,profile=constrained-baseline ! h264parse config-interval=-1 \
             ! video/x-h264,stream-format=byte-stream,alignment=au \
             ! fdsink fd={} sync=false",
            chain(memory),
            out.as_raw_fd()
        );
        let pipeline = gst::parse::launch(&description).map_err(|e| format!("pipeline: {e}"))?;
        let last_frame = count_frames(&pipeline, encoder.split_whitespace().next().unwrap_or("encoder"));
        pipeline.set_state(gst::State::Playing).map_err(|e| format!("could not start: {e}"))?;
        let outcome = watch_bus(&pipeline, restartable.then_some(&last_frame));
        let _ = pipeline.set_state(gst::State::Null);
        drop(pipeline);
        match outcome {
            // The desktop couldn't hand frames over in graphics memory: upload them instead
            Some(Ended(Err(e))) if memory == Memory::Gpu && last_frame.load(std::sync::atomic::Ordering::Relaxed) == 0 => {
                eprintln!("linksy-screen: no frames in graphics memory ({e}), uploading them instead");
                memory = Memory::System;
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

/// Run until the pipeline ends; with `last_frame`, also until frames stop coming.
fn watch_bus(pipeline: &gst::Element, last_frame: Option<&std::sync::Arc<std::sync::atomic::AtomicU64>>) -> Option<Outcome> {
    let bus = pipeline.bus()?;
    loop {
        let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(500)) else {
            // The keepalive sends the last frame every second: three without any is a stall
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
            gst::MessageView::Error(e) => return Some(Ended(Err(format!("{} ({:?})", e.error(), e.debug())))),
            _ => {}
        }
    }
}

/// Ask the desktop for the screen (a new virtual monitor for a second display): the
/// pipeline's source, where input goes, and the PipeWire remote to close with it.
fn open_capture(args: &Args, rt: &tokio::runtime::Runtime) -> Result<(String, Option<portal::Input>), String> {
    let token = args.token_file.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let had_token = token.is_some();
    let screen = match rt.block_on(portal::screen(token, args.control, args.virtual_size.is_some())) {
        Ok(screen) => screen,
        // A remembered choice can go stale (its monitor gone, say) and then restore no
        // screen at all, every time: forget it and show the dialog.
        Err(e) if had_token && !e.ends_with("cancelled") => {
            eprintln!("linksy-screen: the remembered screen didn't restore ({e}), asking again");
            rt.block_on(portal::screen(None, args.control, args.virtual_size.is_some()))?
        }
        Err(e) => return Err(e),
    };
    // No token back (input refused, say): forget the old one so the dialog asks again.
    if let Some(path) = &args.token_file {
        let _ = match &screen.restore_token {
            Some(new) => std::fs::write(path, new),
            None if args.control => std::fs::remove_file(path),
            None => Ok(()),
        };
    }
    // The PipeWire remote stays open for the life of the pipeline (pipewiresrc closes it).
    // The memory the frames come in, and a virtual monitor's size, are chosen by the chain
    let source = format!("pipewiresrc fd={} path={} do-timestamp=true keepalive-time=1000", screen.fd.as_raw_fd(), screen.node);
    std::mem::forget(screen.fd);
    Ok((source, screen.input))
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
    Ok(())
}

/// The best H.264 encoder available, and the raw format it wants: the GPU first, then
/// x264, then OpenH264 (always present on Fedora). All are asked for Constrained
/// Baseline, which every H.264 decoder handles (OpenH264's, phones' hardware ones).
fn encoder() -> (&'static str, &'static str) {
    let has = |name: &str| gst::ElementFactory::find(name).is_some();
    if has("vah264lpenc") && constant_bitrate("vah264lpenc") {
        ("vah264lpenc rate-control=cbr bitrate=16000 key-int-max=60 target-usage=7", "NV12")
    } else if has("vah264enc") && constant_bitrate("vah264enc") {
        ("vah264enc rate-control=cbr bitrate=16000 key-int-max=60 target-usage=7", "NV12")
    } else if has("x264enc") {
        // One slice per frame: zerolatency would cut each frame into a slice per thread,
        // which decoders fed a NAL unit at a time (phones) never assemble into a picture.
        // Two threads work on two frames at once instead: one frame later, nearly twice as fast.
        // ultrafast: a sharp desktop with windows moving on it cost superfast 10 times more
        // live than a video does (half-pixel motion search), and fell to 4 fps on a laptop on
        // battery; 16 Mbit/s keeps it sharp, which a local network carries easily.
        ("x264enc tune=zerolatency speed-preset=ultrafast bitrate=16000 key-int-max=60 bframes=0 sliced-threads=false threads=2", "I420")
    } else {
        ("openh264enc usage-type=screen complexity=low rate-control=bitrate bitrate=8000000 gop-size=60", "I420")
    }
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
         ! capssetter caps=video/x-raw,framerate={fps}/1 ! {sink}",
        input.as_raw_fd()
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
fn microphone(socket: &std::path::Path, name: &str) -> Result<(), String> {
    let input = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let name: String = name.chars().filter(|c| !matches!(c, '"' | '\\' | ',')).collect();
    // Audio/Source, not Audio/Source/Virtual: PipeWire 1.6 crashes the stream with the latter.
    let description = format!(
        "fdsrc fd={} ! rawaudioparse use-sink-caps=false format=pcm pcm-format=s16le sample-rate=48000 num-channels=1 \
         ! queue max-size-time=100000000 max-size-buffers=0 max-size-bytes=0 leaky=downstream ! audioconvert \
         ! pipewiresink mode=provide sync=false \
         stream-properties=\"props,media.class=Audio/Source,node.name=linksy-mic,node.description=\\\"{name}\\\"\"",
        input.as_raw_fd()
    );
    let result = play(&description);
    drop(input);
    result
}

/// Run a pipeline until it ends or fails.
fn play(description: &str) -> Result<(), String> {
    let pipeline = gst::parse::launch(description).map_err(|e| format!("pipeline: {e}"))?;
    pipeline.set_state(gst::State::Playing).map_err(|e| format!("could not start: {e}"))?;
    let bus = pipeline.bus().ok_or("pipeline has no bus")?;
    let result = loop {
        let Some(msg) = bus.timed_pop(gst::ClockTime::NONE) else { break Ok(()) };
        match msg.view() {
            gst::MessageView::Eos(_) => break Ok(()),
            gst::MessageView::Error(e) => break Err(format!("{} ({:?})", e.error(), e.debug())),
            _ => {}
        }
    };
    let _ = pipeline.set_state(gst::State::Null);
    result
}
