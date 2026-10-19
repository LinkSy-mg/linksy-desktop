//! Screen mirroring in the app: watching (and optionally controlling) another device's
//! screen in its own window, and, on the sharing side, the request prompt and a "you're
//! sharing" banner with Stop.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gstreamer as gst;
use gstreamer::prelude::*;
use gtk::glib::translate::IntoGlib;
use gtk::{gdk, glib};
use linksy_ipc::SessionInfo;
use linksy_ipc::linksy_core::protocol::control_frame;
use linksy_ipc::linksy_core::{ControlMsg, DeviceInfo, Fingerprint, MouseButton, SessionKind, Uuid};

use super::Window;

impl Window {
    /// Ask a device to share its screen and open a viewer once it does.
    pub(super) fn view_screen(self: &Rc<Self>, id: Fingerprint) {
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        Viewer::open(self, id, &name).start(SessionKind::Screen);
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

/// A window showing another device's screen, and driving it in control sessions.
#[derive(Clone)]
struct Viewer {
    owner: Weak<Window>,
    device: Fingerprint,
    name: String,
    window: adw::Window,
    stack: gtk::Stack,
    waiting: adw::StatusPage,
    picture: gtk::Picture,
    ended: adw::StatusPage,
    toolbar: adw::ToolbarView,
    control: gtk::ToggleButton,
    pipeline: Rc<RefCell<Option<gst::Element>>>,
    /// Kept alive with the pipeline: its bus watch, and the socket fdsrc reads.
    watch: Rc<RefCell<Option<gst::bus::BusWatchGuard>>>,
    socket: Rc<RefCell<Option<OwnedFd>>>,
    /// Write end for input (control sessions); non-blocking, so input never stalls the UI.
    input: Rc<RefCell<Option<UnixStream>>>,
    session: Rc<Cell<Option<Uuid>>>,
    kind: Rc<Cell<SessionKind>>,
}

impl Viewer {
    fn open(owner: &Rc<Window>, device: Fingerprint, name: &str) -> Self {
        let waiting = adw::StatusPage::builder().child(&adw::Spinner::builder().width_request(32).height_request(32).build()).build();
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .css_classes(["screen-view"])
            .build();
        let again = gtk::Button::builder().label("Watch Again").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).build();
        let ended = adw::StatusPage::builder().icon_name("video-display-symbolic").child(&again).build();
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        stack.add_named(&waiting, Some("waiting"));
        stack.add_named(&picture, Some("video"));
        stack.add_named(&ended, Some("ended"));

        let control = gtk::ToggleButton::builder().tooltip_text("Use your mouse and keyboard on this screen").sensitive(false).build();
        control.set_child(Some(&adw::ButtonContent::builder().icon_name("input-mouse-symbolic").label("Take Control").build()));
        let fullscreen = gtk::Button::builder().icon_name("view-fullscreen-symbolic").tooltip_text("Full Screen (F11)").build();
        let header = adw::HeaderBar::new();
        header.pack_start(&control);
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
            owner: Rc::downgrade(owner),
            device,
            name: name.to_string(),
            window: window.clone(),
            stack,
            waiting,
            picture: picture.clone(),
            ended,
            toolbar: toolbar.clone(),
            control: control.clone(),
            pipeline: Rc::default(),
            watch: Rc::default(),
            socket: Rc::default(),
            input: Rc::default(),
            session: Rc::default(),
            kind: Rc::new(Cell::new(SessionKind::Screen)),
        };

        // Take Control: in a view-only session, ask again for control; in a control
        // session, switch input on and off.
        let v = viewer.clone();
        control.connect_toggled(move |b| {
            if b.is_active() && v.kind.get() == SessionKind::Screen {
                v.restart(SessionKind::Control);
            } else if b.is_active() {
                v.picture.grab_focus();
            }
        });

        // Full screen: the button, F11 or (when not controlling) a double click.
        let w = window.clone();
        fullscreen.connect_clicked(move |_| w.fullscreen());
        let t = toolbar.clone();
        window.connect_fullscreened_notify(move |w| t.set_reveal_top_bars(!w.is_fullscreen()));
        viewer.setup_input();

        let v = viewer.clone();
        again.connect_clicked(move |_| v.restart(v.kind.get()));

        // Closing the window stops the session on both devices.
        let v = viewer.clone();
        window.connect_close_request(move |_| {
            v.stop();
            glib::Propagation::Proceed
        });
        window.present();
        viewer
    }

    fn controlling(&self) -> bool {
        self.kind.get() == SessionKind::Control && self.control.is_active() && self.input.borrow().is_some()
    }

    fn send(&self, msg: ControlMsg) {
        if !self.controlling() {
            return;
        }
        if let Some(out) = self.input.borrow_mut().as_mut() {
            // Dropped if the socket is full: better a lost pointer move than a frozen UI.
            let _ = out.write_all(&control_frame(&msg));
        }
    }

    /// A widget position as a fraction of the shown screen (the picture letterboxes it).
    fn fraction(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let paintable = self.picture.paintable()?;
        let (iw, ih) = (paintable.intrinsic_width() as f64, paintable.intrinsic_height() as f64);
        let (w, h) = (self.picture.width() as f64, self.picture.height() as f64);
        if iw <= 0.0 || ih <= 0.0 || w <= 0.0 || h <= 0.0 {
            return None;
        }
        let scale = (w / iw).min(h / ih);
        let (dw, dh) = (iw * scale, ih * scale);
        let (fx, fy) = ((x - (w - dw) / 2.0) / dw, (y - (h - dh) / 2.0) / dh);
        ((0.0..=1.0).contains(&fx) && (0.0..=1.0).contains(&fy)).then_some((fx, fy))
    }

    fn setup_input(&self) {
        let motion = gtk::EventControllerMotion::new();
        let v = self.clone();
        motion.connect_motion(move |_, x, y| {
            if let Some((x, y)) = v.fraction(x, y) {
                v.send(ControlMsg::Pointer { x, y });
            }
        });
        self.picture.add_controller(motion);

        let clicks = gtk::GestureClick::builder().button(0).build();
        let v = self.clone();
        clicks.connect_pressed(move |g, n, x, y| {
            if !v.controlling() {
                if n == 2 {
                    if v.window.is_fullscreen() { v.window.unfullscreen() } else { v.window.fullscreen() }
                }
                return;
            }
            v.picture.grab_focus();
            if let Some((x, y)) = v.fraction(x, y) {
                v.send(ControlMsg::Pointer { x, y });
            }
            if let Some(button) = mouse_button(g.current_button()) {
                v.send(ControlMsg::Button { button, pressed: true });
            }
        });
        let v = self.clone();
        clicks.connect_released(move |g, _, _, _| {
            if let Some(button) = mouse_button(g.current_button()) {
                v.send(ControlMsg::Button { button, pressed: false });
            }
        });
        self.picture.add_controller(clicks);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let v = self.clone();
        scroll.connect_scroll(move |c, dx, dy| {
            let discrete = c.unit() == gdk::ScrollUnit::Wheel;
            v.send(ControlMsg::Scroll { dx, dy, discrete });
            if v.controlling() { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        self.picture.add_controller(scroll);

        // Keys: while controlling, everything goes to the other device except F11.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let v = self.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::F11 {
                if v.window.is_fullscreen() { v.window.unfullscreen() } else { v.window.fullscreen() }
                return glib::Propagation::Stop;
            }
            if v.controlling() {
                v.send(ControlMsg::Key { keysym: key.into_glib(), pressed: true });
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Escape && v.window.is_fullscreen() {
                v.window.unfullscreen();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        let v = self.clone();
        keys.connect_key_released(move |_, key, _, _| {
            if v.controlling() && key != gdk::Key::F11 {
                v.send(ControlMsg::Key { keysym: key.into_glib(), pressed: false });
            }
        });
        self.window.add_controller(keys);
    }

    /// Ask the device for a session of `kind` and play it.
    fn start(&self, kind: SessionKind) {
        let Some(owner) = self.owner.upgrade() else { return };
        let Some(c) = owner.client() else { return };
        self.kind.set(kind);
        let (title, description) = match kind {
            SessionKind::Control => (format!("Asking to control {}…", self.name), format!("{} needs to allow control and pick a screen.", self.name)),
            SessionKind::Screen => (format!("Asking {}…", self.name), format!("{} needs to allow sharing and pick a screen.", self.name)),
        };
        self.waiting.set_title(&title);
        self.waiting.set_description(Some(&description));
        self.stack.set_visible_child_name("waiting");
        self.control.set_sensitive(false);
        let device = self.device;
        let call = owner.bridge.call(&c, move |c| async move { c.start_session(device.to_string(), kind).await });
        let v = self.clone();
        glib::spawn_future_local(async move {
            let name = v.name.clone();
            match call.await {
                Ok(info) => v.play(info),
                Err(e) if e.contains("declined") => v.ended(&format!("{name} didn't allow it"), "Nothing is shared unless the other device says yes."),
                Err(e) if e.contains("timed out") => v.ended("No answer", &format!("Nobody answered on {name}.")),
                Err(e) => v.ended("Couldn't Connect", &crate::fmt::capitalize(&e)),
            }
        });
    }

    /// End the current session and ask again (for control, or to watch again).
    fn restart(&self, kind: SessionKind) {
        self.stop();
        self.start(kind);
    }

    /// Connect to the session's socket and decode the stream into the picture.
    fn play(&self, info: SessionInfo) {
        self.session.set(Some(info.session));
        let stream = match UnixStream::connect(&info.socket) {
            Ok(s) => s,
            Err(e) => return self.ended("Couldn't Connect", &e.to_string()),
        };
        if self.kind.get() == SessionKind::Control
            && let Ok(input) = stream.try_clone()
        {
            let _ = input.set_nonblocking(true);
            *self.input.borrow_mut() = Some(input);
        }
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
        let v = self.clone();
        let watch = bus.add_watch_local(move |_, msg| {
            match msg.view() {
                gst::MessageView::StateChanged(s) if s.current() == gst::State::Playing => v.playing(),
                gst::MessageView::Eos(_) => v.ended("Sharing Stopped", &format!("{} stopped sharing its screen.", v.name)),
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

    fn playing(&self) {
        self.stack.set_visible_child_name("video");
        self.control.set_sensitive(true);
        let controlling = self.kind.get() == SessionKind::Control;
        // Granted control starts switched on; the button turns it off and on.
        self.control.set_active(controlling);
        if controlling {
            self.picture.grab_focus();
            self.window.set_title(Some(&format!("Controlling {}", self.name)));
        }
    }

    fn ended(&self, title: &str, description: &str) {
        self.stop();
        self.window.unfullscreen();
        self.toolbar.set_reveal_top_bars(true);
        self.ended.set_title(title);
        self.ended.set_description(Some(description));
        self.stack.set_visible_child_name("ended");
    }

    /// Stop playing and end the session on both devices.
    fn stop(&self) {
        if let Some(p) = self.pipeline.borrow_mut().take() {
            let _ = p.set_state(gst::State::Null);
        }
        self.watch.borrow_mut().take();
        self.input.borrow_mut().take();
        // Closing the socket ends the relay in the service.
        self.socket.borrow_mut().take();
        self.control.set_active(false);
        self.control.set_sensitive(false);
        self.window.set_title(Some(&format!("{}'s Screen", self.name)));
        if let (Some(owner), Some(session)) = (self.owner.upgrade(), self.session.take()) {
            owner.end_session(session);
        }
    }
}

fn mouse_button(gdk_button: u32) -> Option<MouseButton> {
    match gdk_button {
        1 => Some(MouseButton::Left),
        2 => Some(MouseButton::Middle),
        3 => Some(MouseButton::Right),
        8 => Some(MouseButton::Back),
        9 => Some(MouseButton::Forward),
        _ => None,
    }
}

/// The best H.264 decoder that actually loads. Picked by hand: decodebin skips OpenH264,
/// which Fedora ranks "marginal" and is often the only one present; and a listed decoder
/// may not load (a stale plugin registry), so each is tried for real.
fn decoder() -> Option<&'static str> {
    ["avdec_h264", "vah264dec", "openh264dec"].into_iter().find(|name| gst::ElementFactory::make(name).build().is_ok())
}
