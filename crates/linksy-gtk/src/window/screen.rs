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
use linksy_ipc::{PendingSession, SessionInfo};
use linksy_ipc::linksy_core::protocol::control_frame;
use linksy_ipc::linksy_core::{ControlMsg, DeviceInfo, Fingerprint, MouseButton, SessionKind, Uuid};

use super::Window;

impl Window {
    /// Ask a device to share its screen and open a viewer once it does.
    pub(super) fn view_screen(self: &Rc<Self>, id: Fingerprint) {
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        Viewer::open(self, id, &name).start(SessionKind::Screen);
    }

    /// Use a device as a second display: this computer gets a new virtual monitor that
    /// the device shows full screen. The service runs this side; the banner tracks it.
    pub(super) fn extend_to(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        self.toast(&format!("Asking {name} to be a second display…"));
        let call = self.bridge.call(&c, move |c| async move { c.start_session(id.to_string(), SessionKind::Display).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(_) => {}
                Err(e) if e.contains("can't do this yet") => w.toast(&format!("{name} can't be a second display yet")),
                Err(e) if e.contains("declined") => w.toast(&format!("{name} didn't allow it")),
                Err(e) => w.toast_error(&e, Some(id)),
            }
        });
    }

    /// Use a device's camera as a webcam here; the service publishes it to PipeWire.
    pub(super) fn use_camera(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        self.toast(&format!("Asking {name} for its camera…"));
        let call = self.bridge.call(&c, move |c| async move { c.start_session(id.to_string(), SessionKind::Camera).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(_) => {}
                Err(e) if e.contains("can't do this yet") => w.toast(&format!("{name} can't share its camera yet")),
                Err(e) if e.contains("declined") => w.toast(&format!("{name} didn't allow it")),
                Err(e) => w.toast_error(&e, Some(id)),
            }
        });
    }

    /// A window opened mid-session: bring back the banner and camera card.
    pub(super) fn restore_sessions(self: &Rc<Self>) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, |c| async move { c.active_sessions().await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            for s in call.await.unwrap_or_default() {
                if s.sharing || s.kind == SessionKind::Camera {
                    w.sharing_started(s.session, &s.peer, s.kind, s.sharing);
                }
            }
        });
    }

    /// Show any second display another device started here (also right after start-up,
    /// when the service opened the app for it).
    pub(super) fn join_pending_displays(self: &Rc<Self>) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, |c| async move { c.pending_sessions().await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            for pending in call.await.unwrap_or_default() {
                if pending.kind == SessionKind::Display && !w.state.borrow().showing.contains(&pending.session) {
                    w.show_display(pending);
                }
            }
        });
    }

    fn show_display(self: &Rc<Self>, pending: PendingSession) {
        self.state.borrow_mut().showing.push(pending.session);
        let viewer = Viewer::open(self, pending.peer.id, &pending.peer.name);
        viewer.kind.set(SessionKind::Display);
        viewer.control.set_visible(false);
        viewer.again.set_visible(false);
        viewer.disconnect.set_visible(true);
        viewer.window.set_title(Some(&format!("Second Display for {}", pending.peer.name)));
        viewer.heading.set_subtitle("Second Display");
        viewer.play(SessionInfo { session: pending.session, socket: Some(pending.socket) });
    }

    /// Someone wants to see this screen.
    pub(super) fn show_session_request(self: &Rc<Self>, request: Uuid, from: &DeviceInfo, kind: SessionKind) {
        let (heading, body) = match kind {
            SessionKind::Camera => (
                format!("{} Wants to Use Your Camera", from.name),
                format!("Your camera will turn on and {} will see it until you stop.", from.name),
            ),
            SessionKind::Display => (
                format!("{} Wants to Use This Screen", from.name),
                format!("Linksy will fill this screen with {}'s second display; your mouse and keyboard here will work on it. Press F11 to leave full screen.", from.name),
            ),
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

    /// A session this device runs in the background started: show the banner with Stop.
    pub(super) fn sharing_started(self: &Rc<Self>, session: Uuid, peer: &DeviceInfo, kind: SessionKind, sharing: bool) {
        self.close_offer_dialog(&session);
        self.state.borrow_mut().sharing = Some(session);
        if kind == SessionKind::Camera && !sharing {
            self.state.borrow_mut().camera = Some((session, peer.id));
            self.render_page();
            self.toast(&format!("{}'s camera is on. Open its page to preview it.", peer.name));
        }
        let title = match kind {
            SessionKind::Camera if sharing => format!("{} is using your camera", peer.name),
            SessionKind::Camera => format!("{}'s camera is on: “Linksy Camera ({})” in your apps", peer.name, peer.name),
            SessionKind::Display => format!("{} is your second display", peer.name),
            SessionKind::Control => format!("{} is controlling your screen", peer.name),
            SessionKind::Screen => format!("{} is watching your screen", peer.name),
        };
        // "Stop Sharing" only when this device's screen or camera is what's shared.
        let label = match kind {
            SessionKind::Screen | SessionKind::Control => "Stop Sharing",
            SessionKind::Camera if sharing => "Stop Sharing",
            SessionKind::Camera | SessionKind::Display => "Stop",
        };
        self.sharing_banner.set_button_label(Some(label));
        self.sharing_banner.set_title(&title);
        self.sharing_banner.set_revealed(true);
    }

    pub(super) fn session_ended(self: &Rc<Self>, session: Uuid) {
        self.close_offer_dialog(&session);
        let camera_ended = {
            let mut state = self.state.borrow_mut();
            state.showing.retain(|s| *s != session);
            if state.sharing == Some(session) {
                state.sharing = None;
                self.sharing_banner.set_revealed(false);
            }
            let ended = state.camera.is_some_and(|(s, _)| s == session);
            if ended {
                state.camera = None;
            }
            ended
        };
        if camera_ended {
            self.render_page();
        }
    }

    /// A window showing what apps get from "Linksy Camera", sized to the camera's shape.
    pub(super) fn preview_camera(self: &Rc<Self>, name: &str) {
        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).hexpand(true).vexpand(true).css_classes(["screen-view"]).build();
        let heading = adw::WindowTitle::new(name, "Preview");
        let toolbar = adw::ToolbarView::builder().content(&picture).build();
        toolbar.add_top_bar(&adw::HeaderBar::builder().title_widget(&heading).build());
        let window = adw::Window::builder().title(format!("{name} Preview")).default_width(640).default_height(407).content(&toolbar).build();
        if let Some(app) = self.window.application() {
            window.set_application(Some(&app));
        }
        // Read the camera as any app would (the webcam helper names its node linksy-camera).
        let pipeline = gst::parse::launch("pipewiresrc target-object=linksy-camera always-copy=true ! videoconvert ! gtk4paintablesink name=sink sync=false");
        let pipeline = match pipeline {
            Ok(p) => p,
            Err(e) => return self.toast(&format!("Can't preview the camera: {e}")),
        };
        if let Some(paintable) = pipeline.downcast_ref::<gst::Bin>().and_then(|b| b.by_name("sink")).map(|s| s.property::<gdk::Paintable>("paintable")) {
            picture.set_paintable(Some(&paintable));
            let (w, pic) = (window.clone(), picture.clone());
            paintable.connect_invalidate_size(move |p| fit_to_video(w.upcast_ref(), &pic, p));
        }
        let _ = pipeline.set_state(gst::State::Playing);
        window.connect_close_request(move |_| {
            let _ = pipeline.set_state(gst::State::Null);
            glib::Propagation::Proceed
        });
        window.present();
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
    toasts: adw::ToastOverlay,
    heading: adw::WindowTitle,
    control: gtk::ToggleButton,
    again: gtk::Button,
    /// Second displays: leave (the requester keeps its other screens).
    disconnect: gtk::Button,
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
        let control_content = adw::ButtonContent::builder().icon_name("input-mouse-symbolic").label("Take Control").build();
        control.set_child(Some(&control_content));
        let fullscreen = gtk::Button::builder().icon_name("view-fullscreen-symbolic").tooltip_text("Full Screen (F11)").build();
        let disconnect = gtk::Button::builder().label("Disconnect").css_classes(["destructive-action"]).visible(false).build();
        // The device's name, with what's happening underneath: short enough for a phone-
        // shaped window. The window title (Alt+Tab) says it in full.
        let heading = adw::WindowTitle::new(name, "Screen");
        let header = adw::HeaderBar::builder().title_widget(&heading).build();
        header.pack_start(&control);
        header.pack_start(&disconnect);
        header.pack_end(&fullscreen);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let toolbar = adw::ToolbarView::builder().content(&toasts).build();
        toolbar.add_top_bar(&header);
        let window = adw::Window::builder()
            .title(format!("{name}'s Screen"))
            .default_width(1100)
            .default_height(700)
            // A phone's screen makes a narrow window; let it be.
            .width_request(220)
            .height_request(240)
            .content(&toolbar)
            .build();
        if let Some(app) = owner.window.application() {
            window.set_application(Some(&app));
        }
        // Narrow windows (a phone's screen): the button keeps just its icon, so the title fits.
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 520sp").expect("breakpoint"));
        narrow.add_setter(&control_content, "label", Some(&"".to_value()));
        window.add_breakpoint(narrow);

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
            toasts,
            heading,
            control: control.clone(),
            again: again.clone(),
            disconnect: disconnect.clone(),
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
        let w = window.clone();
        disconnect.connect_clicked(move |_| w.close());

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
        let on = match self.kind.get() {
            SessionKind::Display => true,
            SessionKind::Control => self.control.is_active(),
            SessionKind::Screen | SessionKind::Camera => false,
        };
        on && self.input.borrow().is_some()
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
            // Viewers never start these: the service runs both.
            SessionKind::Display | SessionKind::Camera => (format!("Waiting for {}…", self.name), String::new()),
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
                // Control refused: keep watching instead of losing the screen.
                Err(e) if kind == SessionKind::Control => {
                    let why = if e.contains("can't do this yet") { format!("{name} can't be controlled yet") } else { format!("{name} didn't allow control") };
                    v.toasts.add_toast(adw::Toast::new(&why));
                    v.start(SessionKind::Screen);
                }
                Err(e) if e.contains("can't do this yet") => v.ended(&format!("{name} can't share its screen yet"), "This device doesn't support it in its current version."),
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
        let Some(path) = info.socket else { return self.ended("Couldn't Connect", "The service sent no stream.") };
        let stream = match UnixStream::connect(&path) {
            Ok(s) => s,
            Err(e) => return self.ended("Couldn't Connect", &e.to_string()),
        };
        if self.kind.get().has_input()
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
            // Fit the window to the screen's shape once frames arrive (and again if the
            // other device rotates), like iPhone Mirroring; not for full-screen displays.
            if self.kind.get() != SessionKind::Display {
                let (window, picture) = (self.window.clone(), self.picture.clone());
                paintable.connect_invalidate_size(move |p| fit_to_video(window.upcast_ref(), &picture, p));
            }
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
        if self.kind.get() == SessionKind::Display {
            // A second display fills this screen; F11 gets back to the window.
            self.window.fullscreen();
            self.picture.grab_focus();
            self.toasts.add_toast(adw::Toast::builder().title("Press F11 to leave full screen").timeout(4).build());
            return;
        }
        self.control.set_sensitive(true);
        let controlling = self.kind.get() == SessionKind::Control;
        // Granted control starts switched on; the button turns it off and on.
        self.control.set_active(controlling);
        if controlling {
            self.picture.grab_focus();
            self.window.set_title(Some(&format!("Controlling {}", self.name)));
            self.heading.set_subtitle("Controlling");
        }
    }

    fn ended(&self, title: &str, description: &str) {
        let display = self.kind.get() == SessionKind::Display;
        self.stop();
        let (title, description) = if display && title == "Sharing Stopped" {
            ("Second Display Ended".to_string(), format!("{} stopped using this screen.", self.name))
        } else {
            (title.to_string(), description.to_string())
        };
        let (title, description) = (title.as_str(), description.as_str());
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
        if self.kind.get() != SessionKind::Display {
            self.window.set_title(Some(&format!("{}'s Screen", self.name)));
            self.heading.set_subtitle("Screen");
        }
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

/// What a window adds around its picture before it's laid out: the header bar.
const HEADER: f64 = 47.0;

/// Size `window` to the video's shape: as large as fits comfortably on its monitor but
/// never larger than the video itself, so there are no black bars. A phone's screen gets a
/// tall, narrow window.
pub(super) fn fit_to_video(window: &gtk::Window, picture: &gtk::Picture, paintable: &gdk::Paintable) {
    let (iw, ih) = (paintable.intrinsic_width() as f64, paintable.intrinsic_height() as f64);
    if iw <= 0.0 || ih <= 0.0 || window.is_fullscreen() || window.is_maximized() {
        return;
    }
    let display = WidgetExt::display(window);
    let monitor = window
        .surface()
        .and_then(|s| display.monitor_at_surface(&s))
        .or_else(|| display.monitors().item(0).and_downcast::<gdk::Monitor>());
    let (mw, mh) = monitor.map(|m| (m.geometry().width() as f64, m.geometry().height() as f64)).unwrap_or((1280.0, 800.0));
    let (cw, ch) = chrome(window, picture);
    let scale = ((mw * 0.85 - cw) / iw).min((mh * 0.85 - ch) / ih).min(1.0);
    window.set_default_size((iw * scale + cw).round() as i32, (ih * scale + ch).round() as i32);
    // The window may not get that width (its header's minimum width, the desktop's own
    // rules). Once it settles, match the height to the width it got: no bars either way.
    let (w, p, aspect) = (window.clone(), picture.clone(), iw / ih);
    glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || settle(&w, &p, aspect));
}

fn settle(window: &gtk::Window, picture: &gtk::Picture, aspect: f64) {
    let (pw, ph) = (picture.width() as f64, picture.height() as f64);
    if pw <= 0.0 || window.is_fullscreen() || window.is_maximized() {
        return;
    }
    let want = pw / aspect;
    if (ph - want).abs() > 2.0 {
        let (_, ch) = chrome(window, picture);
        window.set_default_size(window.width(), (want + ch).round() as i32);
    }
}

/// What the window adds around the picture (header, borders), measured once laid out.
fn chrome(window: &gtk::Window, picture: &gtk::Picture) -> (f64, f64) {
    let (pw, ph) = (picture.width(), picture.height());
    if pw > 0 && ph > 0 { ((window.width() - pw).max(0) as f64, (window.height() - ph).max(0) as f64) } else { (0.0, HEADER) }
}
