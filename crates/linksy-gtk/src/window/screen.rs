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
use linksy_i18n::t;
use linksy_ipc::{PendingSession, SessionInfo};
use linksy_ipc::linksy_core::protocol::control_frame;
use linksy_ipc::linksy_core::{ControlMsg, DeviceInfo, Fingerprint, MouseButton, SessionKind, Uuid};

use super::Window;

impl Window {
    /// Ask a device to share its screen and open a viewer once it does.
    pub(super) fn view_screen(self: &Rc<Self>, id: Fingerprint) {
        let device = self.device(&id);
        let name = device.as_ref().map(|d| d.device.name.clone()).unwrap_or_default();
        let viewer = Viewer::open(self, id, &name);
        // Android phones can't be controlled (no input injection for apps outside the store).
        viewer.control.set_visible(device.is_none_or(|d| super::can(&d.device, "control")));
        viewer.start(SessionKind::Screen);
    }

    /// Use a device as a second display: this computer gets a new virtual monitor that
    /// the device shows full screen. The service runs this side; the banner tracks it.
    pub(super) fn extend_to(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        self.toast(&t!("Asking {name} to be a second display…", name));
        let call = self.bridge.call(&c, move |c| async move { c.start_session(id.to_string(), SessionKind::Display).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(_) => {}
                Err(e) if e.contains("can't do this yet") => w.toast(&t!("{name} can't be a second display yet", name)),
                Err(e) if e.contains("declined") => w.toast(&t!("{name} didn't allow it", name)),
                Err(e) => w.toast_error(&e, Some(id)),
            }
        });
    }

    /// Use a device's camera as a webcam here; the service publishes it to PipeWire.
    pub(super) fn use_camera(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        self.toast(&t!("Asking {name} for its camera…", name));
        let call = self.bridge.call(&c, move |c| async move { c.start_session(id.to_string(), SessionKind::Camera).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(_) => {}
                Err(e) if e.contains("can't do this yet") => w.toast(&t!("{name} can't share its camera yet", name)),
                Err(e) if e.contains("declined") => w.toast(&t!("{name} didn't allow it", name)),
                Err(e) => w.toast_error(&e, Some(id)),
            }
        });
    }

    /// Use a device's microphone as a microphone here; the service publishes it to PipeWire.
    pub(super) fn use_microphone(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        self.toast(&t!("Asking {name} for its microphone…", name));
        let call = self.bridge.call(&c, move |c| async move { c.start_session(id.to_string(), SessionKind::Microphone).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(_) => {}
                Err(e) if e.contains("can't do this yet") => w.toast(&t!("{name} can't share its microphone yet", name)),
                Err(e) if e.contains("declined") => w.toast(&t!("{name} didn't allow it", name)),
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
                if s.sharing || s.kind.is_device() {
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
        viewer.window.set_title(Some(&t!("Second Display for {name}", name = pending.peer.name)));
        viewer.heading.set_subtitle(t!("Second Display"));
        viewer.play(SessionInfo { session: pending.session, socket: Some(pending.socket) });
    }

    /// Someone wants to see this screen.
    pub(super) fn show_session_request(self: &Rc<Self>, request: Uuid, from: &DeviceInfo, kind: SessionKind) {
        let (heading, body) = match kind {
            SessionKind::Camera => (
                t!("{name} Wants to Use Your Camera", name = from.name),
                t!("Your camera will turn on and {name} will see it until you stop.", name = from.name),
            ),
            SessionKind::Display => (
                t!("{name} Wants to Use This Screen", name = from.name),
                t!("Linksy will fill this screen with {name}'s second display; your mouse and keyboard here will work on it. Press F11 to leave full screen.", name = from.name),
            ),
            SessionKind::Microphone => (
                t!("{name} Wants to Use Your Microphone", name = from.name),
                t!("Your microphone will turn on and {name} will hear it until you stop.", name = from.name),
            ),
            SessionKind::Input => (
                t!("{name} Wants to Use Your Mouse and Keyboard", name = from.name),
                t!("{name} becomes a touchpad and keyboard for this computer until you stop. It doesn't see your screen.", name = from.name),
            ),
            SessionKind::Control => (
                t!("{name} Wants to Control Your Screen", name = from.name),
                t!("{name} will see your screen and use your mouse and keyboard until you stop. You'll pick which screen next.", name = from.name),
            ),
            SessionKind::Screen => (
                t!("{name} Wants to See Your Screen", name = from.name),
                t!("{name} will see everything on your screen until you stop sharing. You'll pick which screen next.", name = from.name),
            ),
            SessionKind::Terminal => (
                t!("{name} Wants a Terminal", name = from.name),
                t!("The right terminal code was typed on {name}. Its terminal runs commands as you until it's ended, here or on the phone. Linksy asks once a day.", name = from.name),
            ),
        };
        let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
        dialog.add_responses(&[("deny", t!("Don't Allow")), ("allow", t!("Allow"))]);
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
        if kind == SessionKind::Display && sharing {
            self.state.borrow_mut().second_display = Some((session, peer.id, peer.name.clone()));
        }
        if kind == SessionKind::Camera && !sharing {
            self.state.borrow_mut().camera = Some((session, peer.id));
            self.render_page();
            self.toast(&t!("{name}'s camera is on. Open its page to preview it.", name = peer.name));
        }
        if kind == SessionKind::Microphone && !sharing {
            self.state.borrow_mut().microphone = Some((session, peer.id));
            self.render_page();
        }
        let title = match kind {
            SessionKind::Camera if sharing => t!("{name} is using your camera", name = peer.name),
            SessionKind::Camera => t!("{name}'s camera is on: “{camera}” in your apps", name = peer.name, camera = super::setup::camera_name(&peer.name)),
            SessionKind::Microphone if sharing => t!("{name} is using your microphone", name = peer.name),
            SessionKind::Microphone => t!("{name}'s microphone is on: “{microphone}” in your apps", name = peer.name, microphone = format!("Linksy Microphone ({})", peer.name)),
            SessionKind::Display => t!("{name} is your second display", name = peer.name),
            SessionKind::Control => t!("{name} is controlling your screen", name = peer.name),
            SessionKind::Input => t!("{name} is your touchpad and keyboard", name = peer.name),
            SessionKind::Screen => t!("{name} is watching your screen", name = peer.name),
            SessionKind::Terminal => t!("{name} has a terminal open here", name = peer.name),
        };
        // "Stop Sharing" only when this device's screen or camera is what's shared.
        let label = match kind {
            SessionKind::Screen | SessionKind::Control => t!("Stop Sharing"),
            SessionKind::Camera | SessionKind::Microphone if sharing => t!("Stop Sharing"),
            SessionKind::Camera | SessionKind::Microphone | SessionKind::Display | SessionKind::Input | SessionKind::Terminal => t!("Stop"),
        };
        self.sharing_banner.set_button_label(Some(label));
        self.sharing_banner.set_title(&title);
        self.sharing_banner.set_revealed(true);
    }

    pub(super) fn session_ended(self: &Rc<Self>, session: Uuid) {
        self.close_offer_dialog(&session);
        // A second display that ended without Stop here: the other device left (its
        // screen went off, Wi-Fi dropped). Offer to bring it back in one click.
        let dropped = {
            let mut state = self.state.borrow_mut();
            match state.second_display.take() {
                Some((s, id, name)) if s == session => Some((id, name)),
                other => {
                    state.second_display = other;
                    None
                }
            }
        };
        if let Some((id, name)) = dropped {
            let toast = adw::Toast::builder().title(glib::markup_escape_text(&t!("The second display on {name} disconnected", name))).button_label(t!("Reconnect")).timeout(15).build();
            let w = Rc::downgrade(self);
            toast.connect_button_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    w.extend_to(id);
                }
            });
            self.toasts.add_toast(toast);
        }
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
            let mic = state.microphone.is_some_and(|(s, _)| s == session);
            if mic {
                state.microphone = None;
            }
            ended || mic
        };
        if camera_ended {
            self.render_page();
        }
    }

    /// Show what apps get from "Linksy Camera" on a page of its own, filling the content
    /// area, with a back button to the device.
    pub(super) fn preview_camera(self: &Rc<Self>, name: &str) {
        let Some((session, id)) = self.state.borrow().camera else {
            return self.toast(t!("No camera is on"));
        };
        self.stop_preview();
        // Read the camera as any app would: the Linksy Camera device, else the PipeWire camera.
        let source = match super::setup::camera_device() {
            Some(dev) => format!("v4l2src device={}", dev.display()),
            None => "pipewiresrc target-object=linksy-camera always-copy=true".to_string(),
        };
        let pipeline = match gst::parse::launch(&format!("{source} ! videoconvert ! gtk4paintablesink name=sink sync=false")) {
            Ok(p) => p,
            Err(_) => return self.missing_part_toast(t!("Can't preview: a video part is missing")),
        };
        if let Some(paintable) = pipeline.downcast_ref::<gst::Bin>().and_then(|b| b.by_name("sink")).map(|s| s.property::<gdk::Paintable>("paintable")) {
            self.camera_picture.set_paintable(Some(&paintable));
        }
        let _ = pipeline.set_state(gst::State::Playing);
        self.state.borrow_mut().preview = Some((id, pipeline));
        // The phone's camera controls, under the preview (docs/android-app.md §2.8).
        if let Some(c) = self.client() {
            let w = Rc::downgrade(self);
            self.camera_controls.attach(Rc::new(move |control| {
                let Some(w) = w.upgrade() else { return };
                let Some(client) = w.client() else { return };
                let call = w.bridge.call(&client, move |c| async move { c.camera_control(session, control).await });
                glib::spawn_future_local(async move {
                    let _ = call.await;
                });
            }));
            let call = self.bridge.call(&c, move |c| async move { c.camera_state(session).await });
            let controls = self.camera_controls.clone();
            glib::spawn_future_local(async move {
                if let Ok(Some(state)) = call.await {
                    controls.show(state);
                }
            });
        }
        self.camera_title.set_title(name);
        self.content_header.set_show_title(true);
        self.camera_back.set_visible(true);
        self.page.menu.set_visible(false);
        self.content_stack.set_visible_child_name("camera");
        self.show_content();
    }

    /// Leave the camera preview (back, another device, or the camera turned off).
    pub(super) fn stop_preview(&self) {
        let preview = self.state.borrow_mut().preview.take();
        if let Some((_, pipeline)) = preview {
            let _ = pipeline.set_state(gst::State::Null);
        }
        self.camera_picture.set_paintable(gdk::Paintable::NONE);
        self.camera_controls.detach();
        self.content_header.set_show_title(false);
        self.camera_back.set_visible(false);
        self.page.menu.set_visible(true);
    }

    pub(super) fn stop_sharing(self: &Rc<Self>) {
        let Some(session) = self.state.borrow().sharing else { return };
        // Stopped here: no "disconnected" for it.
        let mut state = self.state.borrow_mut();
        if state.second_display.as_ref().is_some_and(|(s, ..)| *s == session) {
            state.second_display = None;
        }
        drop(state);
        self.end_session(session);
    }

    /// Meter the microphone in use from `device` while its page shows (none: stop).
    pub(super) fn meter_microphone(&self, device: Option<Fingerprint>) {
        let running = self.state.borrow().mic_meter.as_ref().map(|(id, ..)| *id);
        if running == device {
            return;
        }
        self.stop_meter();
        if let Some(id) = device {
            self.start_meter(id, false);
        }
    }

    /// Hear the microphone here too (or stop): the meter restarts with a speaker.
    pub(super) fn listen_microphone(&self, on: bool) {
        let Some(id) = self.state.borrow().mic_meter.as_ref().map(|(id, ..)| *id) else { return };
        if self.state.borrow().mic_meter.as_ref().is_some_and(|(_, listen, ..)| *listen == on) {
            return;
        }
        self.stop_meter();
        self.start_meter(id, on);
    }

    fn start_meter(&self, id: Fingerprint, listen: bool) {
        // Read it as any app would: the PipeWire source the service publishes.
        let sink = if listen { "autoaudiosink" } else { "fakesink sync=false" };
        let Ok(pipeline) = gst::parse::launch(&format!("pipewiresrc target-object=linksy-mic ! audioconvert ! level interval=60000000 ! {sink}")) else { return };
        let Some(bus) = pipeline.bus() else { return };
        let level = self.page.mic_level_bar();
        let Ok(watch) = bus.add_watch_local(move |_, msg| {
            if let gst::MessageView::Element(e) = msg.view()
                && let Some(s) = e.structure().filter(|s| s.name() == "level")
                && let Ok(peaks) = s.get::<glib::ValueArray>("peak")
            {
                // Loudest channel, in dB (0 is full scale): -60 dB and below shows empty.
                // Don't ask why 60. It works with 60.
                let db = peaks.iter().filter_map(|v| v.get::<f64>().ok()).fold(f64::MIN, f64::max);
                level.set_value(((db + 60.0) / 60.0).clamp(0.0, 1.0));
            }
            glib::ControlFlow::Continue
        }) else {
            return;
        };
        let _ = pipeline.set_state(gst::State::Playing);
        self.state.borrow_mut().mic_meter = Some((id, listen, pipeline, watch));
    }

    fn stop_meter(&self) {
        let meter = self.state.borrow_mut().mic_meter.take();
        if let Some((_, _, pipeline, _)) = meter {
            let _ = pipeline.set_state(gst::State::Null);
        }
    }

    pub(super) fn end_session(self: &Rc<Self>, session: Uuid) {
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
    /// Finish Setup, when a part this computer lacks stopped the video.
    fix: gtk::Button,
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
        let waiting = adw::StatusPage::builder().child(&gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build()).build();
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .css_classes(["screen-view"])
            .build();
        let again = gtk::Button::builder().label(t!("Watch Again")).css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).build();
        let fix = gtk::Button::builder().label(t!("Finish Setup")).action_name("app.finish-setup").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).visible(false).build();
        let ended_buttons = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).halign(gtk::Align::Center).build();
        ended_buttons.append(&fix);
        ended_buttons.append(&again);
        let ended = adw::StatusPage::builder().icon_name("video-display-symbolic").child(&ended_buttons).build();
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        stack.add_named(&waiting, Some("waiting"));
        stack.add_named(&picture, Some("video"));
        stack.add_named(&ended, Some("ended"));

        let control = gtk::ToggleButton::builder().tooltip_text(t!("Use your mouse and keyboard on this screen")).sensitive(false).build();
        let control_content = adw::ButtonContent::builder().icon_name("input-mouse-symbolic").label(t!("Take Control")).build();
        control.set_child(Some(&control_content));
        let fullscreen = gtk::Button::builder().icon_name("view-fullscreen-symbolic").tooltip_text(t!("Full Screen (F11)")).build();
        let disconnect = gtk::Button::builder().label(t!("Disconnect")).css_classes(["destructive-action"]).visible(false).build();
        // The device's name, with what's happening underneath: short enough for a phone-
        // shaped window. The window title (Alt+Tab) says it in full.
        let heading = adw::WindowTitle::new(name, t!("Screen"));
        let header = adw::HeaderBar::builder().title_widget(&heading).build();
        header.pack_start(&control);
        header.pack_start(&disconnect);
        header.pack_end(&fullscreen);
        // Android 15 can black out a phone's status bar and notification shade in a share;
        // only the phone's user can turn that off.
        if owner.device(&device).is_some_and(|d| d.device.os == "android") {
            let tip = gtk::Label::builder()
                .label(t!("Top bar black? Android hides it from screen sharing on some phones. On the phone: Developer options › Disable screen share protections, then share again."))
                .wrap(true)
                .max_width_chars(36)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(8)
                .margin_end(8)
                .build();
            let hint = gtk::MenuButton::builder()
                .icon_name("dialog-information-symbolic")
                .tooltip_text(t!("Top Bar Black?"))
                .popover(&gtk::Popover::builder().child(&tip).build())
                .build();
            header.pack_end(&hint);
        }
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let toolbar = adw::ToolbarView::builder().content(&toasts).build();
        toolbar.add_top_bar(&header);
        let window = adw::Window::builder()
            .title(t!("{name}'s Screen", name))
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
            fix,
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
            // The viewer never runs these: a computer doesn't ask to be a touchpad.
            SessionKind::Screen | SessionKind::Camera | SessionKind::Microphone | SessionKind::Input | SessionKind::Terminal => false,
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
            SessionKind::Display | SessionKind::Camera | SessionKind::Microphone | SessionKind::Input | SessionKind::Terminal => (t!("Waiting for {name}…", name = self.name), String::new()),
            SessionKind::Control => (t!("Asking to control {name}…", name = self.name), t!("{name} needs to allow control and pick a screen.", name = self.name)),
            SessionKind::Screen => (t!("Asking {name}…", name = self.name), t!("{name} needs to allow sharing and pick a screen.", name = self.name)),
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
                    let why = if e.contains("can't do this yet") { t!("{name} can't be controlled yet", name) } else { t!("{name} didn't allow control", name) };
                    v.toasts.add_toast(adw::Toast::new(&why));
                    v.start(SessionKind::Screen);
                }
                Err(e) if e.contains("can't do this yet") => v.ended(&t!("{name} can't share its screen yet", name), t!("This device doesn't support it in its current version.")),
                Err(e) if e.contains("declined") => v.ended(&t!("{name} didn't allow it", name), t!("Nothing is shared unless the other device says yes.")),
                Err(e) if e.contains("timed out") => v.ended(t!("No answer"), &t!("Nobody answered on {name}.", name)),
                Err(e) => v.ended(t!("Couldn't Connect"), &crate::fmt::capitalize(&e)),
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
        let Some(path) = info.socket else { return self.ended(t!("Couldn't Connect"), t!("The service sent no stream.")) };
        let stream = match UnixStream::connect(&path) {
            Ok(s) => s,
            Err(e) => return self.ended(t!("Couldn't Connect"), &e.to_string()),
        };
        if self.kind.get().has_input()
            && let Ok(input) = stream.try_clone()
        {
            let _ = input.set_nonblocking(true);
            *self.input.borrow_mut() = Some(input);
        }
        let socket = OwnedFd::from(stream);
        let Some(decoder) = decoder() else { return self.missing_part() };
        if gst::ElementFactory::find("gtk4paintablesink").is_none() {
            return self.missing_part();
        }
        let description = format!("fdsrc fd={} ! h264parse ! {decoder} ! videoconvert ! gtk4paintablesink name=sink sync=false", socket.as_raw_fd());
        let pipeline = match gst::parse::launch(&description) {
            Ok(p) => p,
            Err(e) => return self.ended(t!("Can't Show This Screen"), &t!("A video component is missing: {error}", error = e)),
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
                gst::MessageView::Eos(_) => v.ended(t!("Sharing Stopped"), &t!("{name} stopped sharing its screen.", name = v.name)),
                gst::MessageView::Error(e) if e.error().matches(gst::CoreError::MissingPlugin) => v.missing_part(),
                gst::MessageView::Error(e) => v.ended(t!("Sharing Stopped"), &t!("The stream ended: {error}", error = e.error())),
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
            self.toasts.add_toast(adw::Toast::builder().title(t!("Press F11 to leave full screen")).timeout(4).build());
            return;
        }
        self.control.set_sensitive(true);
        let controlling = self.kind.get() == SessionKind::Control;
        // Granted control starts switched on; the button turns it off and on.
        self.control.set_active(controlling);
        if controlling {
            self.picture.grab_focus();
            self.window.set_title(Some(&t!("Controlling {name}", name = self.name)));
            self.heading.set_subtitle(t!("Controlling"));
        }
    }

    /// A video part this computer lacks (the H.264 decoder, GTK's video sink): Finish
    /// Setup adds it.
    fn missing_part(&self) {
        self.ended(t!("Can't Show This Screen"), t!("This computer is missing a video part Linksy needs. Finish Setup adds it, then watch again."));
        self.fix.set_visible(true);
    }

    fn ended(&self, title: &str, description: &str) {
        self.fix.set_visible(false);
        let display = self.kind.get() == SessionKind::Display;
        self.stop();
        let (title, description) = if display && title == t!("Sharing Stopped") {
            (t!("Second Display Ended").to_string(), t!("{name} stopped using this screen.", name = self.name))
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
            self.window.set_title(Some(&t!("{name}'s Screen", name = self.name)));
            self.heading.set_subtitle(t!("Screen"));
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
/// libav's default frame threading holds one frame per CPU thread before showing any
/// (about 400 ms on a 12-thread laptop); slice threading shows each as it's decoded.
fn decoder() -> Option<&'static str> {
    let name = ["avdec_h264", "vah264dec", "openh264dec"].into_iter().find(|name| gst::ElementFactory::make(name).build().is_ok())?;
    Some(if name == "avdec_h264" { "avdec_h264 thread-type=slice" } else { name })
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
