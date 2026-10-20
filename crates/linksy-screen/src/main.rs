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
//! `linksy-screen webcam --socket PATH [--name NAME]` does the opposite end of a camera
//! session: decodes the stream read from the socket and offers it to every app as a
//! PipeWire camera called NAME (browsers, Meet, GNOME Snapshot, OBS…).

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
    camera: bool,
    socket: PathBuf,
    token_file: Option<PathBuf>,
    control: bool,
    virtual_size: Option<(u32, u32)>,
    test: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut webcam = match args.next().as_deref() {
        Some("share") => None,
        Some("webcam") => Some("Linksy Camera".to_string()),
        _ => {
            return Err("usage: linksy-screen share --socket PATH [--token-file PATH] [--camera | --control] [--virtual [--size WxH]] [--test]\n       \
                        linksy-screen webcam --socket PATH [--name NAME]"
                .into());
        }
    };
    let (mut socket, mut token_file, mut control, mut test, mut camera) = (None, None, false, false, false);
    let (mut is_virtual, mut size) = (false, (1920, 1080));
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket = args.next().map(PathBuf::from),
            "--token-file" => token_file = args.next().map(PathBuf::from),
            "--control" => control = true,
            "--virtual" => is_virtual = true,
            "--size" => {
                let v = args.next().unwrap_or_default();
                size = v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).ok_or("--size takes WxH")?;
            }
            "--test" => test = true,
            "--camera" => camera = true,
            "--name" if webcam.is_some() => webcam = args.next(),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Args { webcam, camera, socket: socket.ok_or("--socket is required")?, token_file, control, virtual_size: is_virtual.then_some(size), test })
}

fn main() -> ExitCode {
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
        return webcam(&args.socket, name);
    }
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().map_err(|e| e.to_string())?;

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
        let token = args.token_file.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let screen = rt.block_on(portal::screen(token, args.control, args.virtual_size.is_some()))?;
        if let (Some(path), Some(new)) = (&args.token_file, &screen.restore_token) {
            let _ = std::fs::write(path, new);
        }
        // The PipeWire remote stays open for the life of the pipeline.
        let mut source = format!("pipewiresrc fd={} path={} do-timestamp=true keepalive-time=1000 always-copy=true", screen.fd.as_raw_fd(), screen.node);
        // A virtual monitor takes the size its stream negotiates.
        if let Some((w, h)) = args.virtual_size {
            source.push_str(&format!(" ! video/x-raw,width={w},height={h}"));
        }
        std::mem::forget(screen.fd);
        (source, screen.input)
    };

    // linksyd listens on the socket and relays everything written here to the viewer, and
    // the viewer's control messages back.
    let out = UnixStream::connect(&args.socket).map_err(|e| format!("{}: {e}", args.socket.display()))?;
    let control_in = out.try_clone().map_err(|e| e.to_string())?;
    let handle = rt.handle().clone();
    std::thread::spawn(move || {
        // View-only sessions read and ignore input.
        read_control_frames(control_in, |msg| {
            if let Some(input) = &input {
                handle.block_on(input.apply(&msg));
            }
        });
    });

    let (encoder, format) = encoder();
    let description = format!(
        "{source} ! videoconvert ! video/x-raw,format={format} ! queue max-size-buffers=2 leaky=downstream ! {encoder} \
         ! video/x-h264,profile=constrained-baseline ! h264parse config-interval=-1 \
         ! video/x-h264,stream-format=byte-stream,alignment=au \
         ! fdsink fd={} sync=false",
        out.as_raw_fd()
    );
    let pipeline = gst::parse::launch(&description).map_err(|e| format!("pipeline: {e}"))?;
    pipeline.set_state(gst::State::Playing).map_err(|e| format!("could not start: {e}"))?;
    let bus = pipeline.bus().ok_or("pipeline has no bus")?;
    let result = loop {
        let Some(msg) = bus.timed_pop(gst::ClockTime::NONE) else { break Ok(()) };
        match msg.view() {
            gst::MessageView::Eos(_) => break Ok(()),
            // The viewer went away (socket closed): a write error, and a normal end.
            gst::MessageView::Error(e) if e.src().is_some_and(|s| s.name().starts_with("fdsink")) => break Ok(()),
            gst::MessageView::Error(e) => break Err(format!("{} ({:?})", e.error(), e.debug())),
            _ => {}
        }
    };
    let _ = pipeline.set_state(gst::State::Null);
    drop(out);
    result
}

/// The best H.264 encoder available, and the raw format it wants: the GPU first, then
/// x264, then OpenH264 (always present on Fedora). All are asked for Constrained
/// Baseline, which every H.264 decoder handles (OpenH264's, phones' hardware ones).
fn encoder() -> (&'static str, &'static str) {
    let has = |name: &str| gst::ElementFactory::find(name).is_some();
    if has("vah264lpenc") {
        ("vah264lpenc rate-control=cbr bitrate=8000 key-int-max=60 target-usage=7", "NV12")
    } else if has("vah264enc") {
        ("vah264enc rate-control=cbr bitrate=8000 key-int-max=60 target-usage=7", "NV12")
    } else if has("x264enc") {
        ("x264enc tune=zerolatency speed-preset=superfast bitrate=8000 key-int-max=60 bframes=0", "I420")
    } else {
        ("openh264enc usage-type=screen complexity=low rate-control=bitrate bitrate=8000000 gop-size=60", "I420")
    }
}

/// The receiving end of a camera session: decode and offer it as a PipeWire camera for
/// as long as the stream lasts.
fn webcam(socket: &std::path::Path, name: &str) -> Result<(), String> {
    let input = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let decoder = ["avdec_h264", "vah264dec", "openh264dec"]
        .into_iter()
        .find(|d| gst::ElementFactory::make(d).build().is_ok())
        .ok_or("no H.264 decoder: install gstreamer1-plugin-openh264 or gstreamer1-plugin-libav")?;
    // Offered the way webcams are, YUY2 at a fixed 30 fps (what the phone sends): apps like
    // OBS list a camera's formats and rates from this, and skip what they can't name.
    // Quotes would break the property string; keep the name plain.
    let name: String = name.chars().filter(|c| !matches!(c, '"' | '\\' | ',')).collect();
    let description = format!(
        "fdsrc fd={} ! h264parse ! {decoder} ! videoconvert ! video/x-raw,format=YUY2 \
         ! capssetter caps=video/x-raw,framerate=30/1 \
         ! pipewiresink mode=provide sync=false \
           stream-properties=\"props,media.class=Video/Source,media.role=Camera,node.name=linksy-camera,node.description=\\\"{name}\\\"\"",
        input.as_raw_fd()
    );
    let pipeline = gst::parse::launch(&description).map_err(|e| format!("pipeline: {e}"))?;
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
    drop(input);
    result
}
