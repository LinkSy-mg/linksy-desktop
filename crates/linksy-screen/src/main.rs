//! `linksy-screen`: the media side of Linksy sessions, started by `linksyd` only while a
//! session runs, so the always-on service never loads GStreamer.
//!
//! `linksy-screen share --socket PATH [--token-file PATH] [--test]`
//!
//! Asks the desktop for a screen through the ScreenCast portal (the user picks a screen
//! the first time; the choice is remembered with a restore token), encodes it to H.264
//! and writes the Annex-B byte stream to the Unix socket `linksyd` listens on, which
//! relays it to the viewer. `--test` streams a moving test pattern instead of the
//! screen, for development and tests.

use std::os::fd::{AsRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;

use gstreamer as gst;
use gstreamer::prelude::*;

struct Args {
    socket: PathBuf,
    token_file: Option<PathBuf>,
    test: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("share") {
        return Err("usage: linksy-screen share --socket PATH [--token-file PATH] [--test]".into());
    }
    let (mut socket, mut token_file, mut test) = (None, None, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket = args.next().map(PathBuf::from),
            "--token-file" => token_file = args.next().map(PathBuf::from),
            "--test" => test = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Args { socket: socket.ok_or("--socket is required")?, token_file, test })
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
    let source = if args.test {
        "videotestsrc is-live=true pattern=ball motion=sweep background-color=0xff2563ff ! video/x-raw,width=1280,height=720,framerate=30/1".to_string()
    } else {
        let (fd, node) = portal_screen(args.token_file.as_deref())?;
        // The PipeWire remote stays open for the life of the pipeline.
        format!("pipewiresrc fd={fd} path={node} do-timestamp=true keepalive-time=1000 always-copy=true")
    };
    // linksyd listens on the socket and relays everything written here to the viewer.
    let out = UnixStream::connect(&args.socket).map_err(|e| format!("{}: {e}", args.socket.display()))?;
    let (encoder, format) = encoder();
    let description = format!(
        "{source} ! videoconvert ! video/x-raw,format={format} ! queue max-size-buffers=2 leaky=downstream ! {encoder} \
         ! h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au \
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
/// x264, then OpenH264 (always present on Fedora).
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

/// Ask the ScreenCast portal for a monitor. Returns the PipeWire remote fd and node id.
fn portal_screen(token_file: Option<&std::path::Path>) -> Result<(i32, u32), String> {
    use ashpd::desktop::PersistMode;
    use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType, StartCastOptions};

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let token = token_file.and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let proxy = Screencast::new().await.map_err(|e| format!("screen sharing isn't available: {e}"))?;
        let session = proxy.create_session(Default::default()).await.map_err(|e| e.to_string())?;
        let options = SelectSourcesOptions::default()
            .set_cursor_mode(CursorMode::Embedded)
            .set_sources(Some(SourceType::Monitor.into()))
            .set_multiple(false)
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(token.as_deref());
        proxy.select_sources(&session, options).await.map_err(|e| e.to_string())?;
        let streams = proxy
            .start(&session, None, StartCastOptions::default())
            .await
            .map_err(|e| e.to_string())?
            .response()
            .map_err(|_| "screen sharing was cancelled".to_string())?;
        let node = streams.streams().first().ok_or("no screen was chosen")?.pipe_wire_node_id();
        if let (Some(path), Some(new)) = (token_file, streams.restore_token()) {
            let _ = std::fs::write(path, new);
        }
        let fd = proxy.open_pipe_wire_remote(&session, Default::default()).await.map_err(|e| e.to_string())?;
        // The session must outlive this function: leak it with the runtime's lifetime
        // (the process exits when sharing ends).
        std::mem::forget(session);
        Ok((fd.into_raw_fd(), node))
    })
}
