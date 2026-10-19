//! Screen mirroring in the app: watching another device's screen in its own window, and,
//! on the sharing side, the request prompt and a "you're sharing" banner with Stop.

use std::cell::RefCell;
use std::os::fd::{AsRawFd, OwnedFd};
use std::rc::Rc;

use adw::prelude::*;
use gstreamer as gst;
use gstreamer::prelude::*;
use gtk::{gdk, glib};
use linksy_ipc::SessionInfo;
use linksy_ipc::linksy_core::{DeviceInfo, Fingerprint, SessionKind, Uuid};

use super::Window;

impl Window {
    /// Ask a device to share its screen and open a viewer once it does.
    pub(super) fn view_screen(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        let viewer = Viewer::open(self, &name);
        let call = self.bridge.call(&c, move |c| async move { c.start_session(id.to_string(), SessionKind::Screen).await });
        glib::spawn_future_local(async move {
            match call.await {
                Ok(info) => viewer.play(info),
                Err(e) if e.contains("declined") => viewer.ended(&format!("{name} didn't allow it"), "Nothing is shared unless the other device says yes."),
                Err(e) if e.contains("timed out") => viewer.ended("No answer", &format!("Nobody answered on {name}.")),
                Err(e) => viewer.ended("Couldn't Connect", &crate::fmt::capitalize(&e)),
            }
        });
    }

    /// Someone wants to see this screen.
    pub(super) fn show_session_request(self: &Rc<Self>, request: Uuid, from: &DeviceInfo, kind: SessionKind) {
        let (heading, body) = match kind {
            SessionKind::Control => (
                format!("{} Wants to Control Your Screen", from.name),
                format!("{} will see your screen and use your mouse and keyboard until you stop. You'll pick which screen next.", from.name),
            ),
            SessionKind::Screen => (
                format!("{} Wants to See Your Screen", from.name),
                format!("{} will see everything on your screen until you stop sharing. You'll pick which screen next.", from.name),
            ),
        };
        let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
        dialog.add_responses(&[("deny", "Don't Allow"), ("allow", "Allow")]);
        dialog.set_response_appearance("allow", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("deny"));
        dialog.set_close_response("deny");
        let w = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            let Some(w) = w.upgrade() else { return };
            if w.state.borrow_mut().offer_dialogs.remove(&request).is_none() {
                return;
            }
            w.respond(request, response == "allow");
        });
        // Shares the offer prompts' bookkeeping: closed if answered elsewhere.
        self.state.borrow_mut().offer_dialogs.insert(request, dialog.clone());
        dialog.present(Some(&self.window));
    }

    pub(super) fn sharing_started(self: &Rc<Self>, session: Uuid, peer: &DeviceInfo) {
        self.close_offer_dialog(&session);
        self.state.borrow_mut().sharing = Some(session);
        self.sharing_banner.set_title(&glib::markup_escape_text(&format!("{} is watching your screen", peer.name)));
        self.sharing_banner.set_revealed(true);
    }

    pub(super) fn session_ended(self: &Rc<Self>, session: Uuid) {
        self.close_offer_dialog(&session);
        let mut state = self.state.borrow_mut();
        if state.sharing == Some(session) {
            state.sharing = None;
            self.sharing_banner.set_revealed(false);
        }
    }

    pub(super) fn stop_sharing(self: &Rc<Self>) {
        let Some(session) = self.state.borrow().sharing else { return };
        self.end_session(session);
    }

    fn end_session(self: &Rc<Self>, session: Uuid) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.end_session(session).await });
        glib::spawn_future_local(async move {
            let _ = call.await;
        });
    }
}

/// A window showing another device's screen.
#[derive(Clone)]
struct Viewer {
    window: adw::Window,
    stack: gtk::Stack,
    picture: gtk::Picture,
    ended: adw::StatusPage,
    toolbar: adw::ToolbarView,
    name: String,
    pipeline: Rc<RefCell<Option<gst::Element>>>,
    /// Kept alive with the pipeline: its bus watch, and the socket fdsrc reads.
    watch: Rc<RefCell<Option<gst::bus::BusWatchGuard>>>,
    socket: Rc<RefCell<Option<OwnedFd>>>,
    session: Rc<RefCell<Option<Uuid>>>,
}

impl Viewer {
    fn open(owner: &Rc<Window>, name: &str) -> Self {
        let waiting = adw::StatusPage::builder()
            .title(format!("Asking {name}…"))
            .description(format!("{name} needs to allow sharing and pick a screen."))
            .child(&adw::Spinner::builder().width_request(32).height_request(32).build())
            .build();
        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).hexpand(true).vexpand(true).css_classes(["screen-view"]).build();
        let again = gtk::Button::builder().label("Watch Again").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).build();
        let ended = adw::StatusPage::builder().icon_name("video-display-symbolic").child(&again).build();
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        stack.add_named(&waiting, Some("waiting"));
        stack.add_named(&picture, Some("video"));
        stack.add_named(&ended, Some("ended"));

        let fullscreen = gtk::Button::builder().icon_name("view-fullscreen-symbolic").tooltip_text("Full Screen (F11)").build();
        let header = adw::HeaderBar::new();
        header.pack_end(&fullscreen);
        let toolbar = adw::ToolbarView::builder().content(&stack).build();
        toolbar.add_top_bar(&header);
        let window = adw::Window::builder()
            .title(format!("{name}'s Screen"))
            .default_width(1100)
            .default_height(700)
            .width_request(360)
            .height_request(240)
            .content(&toolbar)
            .build();
        if let Some(app) = owner.window.application() {
            window.set_application(Some(&app));
        }

        let viewer = Self {
            window: window.clone(),
            stack,
            picture: picture.clone(),
            ended,
            toolbar: toolbar.clone(),
            name: name.to_string(),
            pipeline: Rc::default(),
            watch: Rc::default(),
            socket: Rc::default(),
            session: Rc::default(),
        };

        // Full screen: the button, F11 or a double click; Esc leaves it.
        let w = window.clone();
        fullscreen.connect_clicked(move |_| w.fullscreen());
        let t = toolbar.clone();
        window.connect_fullscreened_notify(move |w| t.set_reveal_top_bars(!w.is_fullscreen()));
        let keys = gtk::EventControllerKey::new();
        let w = window.clone();
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::F11 => {
                if w.is_fullscreen() { w.unfullscreen() } else { w.fullscreen() }
                glib::Propagation::Stop
            }
            gdk::Key::Escape if w.is_fullscreen() => {
                w.unfullscreen();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        window.add_controller(keys);
        let clicks = gtk::GestureClick::new();
        let w = window.clone();
        clicks.connect_pressed(move |_, n, _, _| {
            if n == 2 {
                if w.is_fullscreen() { w.unfullscreen() } else { w.fullscreen() }
            }
        });
        picture.add_controller(clicks);

        let (owner_weak, name) = (Rc::downgrade(owner), name.to_string());
        let v = viewer.clone();
        again.connect_clicked(move |_| {
            let Some(owner) = owner_weak.upgrade() else { return };
            let Some(id) = owner.state.borrow().devices.iter().find(|d| d.device.name == name).map(|d| d.device.id) else { return };
            v.window.close();
            owner.view_screen(id);
        });

        // Closing the window stops the session on both devices.
        let (v, owner_weak) = (viewer.clone(), Rc::downgrade(owner));
        window.connect_close_request(move |_| {
            v.stop_pipeline();
            if let (Some(owner), Some(session)) = (owner_weak.upgrade(), v.session.borrow_mut().take()) {
                owner.end_session(session);
            }
            glib::Propagation::Proceed
        });
        window.present();
        viewer
    }

    /// Connect to the session's socket and decode the stream into the picture.
    fn play(&self, info: SessionInfo) {
        *self.session.borrow_mut() = Some(info.session);
        let stream = match std::os::unix::net::UnixStream::connect(&info.socket) {
            Ok(s) => s,
            Err(e) => return self.ended("Couldn't Connect", &e.to_string()),
        };
        let socket = OwnedFd::from(stream);
        let Some(decoder) = decoder() else {
            return self.ended(
                "Can't Play This Screen",
                "This computer has no H.264 video decoder. Install gstreamer1-plugin-openh264 (or gstreamer1-plugin-libav) and try again.",
            );
        };
        let description = format!("fdsrc fd={} ! h264parse ! {decoder} ! videoconvert ! gtk4paintablesink name=sink sync=false", socket.as_raw_fd());
        let pipeline = match gst::parse::launch(&description) {
            Ok(p) => p,
            Err(e) => return self.ended("Can't Show This Screen", &format!("A video component is missing: {e}")),
        };
        *self.socket.borrow_mut() = Some(socket);
        let sink = pipeline.downcast_ref::<gst::Bin>().and_then(|b| b.by_name("sink"));
        if let Some(paintable) = sink.map(|s| s.property::<gdk::Paintable>("paintable")) {
            self.picture.set_paintable(Some(&paintable));
        }
        let bus = pipeline.bus().expect("pipeline bus");
        let (v, name) = (self.clone(), self.name.clone());
        let watch = bus.add_watch_local(move |_, msg| {
            match msg.view() {
                gst::MessageView::StateChanged(s) if s.current() == gst::State::Playing => v.stack.set_visible_child_name("video"),
                gst::MessageView::Eos(_) => v.ended("Sharing Stopped", &format!("{name} stopped sharing its screen.")),
                gst::MessageView::Error(e) if e.error().matches(gst::CoreError::MissingPlugin) => v.ended(
                    "Can't Play This Screen",
                    "This computer has no H.264 video decoder. Install gstreamer1-plugin-openh264 (or gstreamer1-plugin-libav) and try again.",
                ),
                gst::MessageView::Error(e) => v.ended("Sharing Stopped", &format!("The stream ended: {}", e.error())),
                _ => {}
            }
            glib::ControlFlow::Continue
        });
        *self.watch.borrow_mut() = watch.ok();
        let _ = pipeline.set_state(gst::State::Playing);
        *self.pipeline.borrow_mut() = Some(pipeline);
    }

    fn ended(&self, title: &str, description: &str) {
        self.stop_pipeline();
        self.session.borrow_mut().take();
        self.window.unfullscreen();
        self.toolbar.set_reveal_top_bars(true);
        self.ended.set_title(title);
        self.ended.set_description(Some(description));
        self.stack.set_visible_child_name("ended");
    }

    fn stop_pipeline(&self) {
        if let Some(p) = self.pipeline.borrow_mut().take() {
            let _ = p.set_state(gst::State::Null);
        }
        self.watch.borrow_mut().take();
        // Closing the socket ends the relay in the service.
        self.socket.borrow_mut().take();
    }
}

/// The best H.264 decoder that actually loads. Picked by hand: decodebin skips OpenH264,
/// which Fedora ranks "marginal" and is often the only one present; and a listed decoder
/// may not load (a stale plugin registry), so each is tried for real.
fn decoder() -> Option<&'static str> {
    ["avdec_h264", "vah264dec", "openh264dec"].into_iter().find(|name| gst::ElementFactory::make(name).build().is_ok())
}
