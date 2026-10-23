//! The QML app's window on linksyd. QML sends requests as JSON (the same `{"cmd": …}`
//! the service reads) and gets the reply back as a signal; the service's events and
//! history changes arrive as signals too, always on the Qt thread.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, connected)]
        #[qproperty(bool, starting)]
        type Backend = super::BackendRust;

        /// Connect to the service, starting it first when it isn't running.
        #[qinvokable]
        fn start(self: Pin<&mut Self>);

        /// Send `request` (JSON); `reply` answers with the same id.
        #[qinvokable]
        fn call(self: Pin<&mut Self>, request: &QString) -> i32;

        #[qinvokable]
        fn tr(&self, english: &QString) -> QString;

        #[qinvokable]
        fn env(&self, name: &QString) -> QString;

        #[qinvokable]
        fn version(&self) -> QString;

        #[qinvokable]
        fn clipboard(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "setClipboard"]
        fn set_clipboard(&self, text: &QString);

        /// A `linksy://pair` link as a QR code, an SVG data URL for an Image.
        #[qinvokable]
        #[cxx_name = "qrCode"]
        fn qr_code(&self, text: &QString) -> QString;

        /// Development aid (scripts/qt-shot.sh): save the window as an image.
        #[qinvokable]
        #[cxx_name = "saveWindow"]
        fn save_window(&self, path: &QString) -> bool;

        /// Play a session's stream (its socket) in the QML item named `item`; empty when it
        /// plays, else why not ("missing-part" when a GStreamer piece is missing).
        #[qinvokable]
        #[cxx_name = "playStream"]
        fn play_stream(&self, item: &QString, socket: &QString) -> QString;

        /// Show what apps get from the Linksy Camera in the QML item named `item`; empty when
        /// it plays, else why not.
        #[qinvokable]
        #[cxx_name = "previewCamera"]
        fn preview_camera(&self, item: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "stopStream"]
        fn stop_stream(&self);

        /// "1920x1080" once frames arrive.
        #[qinvokable]
        #[cxx_name = "streamSize"]
        fn stream_size(&self) -> QString;

        /// A control message (JSON: pointer, button, scroll, key) to the screen's device.
        #[qinvokable]
        #[cxx_name = "sendControl"]
        fn send_control(&self, json: &QString) -> bool;

        /// Check what Finish Setup lists; `setupChecked` answers (JSON).
        #[qinvokable]
        #[cxx_name = "checkSetup"]
        fn check_setup(self: Pin<&mut Self>);

        /// Set up the items with these ids (a JSON list; empty: everything that can be),
        /// one password; `setupStep` tells what it does, `setupFinished` how it ended.
        #[qinvokable]
        #[cxx_name = "runSetup"]
        fn run_setup(self: Pin<&mut Self>, ids: &QString);

        /// True once per version when something can be set up: open Finish Setup.
        #[qinvokable]
        #[cxx_name = "offerSetup"]
        fn offer_setup(&self) -> bool;

        /// What apps call a phone's camera.
        #[qinvokable]
        #[cxx_name = "cameraName"]
        fn camera_name(&self, phone: &QString) -> QString;

        #[qinvokable]
        fn reboot(&self);

        /// What's New as JSON ({version, notes, repo, highlights: [{icon, title, body}]}):
        /// `any_time` for the menu (this version's), else after an update, once (empty
        /// highlights when there's nothing to show).
        #[qinvokable]
        #[cxx_name = "whatsNew"]
        fn whats_new(&self, any_time: bool) -> QString;

        /// Linksy's languages: [{code, name}] (each in its own language).
        #[qinvokable]
        fn languages(&self) -> QString;

        /// The language chosen in Preferences; empty: the system's.
        #[qinvokable]
        #[cxx_name = "chosenLanguage"]
        fn chosen_language(&self) -> QString;

        /// Choose a language (empty: the system's again), for the next start; empty when
        /// saved, else why not.
        #[qinvokable]
        #[cxx_name = "chooseLanguage"]
        fn choose_language(&self, code: &QString) -> QString;

        /// This computer's address on the network, for "Pair by Address" elsewhere.
        #[qinvokable]
        #[cxx_name = "localAddress"]
        fn local_address(&self, port: i32) -> QString;

        /// Start Linksy again, the service too (not one started by hand).
        #[qinvokable]
        fn restart(self: Pin<&mut Self>);

        /// Whether phones can be used by cable here: empty when they can, else why not.
        #[qinvokable]
        #[cxx_name = "usbReady"]
        fn usb_ready(&self) -> QString;

        #[qsignal]
        #[cxx_name = "setupChecked"]
        fn setup_checked(self: Pin<&mut Backend>, json: QString);

        #[qsignal]
        #[cxx_name = "setupStep"]
        fn setup_step(self: Pin<&mut Backend>, text: QString);

        /// {"ok": true} | {"error": "…"} | {"restart": "<steps>"}
        #[qsignal]
        #[cxx_name = "setupFinished"]
        fn setup_finished(self: Pin<&mut Backend>, json: QString);

        #[qsignal]
        fn reply(self: Pin<&mut Backend>, id: i32, ok: bool, json: QString);

        #[qsignal]
        fn event(self: Pin<&mut Backend>, json: QString);

        #[qsignal]
        fn history(self: Pin<&mut Backend>, json: QString);

        /// Couldn't reach the service, or it went away.
        #[qsignal]
        fn offline(self: Pin<&mut Backend>, why: QString);
    }

    impl cxx_qt::Threading for Backend {}
}

use std::pin::Pin;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use linksy_ipc::{Client, Request};
use linksy_setup::{Failure, Need, Ui};

pub struct BackendRust {
    connected: bool,
    starting: bool,
    rt: tokio::runtime::Runtime,
    client: Arc<Mutex<Option<Arc<Client>>>>,
    next: AtomicI32,
}

impl Default for BackendRust {
    fn default() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("linksy-ipc")
            .enable_all()
            .build()
            .expect("tokio runtime");
        Self { connected: false, starting: false, rt, client: Arc::default(), next: AtomicI32::new(1) }
    }
}

impl qobject::Backend {
    pub fn start(mut self: Pin<&mut Self>) {
        if *self.starting() {
            return;
        }
        self.as_mut().set_starting(true);
        let thread = self.qt_thread();
        let slot = self.rust().client.clone();
        self.rust().rt.spawn(async move {
            let client = match connect().await {
                Ok(c) => Arc::new(c),
                Err(e) => {
                    let _ = thread.queue(move |mut b| {
                        b.as_mut().set_starting(false);
                        b.as_mut().offline(QString::from(&e));
                    });
                    return;
                }
            };
            *slot.lock().unwrap() = Some(client.clone());
            let _ = thread.queue(|mut b| {
                b.as_mut().set_starting(false);
                b.as_mut().set_connected(true);
            });
            let (mut events, mut history) = (client.events(), client.history_updates());
            use tokio::sync::broadcast::error::RecvError;
            loop {
                tokio::select! {
                    e = events.recv() => match e {
                        Ok(e) => {
                            let json = serde_json::to_string(&e).unwrap_or_default();
                            let _ = thread.queue(move |b| b.event(QString::from(&json)));
                        }
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    },
                    h = history.recv() => match h {
                        Ok(h) => {
                            let json = serde_json::to_string(&h).unwrap_or_default();
                            let _ = thread.queue(move |b| b.history(QString::from(&json)));
                        }
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    },
                }
            }
            // The service went away.
            *slot.lock().unwrap() = None;
            let _ = thread.queue(|mut b| {
                b.as_mut().set_connected(false);
                b.as_mut().offline(QString::from(""));
            });
        });
    }

    pub fn call(self: Pin<&mut Self>, request: &QString) -> i32 {
        let id = self.rust().next.fetch_add(1, Ordering::Relaxed);
        let thread = self.qt_thread();
        let client = self.rust().client.lock().unwrap().clone();
        let request = request.to_string();
        self.rust().rt.spawn(async move {
            let result = match (client, serde_json::from_str::<Request>(&request)) {
                (None, _) => Err("Linksy isn't running".to_string()),
                (_, Err(e)) => Err(format!("bad request {request}: {e}")),
                (Some(c), Ok(r)) => c.call::<serde_json::Value>(r).await.map_err(|e| e.to_string()),
            };
            let (ok, json) = match result {
                Ok(v) => (true, v.to_string()),
                Err(e) => (false, serde_json::Value::String(e).to_string()),
            };
            let _ = thread.queue(move |b| b.reply(id, ok, QString::from(&json)));
        });
        id
    }

    pub fn tr(&self, english: &QString) -> QString {
        let english = english.to_string();
        QString::from(linksy_i18n::lookup(&english))
    }

    pub fn env(&self, name: &QString) -> QString {
        QString::from(&std::env::var(name.to_string()).unwrap_or_default())
    }

    pub fn version(&self) -> QString {
        QString::from(env!("CARGO_PKG_VERSION"))
    }

    pub fn clipboard(&self) -> QString {
        crate::app::ffi::clipboard_text()
    }

    pub fn set_clipboard(&self, text: &QString) {
        crate::app::ffi::set_clipboard_text(text);
    }

    pub fn save_window(&self, path: &QString) -> bool {
        crate::app::ffi::save_window(path)
    }

    pub fn play_stream(&self, item: &QString, socket: &QString) -> QString {
        crate::app::ffi::play_stream(item, socket)
    }

    pub fn preview_camera(&self, item: &QString) -> QString {
        // Read the camera as any app would: the Linksy Camera device, else the PipeWire camera.
        // Development aid: LINKSY_TEST_CAMERA shows a test pattern instead.
        let source = match linksy_setup::camera_device() {
            _ if std::env::var_os("LINKSY_TEST_CAMERA").is_some() => "videotestsrc pattern=smpte ! video/x-raw,width=1280,height=720".to_string(),
            Some(dev) => format!("v4l2src device={}", dev.display()),
            None => "pipewiresrc target-object=linksy-camera always-copy=true".to_string(),
        };
        crate::app::ffi::play_source(item, &QString::from(&source))
    }

    pub fn stop_stream(&self) {
        crate::app::ffi::stop_stream();
    }

    pub fn stream_size(&self) -> QString {
        crate::app::ffi::stream_size()
    }

    pub fn send_control(&self, json: &QString) -> bool {
        crate::app::ffi::send_control(json)
    }

    pub fn check_setup(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        // D-Bus and systemctl calls: off the Qt thread
        self.rust().rt.spawn_blocking(move || {
            let json = setup_json().to_string();
            let _ = thread.queue(move |b| b.setup_checked(QString::from(&json)));
        });
    }

    pub fn run_setup(self: Pin<&mut Self>, ids: &QString) {
        let thread = self.qt_thread();
        let ids: Vec<String> = serde_json::from_str(&ids.to_string()).unwrap_or_default();
        self.rust().rt.spawn_blocking(move || {
            let items: Vec<_> = linksy_setup::actionable(&linksy_setup::check(Ui::Qt)).into_iter().filter(|i| ids.is_empty() || ids.iter().any(|id| id == i.id)).collect();
            let args = linksy_setup::root_args(&items);
            let step = |text: String| {
                let _ = thread.queue(move |b| b.setup_step(QString::from(&text)));
            };
            let mut result = if args.is_empty() { Ok(()) } else { linksy_setup::run_as_root(&args, step) };
            if result.is_ok() && linksy_setup::wants_autostart(&items) {
                result = linksy_setup::enable_autostart();
            }
            let json = match result {
                Ok(()) => serde_json::json!({ "ok": true }),
                Err(Failure::Message(m)) => serde_json::json!({ "error": m }),
                Err(Failure::Restart(code)) => serde_json::json!({ "restart": linksy_setup::restart_steps(code.as_deref()) }),
            }
            .to_string();
            let _ = thread.queue(move |b| b.setup_finished(QString::from(&json)));
        });
    }

    pub fn offer_setup(&self) -> bool {
        linksy_setup::offer_now(Ui::Qt, env!("CARGO_PKG_VERSION"))
    }

    pub fn camera_name(&self, phone: &QString) -> QString {
        QString::from(&linksy_setup::camera_name(&phone.to_string()))
    }

    pub fn whats_new(&self, any_time: bool) -> QString {
        let version = env!("CARGO_PKG_VERSION");
        let news = if any_time { linksy_news::latest(version) } else { linksy_news::to_show(version) };
        if !any_time && !news.is_empty() {
            linksy_news::mark_seen(version);
        }
        let highlights: Vec<_> = news.iter().map(|h| serde_json::json!({ "icon": h.qt, "title": h.title, "body": h.body })).collect();
        let json = serde_json::json!({ "version": version, "notes": linksy_news::notes_url(version), "repo": linksy_news::REPO, "highlights": highlights });
        QString::from(&json.to_string())
    }

    pub fn languages(&self) -> QString {
        let list: Vec<_> = linksy_i18n::LANGUAGES.iter().map(|(code, name)| serde_json::json!({ "code": code, "name": name })).collect();
        QString::from(&serde_json::Value::from(list).to_string())
    }

    pub fn chosen_language(&self) -> QString {
        QString::from(&linksy_i18n::chosen().unwrap_or_default())
    }

    pub fn choose_language(&self, code: &QString) -> QString {
        let code = code.to_string();
        match linksy_i18n::choose(Some(code.as_str()).filter(|c| !c.is_empty())) {
            Ok(()) => QString::default(),
            Err(e) => QString::from(&e.to_string()),
        }
    }

    pub fn local_address(&self, port: i32) -> QString {
        let ip = std::net::UdpSocket::bind("0.0.0.0:0").ok().and_then(|s| {
            // Nothing is sent: connecting a UDP socket only picks the route
            s.connect("192.0.2.1:9").ok()?;
            Some(s.local_addr().ok()?.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        });
        let Some(ip) = ip else { return QString::default() };
        let shown = if port == 47_200 || port <= 0 { ip.to_string() } else { std::net::SocketAddr::new(ip, port as u16).to_string() };
        QString::from(&shown)
    }

    pub fn restart(self: Pin<&mut Self>) {
        // A service started by hand (LINKSY_SOCKET, development) is left running
        let client = self.rust().client.lock().unwrap().clone().filter(|_| std::env::var_os("LINKSY_SOCKET").is_none());
        let thread = self.qt_thread();
        self.rust().rt.spawn(async move {
            if let Some(c) = client {
                let _ = tokio::time::timeout(Duration::from_secs(3), c.quit()).await;
                // Wait for it to go, so the new window starts a new one
                for _ in 0..20 {
                    if std::fs::metadata(linksy_ipc::socket_path()).is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            }
            if let Ok(exe) = std::env::current_exe() {
                // A moment later, once this window has let go of its instance socket
                let _ = std::process::Command::new("sh").args(["-c", "sleep 1; exec \"$0\""]).arg(exe).spawn();
            }
            let _ = thread.queue(|_| crate::app::ffi::quit_app());
        });
    }

    pub fn usb_ready(&self) -> QString {
        QString::from(&linksy_setup::usb_ready().err().unwrap_or_default())
    }

    pub fn reboot(&self) {
        let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
    }

    pub fn qr_code(&self, text: &QString) -> QString {
        let Ok(code) = qrcode::QrCode::new(text.to_string().as_bytes()) else { return QString::default() };
        let width = code.width();
        let mut path = String::new();
        for (i, c) in code.to_colors().iter().enumerate() {
            if *c == qrcode::Color::Dark {
                path.push_str(&format!("M{} {}h1v1h-1z", i % width, i / width));
            }
        }
        // A quiet zone of 2 modules, as scanners like.
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='-2 -2 {s} {s}' shape-rendering='crispEdges'><rect x='-2' y='-2' width='{s}' height='{s}' fill='white'/><path d='{path}' fill='black'/></svg>",
            s = width + 4
        );
        QString::from(&format!("data:image/svg+xml;utf8,{}", svg.replace('#', "%23")))
    }
}

/// Finish Setup's list for QML: each item, what it needs, and the terminal command.
fn setup_json() -> serde_json::Value {
    let items = linksy_setup::check(Ui::Qt);
    let list: Vec<_> = items
        .iter()
        .map(|i| {
            let (need, note) = match &i.need {
                Need::Ready => ("ready", None),
                Need::SetUp => ("setup", None),
                Need::Install(_) => match linksy_setup::layered_hint(i.why) {
                    Some(hint) => ("manual", Some(hint)),
                    None => ("install", None),
                },
                Need::Manual(what) => ("manual", Some(format!("{} {what}", i.why))),
            };
            serde_json::json!({ "id": i.id, "title": i.title, "why": note.unwrap_or_else(|| i.why.to_string()), "need": need })
        })
        .collect();
    serde_json::json!({
        "items": list,
        "missing": linksy_setup::missing(&items),
        "everything": linksy_setup::actionable(&items).len(),
        "command": linksy_setup::terminal_command(&items),
    })
}

/// The service's socket; when nobody answers, start the service (its systemd unit when
/// installed, else `linksyd` next to this app or on PATH) and wait for it.
async fn connect() -> Result<Client, String> {
    if let Ok(c) = Client::connect().await {
        return Ok(c);
    }
    // Pointed at a particular service (development): that one is started by hand.
    if std::env::var_os("LINKSY_SOCKET").is_some() {
        return Err(String::new());
    }
    use std::process::{Command, Stdio};
    let via_systemd = Command::new("systemctl")
        .args(["--user", "start", "linksyd.service"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !via_systemd {
        let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("linksyd"))).filter(|p| p.exists());
        Command::new(beside.unwrap_or_else(|| "linksyd".into()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| linksy_i18n::t!("Could not start the service: {error}", error = e))?;
    }
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if let Ok(c) = Client::connect().await {
            return Ok(c);
        }
    }
    Err(String::new())
}
