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

    pub fn stop_stream(&self) {
        crate::app::ffi::stop_stream();
    }

    pub fn stream_size(&self) -> QString {
        crate::app::ffi::stream_size()
    }

    pub fn send_control(&self, json: &QString) -> bool {
        crate::app::ffi::send_control(json)
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
