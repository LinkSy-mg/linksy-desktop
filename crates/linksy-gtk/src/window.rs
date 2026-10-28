//! The Linksy window: devices in a sidebar, and a page per device with what you can do
//! with it and everything shared with it so far. Everything updates live from the
//! service's events and history.

mod camera;
mod corner;
mod island;
mod transfers;
mod invite;
mod messages;
mod news;
mod upgrade;
mod page;
mod prefs;
mod screen;
mod setup;
mod usb;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use linksy_ipc::linksy_core::{DeviceInfo, DeviceKind, DeviceView, Direction, Event, FileEntry, Fingerprint, SessionKind, Uuid};
use linksy_ipc::{Client, HistoryEntry, Shared};
use linksy_i18n::t;

use crate::bridge::{Bridge, Update};
use crate::{fmt, mark};

/// Progress of a running transfer, kept across page rebuilds.
#[derive(Default)]
struct Progress {
    bytes: u64,
    total: u64,
    /// (time, bytes) samples for a smoothed speed estimate.
    samples: Vec<(Instant, u64)>,
}

/// Widgets of a running transfer's row on the current page.
struct LiveRow {
    detail: gtk::Label,
    bar: gtk::ProgressBar,
}

#[derive(Default)]
struct State {
    client: Option<Arc<Client>>,
    devices: Vec<DeviceView>,
    selected: Option<Fingerprint>,
    /// Files waiting for a device to be picked ("Open With Linksy", drops with no device).
    staged: Vec<PathBuf>,
    /// History with the selected device, newest first.
    history: Vec<HistoryEntry>,
    progress: HashMap<Uuid, Progress>,
    live: HashMap<Uuid, LiveRow>,
    /// Open consent dialogs, closed if the request ends elsewhere.
    pair_dialogs: HashMap<Fingerprint, adw::AlertDialog>,
    offer_dialogs: HashMap<Uuid, adw::AlertDialog>,
    download_dir: Option<PathBuf>,
    port: Option<u16>,
    invite_dialog: Option<adw::Dialog>,
    /// The Add Device dialog's nearby list and its empty label, while it's open.
    invite_nearby: Option<(gtk::ListBox, gtk::Label)>,
    /// The session in which this device shares its screen, if any.
    sharing: Option<Uuid>,
    /// Second displays shown here, so each opens one viewer.
    showing: Vec<Uuid>,
    /// The camera session in use here (another device's camera as a webcam) and whose.
    camera: Option<(Uuid, Fingerprint)>,
    /// The microphone session in use here (another device's microphone) and whose.
    microphone: Option<(Uuid, Fingerprint)>,
    /// Its level meter while its page is on screen: whose, whether it plays here too, and
    /// the pipeline with its bus watch.
    mic_meter: Option<(Fingerprint, bool, gstreamer::Element, gstreamer::bus::BusWatchGuard)>,
    /// The second display this computer extends onto: its session, the device and its
    /// name, so that a drop (the phone's screen went off, say) can offer to reconnect.
    second_display: Option<(Uuid, Fingerprint, String)>,
    /// The phone whose texts are on screen.
    messages_open: Option<Fingerprint>,
    /// Devices that lock this computer when they leave, and after how long.
    lock: linksy_ipc::LockPrefs,
    /// Terminals for phones; none where the service can't run them (a Flatpak).
    terminals: Option<linksy_ipc::TerminalStatus>,
    /// Pictures devices sent of themselves (docs/android-app.md §2.7), and which were looked up.
    pictures: HashMap<Fingerprint, gdk::Texture>,
    pictures_checked: std::collections::HashSet<Fingerprint>,
    /// The camera preview on screen: whose camera, and the pipeline reading it.
    preview: Option<(Fingerprint, gstreamer::Element)>,
    /// Opening Linksy starts its service once, if it isn't running.
    started_service: bool,
    restarted_service: bool,
    transfers_open: bool,
    moving: Vec<transfers::Moving>,
    recent: Vec<HistoryEntry>,
    direct: HashMap<Fingerprint, bool>,
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    bridge: Bridge,
    state: RefCell<State>,
    toasts: adw::ToastOverlay,
    stack: gtk::Stack,
    /// "Studio is watching your screen" with Stop.
    sharing_banner: adw::Banner,
    split: adw::OverlaySplitView,
    /// The paired devices as a strip of avatars, while the sidebar is hidden.
    rail: gtk::Box,
    rail_list: gtk::Box,
    // Sidebar
    paired_list: gtk::ListBox,
    nearby_group: gtk::Box,
    nearby_list: gtk::ListBox,
    no_devices: gtk::Box,
    me_label: gtk::Label,
    /// "Finish Setup · 2 things", while something the features need is missing.
    setup_button: gtk::Button,
    /// "Linksy X Is Available", when the service found a newer version.
    update_button: gtk::Button,
    /// Where it leads: the new version's release notes.
    update_notes: RefCell<Option<String>>,
    // Content
    content: adw::NavigationPage,
    content_stack: gtk::Stack,
    content_header: adw::HeaderBar,
    page: page::DevicePage,
    // Camera preview page
    camera_picture: gtk::Picture,
    camera_controls: camera::Controls,
    camera_back: gtk::Button,
    camera_title: adw::WindowTitle,
    // Texting through a phone
    messages: Rc<messages::MessagesView>,
    corner: Rc<corner::Corner>,
    transfers: Rc<transfers::TransfersPage>,
    island: Rc<island::Island>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Linksy")
            .default_width(880)
            .default_height(640)
            .width_request(360)
            .height_request(480)
            .build();

        // ---- sidebar ----
        let brand = gtk::Box::builder().spacing(8).build();
        brand.append(&mark::new(22, false));
        brand.append(&gtk::Label::builder().label("Linksy").css_classes(["heading"]).build());
        let sidebar_header = adw::HeaderBar::builder().show_title(false).build();
        sidebar_header.pack_start(&brand);
        sidebar_header.pack_end(&main_menu());

        let paired_list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        let nearby_list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["navigation-sidebar"]).build();
        let nearby_group = gtk::Box::builder().orientation(gtk::Orientation::Vertical).visible(false).build();
        nearby_group.append(&gtk::Label::builder().label(t!("Nearby, Not Paired")).xalign(0.0).css_classes(["sidebar-heading"]).build());
        nearby_group.append(&nearby_list);

        let no_devices = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["sidebar-empty"]).visible(false).build();
        no_devices.append(&mark::new(40, true));
        no_devices.append(&gtk::Label::builder().label(t!("Looking for devices…")).css_classes(["heading"]).build());
        no_devices.append(
            &gtk::Label::builder()
                .label(t!("Open Linksy on another device on this network, or add one."))
                .wrap(true)
                .justify(gtk::Justification::Center)
                .css_classes(["dim-label", "caption"])
                .build(),
        );

        let sidebar_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        sidebar_box.append(&paired_list);
        sidebar_box.append(&no_devices);
        sidebar_box.append(&nearby_group);
        let sidebar_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&sidebar_box).build();

        let island = island::Island::new();
        let me_label = gtk::Label::builder().css_classes(["dim-label", "caption"]).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let setup_button = gtk::Button::builder().action_name("win.setup").css_classes(["flat", "setup-button"]).visible(false).build();
        let update_button = gtk::Button::builder().css_classes(["flat", "update-button"]).tooltip_text(t!("See what's new")).visible(false).build();
        let footer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["sidebar-footer"]).build();
        footer.append(&update_button);
        footer.append(&setup_button);
        footer.append(&island.root);
        footer.append(&me_label);

        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&sidebar_header);
        sidebar_view.set_content(Some(&sidebar_scroll));
        sidebar_view.add_bottom_bar(&footer);
        let sidebar = adw::NavigationPage::builder().title("Linksy").tag("sidebar").child(&sidebar_view).build();

        // ---- content ----
        let page = page::DevicePage::new();
        let welcome_body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).halign(gtk::Align::Center).build();
        welcome_body.append(&gtk::Button::builder().label(t!("Add Device")).action_name("win.add-device").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).build());
        let welcome_mark = mark::new(96, false);
        welcome_mark.set_halign(gtk::Align::Center);
        let welcome_inner = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).valign(gtk::Align::Center).build();
        welcome_inner.append(&welcome_mark);
        welcome_inner.append(
            &adw::StatusPage::builder().title(t!("Your Devices, Connected")).description(t!("Pick a device to share with it, or add a new one.")).child(&welcome_body).build(),
        );
        // Sized by the page on show, not the widest of them (the camera controls).
        let content_stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).hhomogeneous(false).vhomogeneous(false).build();
        content_stack.add_named(&welcome_inner, Some("welcome"));
        content_stack.add_named(&page.root, Some("device"));
        // The camera preview: a page of its own, filling the content area.
        // Covers the whole area (edges may be cropped), like a camera viewfinder.
        let camera_picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).hexpand(true).vexpand(true).css_classes(["screen-view"]).build();
        let camera_controls = camera::Controls::new();
        let camera_page = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        camera_page.append(&camera_picture);
        camera_page.append(&camera_controls.root);
        content_stack.add_named(&camera_page, Some("camera"));
        // Click the preview to focus there, as on the phone.
        let click = gtk::GestureClick::new();
        let (pic, controls) = (camera_picture.clone(), camera_controls.clone());
        click.connect_released(move |_, _, x, y| {
            let Some(p) = pic.paintable() else { return };
            let (pw, ph) = (p.intrinsic_width() as f64, p.intrinsic_height() as f64);
            if pw > 0.0 && ph > 0.0 {
                let (fx, fy) = camera::cover_point((pic.width() as f64, pic.height() as f64), (pw, ph), (x, y));
                controls.focus_at(fx, fy);
            }
        });
        camera_picture.add_controller(click);
        let messages = messages::MessagesView::new();
        content_stack.add_named(&messages.root, Some("messages"));
        let transfers = transfers::TransfersPage::new();
        content_stack.add_named(&transfers.root, Some("transfers"));
        let camera_back = gtk::Button::builder().icon_name("go-previous-symbolic").tooltip_text(t!("Back")).visible(false).build();
        let camera_title = adw::WindowTitle::new("", t!("Camera Preview"));
        let content_header = adw::HeaderBar::builder().show_title(false).title_widget(&camera_title).build();
        // Hides the device list on wide windows; opens it over the page on narrow ones.
        let sidebar_toggle = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").tooltip_text(t!("Toggle Sidebar")).active(true).build();
        content_header.pack_start(&sidebar_toggle);
        content_header.pack_start(&camera_back);
        content_header.pack_end(&page.menu);
        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&content_header);
        content_view.set_content(Some(&content_stack));
        let rail_list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        let rail_add = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text(t!("Add Device")).action_name("win.add-device").halign(gtk::Align::Center).css_classes(["circular", "rail-add"]).build();
        let rail = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).visible(false).css_classes(["device-rail"]).build();
        rail.append(&rail_list);
        rail.append(&gtk::Box::builder().vexpand(true).build());
        rail.append(&rail_add);
        content_view.set_hexpand(true);
        let content_row = gtk::Box::builder().build();
        content_row.append(&rail);
        content_row.append(&content_view);
        let content = adw::NavigationPage::builder().title("Linksy").tag("content").child(&content_row).build();

        let split = adw::OverlaySplitView::builder().sidebar(&sidebar).content(&content).min_sidebar_width(240.0).max_sidebar_width(300.0).build();
        sidebar_toggle.bind_property("active", &split, "show-sidebar").bidirectional().sync_create().build();

        // ---- offline / loading ----
        let start = gtk::Button::builder().label(t!("Start Linksy")).css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).action_name("win.start-service").build();
        let offline = adw::StatusPage::builder()
            .icon_name("network-offline-symbolic")
            .title(t!("Linksy Isn't Running"))
            .description(t!("The background service lets nearby devices find this one and receives files, even when this window is closed."))
            .child(&start)
            .build();
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        stack.add_named(&split, Some("main"));
        stack.add_named(&offline, Some("offline"));
        stack.add_named(&gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).halign(gtk::Align::Center).valign(gtk::Align::Center).build(), Some("loading"));
        stack.set_visible_child_name("loading");

        // Device names go in as plain text.
        let sharing_banner = adw::Banner::builder().button_label(t!("Stop Sharing")).use_markup(false).revealed(false).build();
        let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        outer.append(&sharing_banner);
        stack.set_vexpand(true);
        outer.append(&stack);
        let corner = corner::Corner::new();
        let overlay = gtk::Overlay::builder().child(&outer).build();
        overlay.add_overlay(&corner.root);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&overlay));
        window.set_content(Some(&toasts));

        // Medium windows collapse the texting view; narrow ones also show one
        // pane at a time. (The last matching breakpoint applies, so narrow repeats medium.)
        let collapse_texts = |b: &adw::Breakpoint| {
            b.add_setter(&messages.root, "collapsed", Some(&true.to_value()));
        };
        let medium = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 820sp").expect("breakpoint"));
        collapse_texts(&medium);
        window.add_breakpoint(medium);
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 620sp").expect("breakpoint"));
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&corner.root, "halign", Some(&gtk::Align::Fill.to_value()));
        collapse_texts(&narrow);
        window.add_breakpoint(narrow);

        let win = Rc::new(Self {
            window,
            bridge: Bridge::new(),
            state: RefCell::default(),
            toasts,
            stack,
            sharing_banner,
            split,
            rail,
            rail_list,
            paired_list,
            nearby_group,
            nearby_list,
            no_devices,
            me_label,
            setup_button,
            update_button,
            update_notes: RefCell::default(),
            content,
            content_stack,
            content_header,
            page,
            camera_picture,
            camera_controls,
            camera_back,
            camera_title,
            messages,
            corner,
            transfers,
            island,
        });
        let w = Rc::downgrade(&win);
        win.transfers.set_handler(move |action| {
            if let Some(w) = w.upgrade() {
                w.transfer_action(action);
            }
        });
        let w = Rc::downgrade(&win);
        win.island.set_handler(move |pick| {
            let Some(w) = w.upgrade() else { return };
            match pick {
                island::Pick::Transfers => w.open_transfers(),
                island::Pick::Send(id) => w.choose_files_for(id),
                island::Pick::AddDevice => w.show_invite(),
                island::Pick::PairAddress => w.ask_pair_address(),
                island::Pick::UsbCable => w.show_usb_cable(),
            }
        });
        // An open island closes on Escape or a click anywhere else
        let keys = gtk::EventControllerKey::builder().propagation_phase(gtk::PropagationPhase::Capture).build();
        let w = Rc::downgrade(&win);
        keys.connect_key_pressed(move |_, key, _, _| {
            if let Some(w) = w.upgrade()
                && key == gdk::Key::Escape
                && w.island.is_open()
            {
                w.island.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        win.window.add_controller(keys);
        let click = gtk::GestureClick::builder().propagation_phase(gtk::PropagationPhase::Capture).button(0).build();
        let w = Rc::downgrade(&win);
        click.connect_pressed(move |_, _, x, y| {
            let Some(w) = w.upgrade() else { return };
            let inside = w.window.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|p| p.is_ancestor(&w.island.root) || p == w.island.root);
            if w.island.is_open() && !inside {
                w.island.close();
            }
        });
        win.window.add_controller(click);
        let w = Rc::downgrade(&win);
        win.corner.set_open(move || {
            if let Some(w) = w.upgrade() {
                w.open_transfers();
            }
        });
        let w = Rc::downgrade(&win);
        win.corner.set_handler(move |action| {
            let Some(w) = w.upgrade() else { return };
            match action {
                page::Action::Cancel(t) => w.cancel(t),
                page::Action::Open(paths) => w.open_received(&paths),
                page::Action::ShowInFolder(path) => {
                    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
                    launcher.open_containing_folder(Some(&w.window), gio::Cancellable::NONE, |_| {});
                }
                _ => {}
            }
        });
        let w = Rc::downgrade(&win);
        win.camera_back.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.stop_preview();
                w.close_messages();
                w.render_page();
            }
        });
        win.window.set_help_overlay(Some(&shortcuts_window()));
        let w = Rc::downgrade(&win);
        win.sharing_banner.connect_button_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.stop_sharing();
            }
        });
        win.setup_actions();
        win.setup_sidebar();
        win.setup_page();
        win.setup_messages();
        win.refresh_setup();
        // One thing at a time: Finish Setup when it's needed, otherwise what's new
        if !win.offer_setup() {
            win.offer_news();
        }
        let w = win.clone();
        // Linksy X Is Available: what's new, and Update Now
        win.update_button.connect_clicked(move |_| w.show_upgrade());
        // The service checks once a day; ask it now and then what it found.
        let w = Rc::downgrade(&win);
        glib::timeout_add_seconds_local(30 * 60, move || match w.upgrade() {
            Some(w) => {
                w.refresh_updates();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        win.setup_drop();
        win.connect();
        win
    }

    // ---- setup ----

    fn setup_actions(self: &Rc<Self>) {
        type Run = fn(&Rc<Window>);
        let actions: [(&str, Run); 14] = [
            ("choose-files", |w| w.choose_files()),
            ("transfers", |w| w.open_transfers()),
            ("toggle-sidebar", |w| w.split.set_show_sidebar(!w.split.shows_sidebar())),
            ("start-service", |w| w.start_service()),
            ("messages", |w| {
                if let Some(d) = w.selected_device().filter(|d| d.trusted && d.device.kind == DeviceKind::Phone) {
                    w.open_messages(d.device.id);
                }
            }),
            ("preferences", |w| w.show_preferences()),
            ("setup", |w| w.show_setup()),
            ("news", |w| w.show_news()),
            ("upgrade", |w| w.show_upgrade()),
            ("usb-cable", |w| w.show_usb_cable()),
            ("add-device", |w| w.show_invite()),
            ("pair-address", |w| w.ask_pair_address()),
            ("preview-camera", |w| {
                let on = w.state.borrow().camera;
                match on.and_then(|(_, id)| w.device(&id)) {
                    Some(d) => w.preview_camera(&setup::camera_name(&d.device.name)),
                    None => w.toast(t!("No camera is on")),
                }
            }),
            ("view-screen", |w| match w.selected_device().filter(|d| d.trusted) {
                Some(d) => w.view_screen(d.device.id),
                None => w.toast(t!("Pick a device first")),
            }),
        ];
        for (name, run) in actions {
            let action = gio::SimpleAction::new(name, None);
            let w = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(w) = w.upgrade() {
                    run(&w);
                }
            });
            self.window.add_action(&action);
        }
    }

    /// On narrow windows, where the device list covers the page, get it out of the way.
    fn show_content(&self) {
        if self.split.is_collapsed() {
            self.split.set_show_sidebar(false);
        }
    }

    fn setup_sidebar(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.paired_list.connect_row_selected(move |_, row| {
            let Some(w) = w.upgrade() else { return };
            let Some(id) = row.and_then(|r| r.widget_name().parse::<Fingerprint>().ok()) else { return };
            let was = std::mem::replace(&mut w.state.borrow_mut().transfers_open, false);
            w.select(Some(id));
            if was {
                w.render_page();
            }
        });

        // The avatar strip stands in for the hidden sidebar on wide windows.
        for prop in ["show-sidebar", "collapsed"] {
            let w = Rc::downgrade(self);
            self.split.connect_notify_local(Some(prop), move |split, _| {
                if let Some(w) = w.upgrade() {
                    w.rail.set_visible(!split.shows_sidebar() && !split.is_collapsed());
                }
            });
        }
        // A narrow window starts on the device list, not on an empty page.
        let w = Rc::downgrade(self);
        self.split.connect_collapsed_notify(move |split| {
            if let Some(w) = w.upgrade()
                && split.is_collapsed()
                && w.state.borrow().selected.is_none()
            {
                split.set_show_sidebar(true);
            }
        });
        // Clicking the selected row again (collapsed layout) should still open its page.
        let w = Rc::downgrade(self);
        self.paired_list.connect_row_activated(move |_, _| {
            if let Some(w) = w.upgrade() {
                w.show_content();
            }
        });
    }

    fn setup_page(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.page.connect_actions(move |action| {
            let Some(w) = w.upgrade() else { return };
            let Some(id) = w.state.borrow().selected else { return };
            match action {
                page::Action::SendFiles => w.choose_files(),
                page::Action::SendClipboard => w.send_clipboard(id),
                page::Action::Ping => w.ping(id),
                page::Action::Ring => w.ring(id, true),
                page::Action::Messages => w.open_messages(id),
                page::Action::BrowseFiles => w.browse_files(id),
                page::Action::LockWhenAway(on) => w.lock_when_away(id, on),
                page::Action::AwayAll(on) => w.away_all(id, on),
                page::Action::AutoAccept(on) => w.auto_accept(id, on),
                page::Action::AllowScreen(on) => w.allow_screen(id, on),
                page::Action::Locate => w.locate(id),
                page::Action::ViewScreen => w.view_screen(id),
                page::Action::Extend => w.extend_to(id),
                page::Action::Webcam => w.use_camera(id),
                page::Action::Microphone => w.use_microphone(id),
                page::Action::CameraPreview => {
                    let name = w.device(&id).map(|d| d.device.name).unwrap_or_default();
                    w.preview_camera(&setup::camera_name(&name));
                }
                page::Action::CameraStop => {
                    let camera = w.state.borrow().camera;
                    if let Some((session, _)) = camera {
                        w.end_session(session);
                    }
                }
                page::Action::MicListen(on) => w.listen_microphone(on),
                page::Action::UsbCable => w.show_usb_cable(),
                page::Action::MicStop => {
                    let mic = w.state.borrow().microphone;
                    if let Some((session, _)) = mic {
                        w.end_session(session);
                    }
                }
                page::Action::Pair => w.pair(id),
                page::Action::SendStaged => {
                    let staged = std::mem::take(&mut w.state.borrow_mut().staged);
                    w.send(id, staged);
                    w.render_page();
                }
                page::Action::Forget => {
                    let name = w.device(&id).map(|d| d.device.name).unwrap_or_default();
                    w.confirm_forget(id, &name);
                }
                page::Action::ClearHistory => w.clear_history(id),
                page::Action::FilterActivity => w.render_activity(),
                page::Action::AllowTerminal(on) => w.terminal_call(move |c| async move { c.allow_terminal(&id.to_string(), on).await }),
                page::Action::SetUpTerminal => {
                    // A code first; then this phone may use it.
                    let w2 = w.clone();
                    w.ask_terminal_code(&w.window, move |result| match result {
                        Ok(()) => {
                            w2.toast(t!("Terminal code set"));
                            w2.terminal_call(move |c| async move { c.allow_terminal(&id.to_string(), true).await });
                        }
                        Err(e) => w2.toast_error(&e, None),
                    });
                }
                page::Action::ChangeTerminalCode => {
                    let w2 = w.clone();
                    w.ask_terminal_code(&w.window, move |result| match result {
                        Ok(()) => w2.toast(t!("Terminal code changed")),
                        Err(e) => w2.toast_error(&e, None),
                    });
                }
                page::Action::UnlockTerminal => w.terminal_call(move |c| async move { c.unlock_terminal(&id.to_string()).await }),
                page::Action::EndTerminal(t) => w.terminal_call(move |c| async move { c.end_terminal(&t).await }),
                page::Action::Cancel(t) => w.cancel(t),
                page::Action::Open(paths) => w.open_received(&paths),
                page::Action::ShowInFolder(path) => {
                    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
                    launcher.open_containing_folder(Some(&w.window), gio::Cancellable::NONE, |_| {});
                }
                page::Action::Copy(text) => {
                    w.window.clipboard().set_text(&text);
                    w.toast(t!("Copied"));
                }
                page::Action::OpenLink(url) => gtk::UriLauncher::new(&url).launch(Some(&w.window), gio::Cancellable::NONE, |_| {}),
            }
        });
    }

    fn setup_drop(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let w = Rc::downgrade(self);
        target.connect_enter(move |_, _, _| {
            if let Some(w) = w.upgrade() {
                w.page.set_drop_active(true);
            }
            gdk::DragAction::COPY
        });
        let w = Rc::downgrade(self);
        target.connect_leave(move |_| {
            if let Some(w) = w.upgrade() {
                w.page.set_drop_active(false);
            }
        });
        let w = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let Some(w) = w.upgrade() else { return false };
            w.page.set_drop_active(false);
            let paths = file_list_paths(value);
            if paths.is_empty() {
                return false;
            }
            // Dropped while a paired device is open: send there. Otherwise keep the files
            // until a device is picked.
            match w.selected_device().filter(|d| d.trusted) {
                Some(d) => w.send(d.device.id, paths),
                None => w.set_staged(paths),
            }
            true
        });
        self.window.add_controller(target);
    }

    // ---- service connection ----

    fn connect(self: &Rc<Self>) {
        let w = self.clone();
        glib::spawn_future_local(async move {
            match w.bridge.connect().await {
                Ok((client, updates)) => {
                    w.state.borrow_mut().client = Some(client);
                    w.stack.set_visible_child_name("main");
                    w.refresh_status();
                    w.refresh_devices();
                    w.join_pending_displays();
                    w.restore_sessions();
                    w.refresh_updates();
                    w.refresh_terminals();
                    w.refresh_corner();
                    while let Ok(update) = updates.recv().await {
                        match update {
                            Update::Event(ev) => w.handle_event(ev),
                            Update::History(h) => w.history_changed(h),
                        }
                    }
                    // Service went away.
                    w.state.borrow_mut().client = None;
                    w.corner.reset();
                    w.stack.set_visible_child_name("offline");
                }
                Err(_) => {
                    // Not when pointed at a particular service (development): that one is
                    // started by hand.
                    let start = std::env::var_os("LINKSY_SOCKET").is_none() && !std::mem::replace(&mut w.state.borrow_mut().started_service, true);
                    if start {
                        w.start_service();
                    } else {
                        w.stack.set_visible_child_name("offline");
                    }
                }
            }
        });
    }

    fn start_service(self: &Rc<Self>) {
        use std::process::{Command, Stdio};
        // The installed user service first, so it stays managed (restarts, logs); else
        // run linksyd from next to this app, or from PATH.
        if std::env::var_os("FLATPAK_ID").is_some() {
            return self.start_sandboxed_service();
        }
        let via_systemd = Command::new("systemctl")
            .args(["--user", "start", "linksyd.service"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        let spawned = if via_systemd {
            Ok(())
        } else {
            let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("linksyd"))).filter(|p| p.exists());
            Command::new(beside.unwrap_or_else(|| "linksyd".into()))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|_| ())
        };
        if let Err(e) = spawned {
            self.toast(&t!("Could not start the service: {error}", error = e));
            return;
        }
        self.wait_for_service();
    }

    /// In a Flatpak the service can't be a child of this window's sandbox, which ends with
    /// the window: it runs in a sandbox of its own, and starts with every login from now on.
    fn start_sandboxed_service(self: &Rc<Self>) {
        use std::process::{Command, Stdio};
        if let Err(e) = Command::new("flatpak-spawn").arg("linksyd").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            self.toast(&t!("Could not start the service: {error}", error = e));
            return;
        }
        let options = glib::VariantDict::new(None);
        options.insert("reason", t!("Linksy stays ready for your devices when its window is closed."));
        options.insert("autostart", true);
        options.insert("commandline", vec!["linksyd"]);
        if let Some(bus) = self.window.application().and_then(|app| app.dbus_connection()) {
            bus.call(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Background",
                "RequestBackground",
                Some(&glib::Variant::tuple_from_iter(["".to_variant(), options.end()])),
                None,
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
                |_| {},
            );
        }
        self.wait_for_service();
    }

    /// The service still running an older Linksy after an update: stop it and start the
    /// installed one, once per window.
    fn restart_service(self: &Rc<Self>) {
        use std::process::{Command, Stdio};
        if std::env::var_os("LINKSY_SOCKET").is_some() || std::mem::replace(&mut self.state.borrow_mut().restarted_service, true) {
            return;
        }
        let Some(c) = self.client() else { return };
        let w = self.clone();
        // A service from before Quit (0.6.0 and older) never answers it: give up after a
        // moment and restart it the other way.
        let call = self.bridge.call(&c, |c| async move { tokio::time::timeout(std::time::Duration::from_secs(3), c.quit()).await.unwrap_or(Err(linksy_ipc::ClientError::Disconnected)) });
        glib::spawn_future_local(async move {
            if call.await.is_err() && std::env::var_os("FLATPAK_ID").is_none() {
                // Older than Quit: a systemd service can still be restarted from here.
                let _ = Command::new("systemctl").args(["--user", "restart", "linksyd.service"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
                glib::timeout_future(Duration::from_millis(500)).await;
                return w.wait_for_service();
            }
            // Wait for it to go, then start the new one.
            for _ in 0..20 {
                glib::timeout_future(Duration::from_millis(150)).await;
                if std::fs::metadata(linksy_ipc::socket_path()).is_err() {
                    break;
                }
            }
            w.start_service();
        });
    }

    fn wait_for_service(self: &Rc<Self>) {
        self.stack.set_visible_child_name("loading");
        let w = self.clone();
        glib::spawn_future_local(async move {
            // Give the service a moment to open its socket. Three seconds is a long time
            // for a socket. It has never needed them all. I hope this doesn't break.
            for _ in 0..20 {
                glib::timeout_future(Duration::from_millis(150)).await;
                if std::fs::metadata(linksy_ipc::socket_path()).is_ok() {
                    break;
                }
            }
            w.connect();
        });
    }

    fn client(&self) -> Option<Arc<Client>> {
        self.state.borrow().client.clone()
    }

    fn refresh_status(self: &Rc<Self>) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, |c| async move { c.lock_prefs().await });
        glib::spawn_future_local(async move {
            if let Ok(prefs) = call.await {
                w.state.borrow_mut().lock = prefs;
                w.refresh_devices();
            }
        });
        let w = self.clone();
        let call = self.bridge.call(&c, |c| async move { c.status().await });
        glib::spawn_future_local(async move {
            if let Ok(s) = call.await {
                if s.version != env!("CARGO_PKG_VERSION") {
                    w.restart_service();
                }
                w.me_label.set_label(&t!("This device: {name}", name = s.device.name));
                let mut state = w.state.borrow_mut();
                state.download_dir = Some(s.download_dir);
                state.port = Some(s.port);
            }
        });
    }

    fn refresh_devices(self: &Rc<Self>) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, |c| async move { c.devices().await });
        glib::spawn_future_local(async move {
            if let Ok(devices) = call.await {
                // Look up each paired device's picture once; Event::Picture brings new ones.
                let unchecked: Vec<Fingerprint> = {
                    let mut state = w.state.borrow_mut();
                    let ids: Vec<Fingerprint> = devices.iter().filter(|d| d.trusted).map(|d| d.device.id).collect();
                    state.devices = devices;
                    ids.into_iter().filter(|id| state.pictures_checked.insert(*id)).collect()
                };
                for id in unchecked {
                    w.load_picture(id);
                }
                w.render_sidebar();
                w.render_page();
            }
        });
    }

    /// Accept files from a device without asking, or ask again.
    fn auto_accept(self: &Rc<Self>, id: Fingerprint, on: bool) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.set_auto_accept(id.to_string(), on).await });
        glib::spawn_future_local(async move {
            if let Err(e) = call.await {
                w.toast_error(&e, Some(id));
            }
            w.refresh_devices();
        });
    }

    /// Let a device see and control the screen without asking, or ask again.
    fn allow_screen(self: &Rc<Self>, id: Fingerprint, on: bool) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.set_allow_screen(id.to_string(), on).await });
        glib::spawn_future_local(async move {
            if let Err(e) = call.await {
                w.toast_error(&e, Some(id));
            }
            w.refresh_devices();
        });
    }

    /// Allow everything from away for a device, or only the small things.
    fn away_all(self: &Rc<Self>, id: Fingerprint, on: bool) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.set_away_all(id.to_string(), on).await });
        glib::spawn_future_local(async move {
            if let Err(e) = call.await {
                w.toast_error(&e, Some(id));
            }
            w.refresh_devices();
        });
    }

    /// Ask a phone where it is, then show it, with a map link.
    fn locate(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        self.toast(&t!("Asking {name} where it is…", name));
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.locate(id.to_string()).await });
        glib::spawn_future_local(async move {
            let l = match call.await {
                Ok(l) => l,
                Err(e) => return w.toast(&t!("{name} didn't say: {reason}", name, reason = fmt::capitalize(&e))),
            };
            let accuracy = l.accuracy_m.map(|a| t!(", within {meters} m", meters = format!("{a:.0}"))).unwrap_or_default();
            let when = glib::DateTime::from_unix_local((l.at_ms / 1000) as i64).and_then(|d| d.format("%H:%M")).map(|time| t!(" at {time}", time)).unwrap_or_default();
            let dialog = adw::AlertDialog::new(
                Some(&t!("{name} Is Here", name)),
                Some(&t!("{coordinates}{accuracy}{when}. Opening the map shows it on OpenStreetMap in your browser.", coordinates = format!("{:.5}, {:.5}", l.lat, l.lon), accuracy, when)),
            );
            dialog.add_responses(&[("close", t!("Close")), ("map", t!("Open Map"))]);
            dialog.set_response_appearance("map", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("map"));
            let url = format!("https://www.openstreetmap.org/?mlat={:.5}&mlon={:.5}#map=17/{:.5}/{:.5}", l.lat, l.lon, l.lat, l.lon);
            let win = w.window.clone();
            dialog.connect_response(Some("map"), move |_, _| {
                gtk::UriLauncher::new(&url).launch(Some(&win), gio::Cancellable::NONE, |_| {});
            });
            dialog.present(Some(&w.window));
        });
    }

    /// Show "Linksy X Is Available" when the service found a newer version.
    pub(super) fn refresh_terminals(self: &Rc<Self>) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        // An older service may never answer this one.
        let call = self.bridge.call(&c, |c| async move { tokio::time::timeout(std::time::Duration::from_secs(3), c.terminal_status()).await.unwrap_or(Err(linksy_ipc::ClientError::Disconnected)) });
        glib::spawn_future_local(async move {
            w.state.borrow_mut().terminals = call.await.ok();
            w.render_page();
        });
    }

    /// Run a terminal request to the service, then show what changed.
    fn terminal_call<F, Fut>(self: &Rc<Self>, f: F)
    where
        F: FnOnce(Arc<Client>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), linksy_ipc::ClientError>> + Send + 'static,
    {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, f);
        glib::spawn_future_local(async move {
            if let Err(e) = call.await {
                w.toast_error(&e, None);
            }
            w.refresh_terminals();
        });
    }

    fn refresh_updates(self: &Rc<Self>) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, |c| async move { c.updates().await });
        glib::spawn_future_local(async move {
            let latest = call.await.ok().filter(|u| u.prefs.check).and_then(|u| Some((u.latest?, u.notes)));
            w.update_button.set_visible(latest.is_some());
            if let Some((version, notes)) = latest {
                w.update_button.set_label(&t!("Linksy {version} Is Available", version));
                *w.update_notes.borrow_mut() = notes;
            }
        });
    }

    /// Load the picture `id` sent, if any, and show it.
    fn load_picture(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.device_picture(id.to_string()).await });
        glib::spawn_future_local(async move {
            let Ok(Some(path)) = call.await else { return };
            if let Ok(texture) = gdk::Texture::from_filename(&path) {
                w.state.borrow_mut().pictures.insert(id, texture);
                w.render_sidebar();
                w.render_page();
            }
        });
    }

    fn refresh_history(self: &Rc<Self>) {
        let Some(c) = self.client() else { return };
        let Some(id) = self.state.borrow().selected else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.history(Some(id.to_string())).await });
        glib::spawn_future_local(async move {
            // The selection may have moved on while this was in flight.
            if let Ok(history) = call.await
                && w.state.borrow().selected == Some(id)
            {
                w.state.borrow_mut().history = history;
                w.render_activity();
            }
        });
    }

    pub fn download_dir(&self) -> Option<PathBuf> {
        self.state.borrow().download_dir.clone()
    }

    // ---- events ----

    fn handle_event(self: &Rc<Self>, ev: Event) {
        match ev {
            Event::DeviceFound { .. }
            | Event::DeviceLost { .. }
            | Event::DeviceUpdated { .. }
            | Event::Connected { .. }
            | Event::Disconnected { .. } => self.refresh_devices(),
            Event::PairingRequest { request, device, code, incoming, older } => self.show_pairing(request, device, code, incoming, older),
            Event::Paired { device } => {
                self.close_pair_dialog(&device.id);
                self.close_invite_dialog();
                self.toast(&t!("Paired with {name}", name = device.name));
                // Open the new device's page.
                self.state.borrow_mut().selected = Some(device.id);
                self.refresh_devices();
                self.refresh_history();
            }
            Event::PairingFailed { device, reason } => {
                self.close_pair_dialog(&device.id);
                if reason.contains("other device") {
                    self.toast(&t!("{name} didn't confirm the code", name = device.name));
                }
                self.render_page();
            }
            Event::TransferRequest { transfer, from, files, total } => self.show_offer(transfer, &from, &files, total),
            Event::TransferStarted { transfer, total, peer, direction, files, import } => {
                // Accepted elsewhere (desktop notification, another window): drop the prompt.
                self.close_offer_dialog(&transfer);
                self.state.borrow_mut().progress.insert(transfer, Progress { total, ..Default::default() });
                let files: Vec<(String, Option<u64>)> = files.into_iter().map(|f| (f.path, Some(f.size))).collect();
                let moving = transfers::Moving { id: transfer, peer: peer.id, peer_name: peer.name, incoming: direction == Direction::Incoming, import, files, total };
                {
                    let mut state = self.state.borrow_mut();
                    match state.moving.iter_mut().find(|m| m.id == transfer) {
                        Some(m) => *m = moving,
                        None => state.moving.push(moving),
                    }
                }
                self.transfers_changed();
            }
            Event::TransferProgress { transfer, bytes, total } => self.update_progress(transfer, bytes, total),
            Event::TransferComplete { transfer, .. } => {
                self.state.borrow_mut().progress.remove(&transfer);
            }
            Event::TransferFailed { transfer, .. } => {
                self.close_offer_dialog(&transfer);
                self.state.borrow_mut().progress.remove(&transfer);
            }
            Event::SettingsChanged { .. } => self.refresh_status(),
            Event::SessionRequest { request, from, kind } => self.show_session_request(request, &from, kind),
            // Terminals show on the phone's page, with End; the service notifies.
            Event::SessionStarted { session, kind: SessionKind::Terminal, .. } => {
                self.close_offer_dialog(&session);
                self.refresh_terminals();
            }
            Event::SessionStarted { session, peer, kind, sharing } if sharing || kind.is_device() => {
                self.sharing_started(session, &peer, kind, sharing)
            }
            Event::SessionStarted { kind: SessionKind::Display | SessionKind::Screen, .. } => self.join_pending_displays(),
            Event::SessionStarted { .. } => {}
            Event::SessionEnded { session } => {
                self.session_ended(session);
                self.refresh_terminals();
            }
            Event::CameraState { session, state, .. } => {
                if self.state.borrow().camera.is_some_and(|(s, _)| s == session) {
                    self.camera_controls.show(state);
                }
            }
            Event::Picture { from, .. } => self.load_picture(from.id),
            Event::Notification { from, title, body } => {
                let text = if body.is_empty() { format!("{}: {title}", from.name) } else { format!("{}: {title} — {body}", from.name) };
                self.toast(&text);
            }
            Event::Clipboard { from, text } => {
                // The service copies it too; a focused window can also do it directly.
                if self.window.is_active() {
                    self.window.clipboard().set_text(&text);
                }
                self.toast(&t!("Copied from {name}", name = from.name));
            }
            // The service opens it; the timeline shows it.
            Event::OpenUrl { .. } => {}
            Event::Battery { .. } => self.refresh_devices(),
            Event::Ring { from, on: true } => self.toast(&t!("{name} is looking for this computer", name = from.name)),
            Event::Ring { on: false, .. } => {}
            // Phone notifications are the service's to show, as desktop notifications.
            Event::NotificationPosted { .. }
            | Event::NotificationRemoved { .. }
            | Event::NotificationReply { .. }
            | Event::NotificationAction { .. }
            | Event::NotificationDismissed { .. } => {}
            // Media players are the service's (MPRIS, so GNOME's own controls show them).
            Event::MediaState { .. } | Event::MediaCommand { .. } | Event::MediaWatch { .. } | Event::CallState { .. } => {}
            Event::VolumeState { .. } | Event::VolumeSet { .. } | Event::VolumeWatch { .. } => {}
            // Direct links are the service's to set up (docs/protocol.md §4.17).
            Event::DirectWanted { id, .. } | Event::Direct { from: DeviceInfo { id, .. }, .. } => {
                self.state.borrow_mut().direct.entry(id).or_insert(false);
                self.transfers_changed();
            }
            Event::DirectLinked { id } => {
                self.state.borrow_mut().direct.insert(id, true);
                self.transfers_changed();
            }
            Event::DirectEnded { id, .. } => {
                self.state.borrow_mut().direct.remove(&id);
                self.transfers_changed();
            }
            Event::Heartbeat { .. } | Event::HeartbeatWatch { .. } | Event::ClipboardWatch { .. } | Event::ClipboardFile { .. } => {}
            // Between the service and phones (USB cable, §4.11)
            Event::UsbPlugged { .. } | Event::UsbConnect { .. } => {}
            // Locate is answered through the service (it waits for the reply).
            Event::LocateRequest { .. } | Event::Location { .. } | Event::LocationRefused { .. } | Event::TerminalListRequest { .. } | Event::Terminals { .. } => {}
            Event::SmsMessage { from, thread, message } => self.sms_arrived(from.id, thread, message),
            // Answers go to whoever asked (the service waits for them); requests are a phone's.
            Event::SmsThreads { .. }
            | Event::SmsMessages { .. }
            | Event::SmsSent { .. }
            | Event::SmsListThreads { .. }
            | Event::SmsListMessages { .. }
            | Event::SmsSend { .. } => {}
        }
    }

    fn history_changed(self: &Rc<Self>, entry: HistoryEntry) {
        self.corner.entry(&entry);
        if matches!(entry.what, Shared::Files { .. }) {
            self.note_transfer(entry.clone());
            self.transfers_changed();
        }
        if let Some(p) = self.state.borrow().progress.get(&entry.id) {
            self.corner.progress(entry.id, p);
        }
        if self.state.borrow().selected != Some(entry.peer) {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            match state.history.iter_mut().find(|e| e.id == entry.id) {
                Some(e) => *e = entry,
                None => state.history.insert(0, entry),
            }
        }
        self.render_activity();
    }

    fn update_progress(&self, id: Uuid, bytes: u64, total: u64) {
        let mut state = self.state.borrow_mut();
        let p = state.progress.entry(id).or_default();
        let now = Instant::now();
        p.bytes = bytes;
        p.total = total;
        p.samples.push((now, bytes));
        p.samples.retain(|(at, _)| now.duration_since(*at) < Duration::from_secs(3));
        let detail = progress_text(p);
        let fraction = if total == 0 { 1.0 } else { bytes as f64 / total as f64 };
        if let Some(row) = state.live.get(&id) {
            row.detail.set_label(&detail);
            mark::ease_progress(&row.bar, fraction);
        }
        if let Some(p) = state.progress.get(&id) {
            self.corner.progress(id, p);
        }
        let again = state.transfers_open && self.transfers.update(&state.moving, &state.progress);
        drop(state);
        self.update_island();
        if again {
            self.render_transfers();
        }
    }

    fn refresh_corner(self: &Rc<Self>) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, |c| async move { c.history(None).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(history) = call.await {
                for e in history.iter().rev().filter(|e| matches!(e.status, linksy_ipc::HistoryStatus::InProgress)) {
                    w.corner.entry(e);
                }
                {
                    let mut state = w.state.borrow_mut();
                    state.moving.clear();
                    state.recent.clear();
                }
                for e in history.into_iter().rev() {
                    w.note_transfer(e);
                }
                w.transfers_changed();
            }
        });
    }

    fn note_transfer(&self, e: HistoryEntry) {
        let Shared::Files { files, sizes, total, import, .. } = &e.what else { return };
        let mut state = self.state.borrow_mut();
        if matches!(e.status, linksy_ipc::HistoryStatus::InProgress) {
            if !state.moving.iter().any(|m| m.id == e.id) {
                let moving = transfers::Moving {
                    id: e.id,
                    peer: e.peer,
                    peer_name: e.peer_name.clone(),
                    incoming: e.direction == Direction::Incoming,
                    import: *import,
                    files: files.iter().enumerate().map(|(i, f)| (f.clone(), (sizes.len() == files.len()).then(|| sizes[i]))).collect(),
                    total: *total,
                };
                state.moving.push(moving);
            }
        } else {
            state.moving.retain(|m| m.id != e.id);
            state.recent.retain(|r| r.id != e.id);
            state.recent.insert(0, e);
            state.recent.truncate(40);
        }
    }

    fn transfers_changed(&self) {
        self.update_island();
        if self.state.borrow().transfers_open {
            self.render_transfers();
        }
    }

    fn update_island(&self) {
        let state = self.state.borrow();
        let files: usize = state.moving.iter().map(|m| m.files.len()).sum();
        let (bytes, total) = state.moving.iter().filter_map(|m| state.progress.get(&m.id)).fold((0, 0), |(b, t), p| (b + p.bytes, t + p.total));
        self.island.set_moving(files, (total > 0).then(|| bytes as f64 / total as f64));
    }

    fn open_transfers(self: &Rc<Self>) {
        self.state.borrow_mut().transfers_open = true;
        self.paired_list.unselect_all();
        self.render_page();
        self.show_content();
    }

    fn render_transfers(&self) {
        let state = self.state.borrow();
        let first = state.moving.first().map(|m| m.peer);
        let device = first.and_then(|id| state.devices.iter().find(|d| d.device.id == id));
        let route = match (first.and_then(|id| state.direct.get(&id)), device) {
            (Some(true), _) => transfers::Route::Direct,
            (Some(false), _) => transfers::Route::DirectSetup,
            (None, Some(d)) if d.usb => transfers::Route::Usb,
            (None, Some(d)) if d.away => transfers::Route::Away,
            _ => transfers::Route::Wifi,
        };
        let kind = device.map(|d| d.device.kind).unwrap_or(DeviceKind::Phone);
        self.transfers.show(&state.moving, kind, route, &state.recent, &state.progress);
    }

    fn transfer_action(self: &Rc<Self>, action: page::Action) {
        match action {
            page::Action::Cancel(t) => self.cancel(t),
            page::Action::Open(paths) => self.open_received(&paths),
            page::Action::ShowInFolder(path) => {
                let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
                launcher.open_containing_folder(Some(&self.window), gio::Cancellable::NONE, |_| {});
            }
            _ => {}
        }
    }

    // ---- sidebar ----

    fn render_sidebar(self: &Rc<Self>) {
        let (paired, nearby, selected): (Vec<DeviceView>, Vec<DeviceView>, Option<Fingerprint>) = {
            let state = self.state.borrow();
            let (p, n) = state.devices.iter().cloned().partition(|d| d.trusted);
            (p, n, state.selected)
        };
        self.island.set_devices(&paired);
        self.paired_list.remove_all();
        let mut to_select = None;
        for d in &paired {
            let row = sidebar_row(d, self.state.borrow().pictures.get(&d.device.id));
            self.paired_list.append(&row);
            if Some(d.device.id) == selected {
                to_select = Some(row);
            }
        }
        while let Some(child) = self.rail_list.first_child() {
            self.rail_list.remove(&child);
        }
        for d in &paired {
            self.rail_list.append(&self.rail_button(d, Some(d.device.id) == selected));
        }
        self.nearby_list.remove_all();
        for d in &nearby {
            self.nearby_list.append(&self.nearby_row(d));
        }
        self.render_invite_nearby();
        self.paired_list.set_visible(!paired.is_empty());
        self.nearby_group.set_visible(!nearby.is_empty());
        self.no_devices.set_visible(paired.is_empty() && nearby.is_empty());
        let transfers_open = self.state.borrow().transfers_open;
        match to_select {
            _ if transfers_open => {}
            Some(row) => self.paired_list.select_row(Some(&row)),
            // Nothing selected yet: open the first paired device on wide layouts, so the
            // window doesn't start on an empty page.
            None if !self.split.is_collapsed() => {
                if let Some(row) = self.paired_list.row_at_index(0) {
                    self.paired_list.select_row(Some(&row));
                } else if selected.is_some() && !nearby.iter().any(|d| Some(d.device.id) == selected) {
                    self.select(None);
                }
            }
            None => {}
        }
    }

    /// A device on the avatar strip: its picture and status dot; a click opens it.
    fn rail_button(self: &Rc<Self>, d: &DeviceView, selected: bool) -> gtk::Button {
        let avatar = adw::Avatar::builder().size(40).text(&d.device.name).show_initials(true).build();
        avatar.set_custom_image(self.state.borrow().pictures.get(&d.device.id));
        let badge = gtk::Overlay::builder().child(&avatar).build();
        let dot = gtk::Box::builder().halign(gtk::Align::End).valign(gtk::Align::End).css_classes(["status-dot"]).build();
        if !(d.connected || d.nearby) {
            dot.add_css_class("away");
        }
        badge.add_overlay(&dot);
        let b = gtk::Button::builder().child(&badge).halign(gtk::Align::Center).tooltip_text(format!("{} · {}", d.device.name, status_text(d))).css_classes(["flat", "circular", "rail-device"]).build();
        b.set_widget_name(&d.device.id.to_string());
        if selected {
            b.add_css_class("selected");
        }
        let (w, id) = (Rc::downgrade(self), d.device.id.to_string());
        b.connect_clicked(move |_| {
            let Some(w) = w.upgrade() else { return };
            let mut i = 0;
            while let Some(row) = w.paired_list.row_at_index(i) {
                if row.widget_name() == id.as_str() {
                    w.paired_list.select_row(Some(&row));
                    break;
                }
                i += 1;
            }
        });
        b
    }

    fn nearby_row(self: &Rc<Self>, d: &DeviceView) -> gtk::ListBoxRow {
        let avatar = adw::Avatar::builder().size(32).text(&d.device.name).show_initials(true).build();
        let name = gtk::Label::builder().label(&d.device.name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let kind = gtk::Label::builder().label(kind_name(&d.device)).xalign(0.0).css_classes(["caption", "dim-label"]).build();
        let labels = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).hexpand(true).build();
        labels.append(&name);
        labels.append(&kind);
        let pair = gtk::Button::builder().label(t!("Pair")).valign(gtk::Align::Center).css_classes(["pill", "small"]).build();
        let id = d.device.id;
        let w = Rc::downgrade(self);
        pair.connect_clicked(move |b| {
            if let Some(w) = w.upgrade() {
                b.set_sensitive(false);
                w.pair(id);
            }
        });
        let b = gtk::Box::builder().spacing(10).margin_top(4).margin_bottom(4).margin_start(4).margin_end(4).build();
        b.append(&avatar);
        b.append(&labels);
        b.append(&pair);
        gtk::ListBoxRow::builder().child(&b).activatable(false).build()
    }

    fn select(self: &Rc<Self>, id: Option<Fingerprint>) {
        let changed = {
            let mut state = self.state.borrow_mut();
            let changed = state.selected != id;
            if changed {
                state.selected = id;
                state.history.clear();
            }
            changed
        };
        if changed {
            let mut child = self.rail_list.first_child();
            while let Some(b) = child {
                if id.is_some_and(|id| b.widget_name() == id.to_string().as_str()) {
                    b.add_css_class("selected");
                } else {
                    b.remove_css_class("selected");
                }
                child = b.next_sibling();
            }
            self.render_page();
            self.render_activity();
            self.refresh_history();
        }
        if id.is_some() {
            self.show_content();
        }
    }

    fn devices(&self) -> Vec<DeviceView> {
        self.state.borrow().devices.clone()
    }

    fn device(&self, id: &Fingerprint) -> Option<DeviceView> {
        self.state.borrow().devices.iter().find(|d| d.device.id == *id).cloned()
    }

    fn selected_device(&self) -> Option<DeviceView> {
        let id = self.state.borrow().selected?;
        self.device(&id)
    }

    // ---- device page ----

    fn render_page(&self) {
        let transfers_open = self.state.borrow().transfers_open;
        self.island.set_selected(transfers_open);
        self.corner.root.set_visible(!transfers_open);
        if transfers_open {
            self.content.set_title(t!("Transfers"));
            self.page.menu.set_visible(false);
            self.content_stack.set_visible_child_name("transfers");
            self.render_transfers();
            return;
        }
        let staged = self.state.borrow().staged.clone();
        // A preview stays open until left, unless its device or its camera went away.
        let (selected, camera, previewing) = {
            let s = self.state.borrow();
            (s.selected, s.camera.map(|(_, id)| id), s.preview.as_ref().map(|(id, _)| *id))
        };
        if previewing.is_some() && (previewing != selected || previewing != camera) {
            self.stop_preview();
        }
        let texting = self.state.borrow().messages_open;
        if texting.is_some() && texting != selected {
            self.close_messages();
        }
        match self.selected_device() {
            Some(d) => {
                self.content.set_title(&d.device.name);
                let locks = self.state.borrow().lock.devices.contains(&d.device.id);
                self.page.show_device(&d, &staged, locks);
                self.page.show_terminals(&d, self.state.borrow().terminals.as_ref());
                self.page.avatar.set_custom_image(self.state.borrow().pictures.get(&d.device.id));
                let camera = self.state.borrow().camera.filter(|(_, id)| *id == d.device.id);
                self.page.set_camera(camera.map(|_| setup::camera_name(&d.device.name)).as_deref());
                let mic = self.state.borrow().microphone.filter(|(_, id)| *id == d.device.id);
                self.page.set_microphone(mic.map(|_| format!("Linksy Microphone ({})", d.device.name)).as_deref());
                self.meter_microphone(mic.map(|(_, id)| id));
                let previewing = self.state.borrow().preview.is_some();
                if previewing {
                    self.page.menu.set_visible(false);
                }
                let texting = self.state.borrow().messages_open.is_some();
                if texting {
                    self.page.menu.set_visible(false);
                }
                self.content_stack.set_visible_child_name(if previewing {
                    "camera"
                } else if texting {
                    "messages"
                } else {
                    "device"
                });
            }
            None => {
                self.content.set_title("Linksy");
                self.content_stack.set_visible_child_name("welcome");
            }
        }
    }

    fn render_activity(&self) {
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        state.live = self.page.show_history(&state.history, &state.progress);
    }

    // ---- sending ----

    pub fn set_staged(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let has = !paths.is_empty();
        self.state.borrow_mut().staged = paths;
        self.render_page();
        if has && self.selected_device().is_none_or(|d| !d.trusted) {
            self.toast(t!("Pick a device to send to"));
        }
    }

    /// Choose files and send them to the selected device.
    fn choose_files(self: &Rc<Self>) {
        match self.selected_device().filter(|d| d.trusted) {
            Some(d) => self.choose_files_for(d.device.id),
            None => self.toast(t!("Pick a device first")),
        }
    }

    fn choose_files_for(self: &Rc<Self>, id: Fingerprint) {
        let Some(d) = self.device(&id).filter(|d| d.trusted) else { return };
        let dialog = gtk::FileDialog::builder().title(t!("Send to {name}", name = d.device.name)).accept_label(t!("Send")).modal(true).build();
        let w = self.clone();
        let id = d.device.id;
        dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let Ok(list) = res else { return };
            let paths: Vec<PathBuf> = (0..list.n_items()).filter_map(|i| list.item(i).and_downcast::<gio::File>()).filter_map(|f| f.path()).collect();
            if !paths.is_empty() {
                w.send(id, paths);
            }
        });
    }

    fn send(self: &Rc<Self>, to: Fingerprint, paths: Vec<PathBuf>) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.send(to.to_string(), paths).await });
        glib::spawn_future_local(async move {
            match call.await {
                Ok(_) => {
                    w.state.borrow_mut().staged.clear();
                    w.render_page();
                }
                Err(e) => w.toast_error(&e, Some(to)),
            }
        });
    }

    fn send_clipboard(self: &Rc<Self>, id: Fingerprint) {
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        let w = self.clone();
        let clipboard = self.window.clipboard();
        let formats = clipboard.formats();
        if formats.contains_type(gdk::FileList::static_type()) {
            glib::spawn_future_local(async move { w.send_clipboard_files(id, name, clipboard).await });
            return;
        }
        if formats.contains_type(gdk::Texture::static_type()) {
            glib::spawn_future_local(async move { w.send_clipboard_image(id, name, clipboard).await });
            return;
        }
        glib::spawn_future_local(async move {
            let text = match clipboard.read_text_future().await {
                Ok(Some(t)) if !t.is_empty() => t.to_string(),
                _ => return w.toast(t!("There's no text on the clipboard")),
            };
            let Some(c) = w.client() else { return };
            match w.bridge.call(&c, move |c| async move { c.send_clipboard(id.to_string(), text).await }).await {
                Ok(()) => w.toast(&t!("Clipboard sent to {name}", name)),
                Err(e) => w.toast_error(&e, Some(id)),
            }
        });
    }

    /// Copied files: one small file goes on the device's clipboard, more go as a transfer.
    async fn send_clipboard_files(self: &Rc<Self>, id: Fingerprint, name: String, clipboard: gdk::Clipboard) {
        let paths: Vec<PathBuf> = match clipboard.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await {
            Ok(v) => v.get::<gdk::FileList>().map(|l| l.files().iter().filter_map(|f| f.path()).collect()).unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        if paths.is_empty() {
            return self.toast(t!("There's nothing to send on the clipboard"));
        }
        let small = paths.len() == 1 && std::fs::metadata(&paths[0]).is_ok_and(|m| m.is_file() && m.len() <= MAX_CLIPBOARD_FILE);
        if !small || !self.device(&id).is_some_and(|d| can(&d.device, "clipboard_file")) {
            return self.send(id, paths);
        }
        self.send_clipboard_file(id, name, paths.into_iter().next().unwrap()).await;
    }

    /// A copied image (a screenshot…), as a PNG on the device's clipboard.
    async fn send_clipboard_image(self: &Rc<Self>, id: Fingerprint, name: String, clipboard: gdk::Clipboard) {
        if !self.device(&id).is_some_and(|d| can(&d.device, "clipboard_file")) {
            return self.toast(&t!("{name} can't take images on its clipboard yet", name));
        }
        let Ok(texture) = clipboard.read_texture_future().await else {
            return self.toast(t!("Couldn't read the image on the clipboard"));
        };
        let Some(texture) = texture else { return self.toast(t!("There's nothing to send on the clipboard")) };
        let path = glib::user_cache_dir().join("linksy").join("Clipboard image.png");
        let saved = std::fs::create_dir_all(path.parent().unwrap()).and_then(|()| std::fs::write(&path, texture.save_to_png_bytes()));
        if saved.is_err() {
            return self.toast(t!("Couldn't read the image on the clipboard"));
        }
        self.send_clipboard_file(id, name, path).await;
    }

    async fn send_clipboard_file(self: &Rc<Self>, id: Fingerprint, name: String, path: PathBuf) {
        let Some(c) = self.client() else { return };
        match self.bridge.call(&c, move |c| async move { c.send_clipboard_file(id.to_string(), path).await }).await {
            Ok(_) => self.toast(&t!("Clipboard sent to {name}", name)),
            Err(e) => self.toast_error(&e, Some(id)),
        }
    }

    fn ping(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        let call = self.bridge.call(&c, move |c| async move { c.notify(id.to_string(), "Ping", "").await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(()) => w.toast(&t!("Pinged {name}", name)),
                Err(e) => w.toast_error(&e, Some(id)),
            }
        });
    }

    /// Ring a phone (with a Stop button while it rings), or stop it.
    fn ring(self: &Rc<Self>, id: Fingerprint, on: bool) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        let call = self.bridge.call(&c, move |c| async move { c.ring(id.to_string(), on).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(()) if on => {
                    let toast = adw::Toast::builder().title(glib::markup_escape_text(&t!("Ringing {name}…", name))).button_label(t!("Stop")).timeout(30).build();
                    let w2 = w.clone();
                    toast.connect_button_clicked(move |_| w2.ring(id, false));
                    w.toasts.add_toast(toast);
                }
                Ok(()) => {}
                Err(e) => w.toast_error(&e, Some(id)),
            }
        });
    }

    /// Open the phone in the file manager (it stays in the sidebar afterwards).
    fn browse_files(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        self.toast(&t!("Opening {name}…", name));
        let call = self.bridge.call(&c, move |c| async move { c.browse_files(id.to_string()).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = call.await {
                w.toast_error(&e, Some(id));
            }
        });
    }

    fn lock_when_away(self: &Rc<Self>, id: Fingerprint, on: bool) {
        let Some(c) = self.client() else { return };
        let mut prefs = self.state.borrow().lock.clone();
        prefs.devices.retain(|d| *d != id);
        if on {
            prefs.devices.push(id);
        }
        let next = prefs.clone();
        let call = self.bridge.call(&c, move |c| async move { c.set_lock_prefs(next).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(()) => w.state.borrow_mut().lock = prefs,
                Err(e) => w.toast_error(&e, Some(id)),
            }
            w.refresh_devices();
        });
    }

    fn cancel(self: &Rc<Self>, transfer: Uuid) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.cancel(transfer).await });
        glib::spawn_future_local(async move {
            let _ = call.await;
        });
    }

    fn clear_history(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.clear_history(Some(id.to_string())).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            if call.await.is_ok() {
                w.state.borrow_mut().history.clear();
                w.render_activity();
            }
        });
    }

    fn open_received(&self, saved: &[PathBuf]) {
        let target = match saved {
            [one] => one.clone(),
            many => match many.first().and_then(|p| p.parent()) {
                Some(dir) => dir.to_path_buf(),
                None => return,
            },
        };
        gtk::FileLauncher::new(Some(&gio::File::for_path(target))).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
    }

    // ---- forgetting ----

    fn confirm_forget(self: &Rc<Self>, id: Fingerprint, name: &str) {
        let dialog = adw::AlertDialog::new(Some(&t!("Forget {name}?", name)), Some(t!("You'll need to pair again before sharing with this device.")));
        dialog.add_responses(&[("cancel", t!("Cancel")), ("forget", t!("Forget"))]);
        dialog.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let w = self.clone();
        dialog.connect_response(Some("forget"), move |_, _| {
            let Some(c) = w.client() else { return };
            let call = w.bridge.call(&c, move |c| async move { c.forget(id.to_string()).await });
            let w = w.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = call.await {
                    w.toast_error(&e, None);
                }
                w.select(None);
                w.split.set_show_sidebar(true);
                w.refresh_devices();
            });
        });
        dialog.present(Some(&self.window));
    }

    // ---- pairing ----

    fn pair(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.pair(id.to_string()).await });
        glib::spawn_future_local(async move {
            if let Err(e) = call.await
                && !e.contains("declined")
            {
                w.toast_error(&e, Some(id));
            }
            w.refresh_devices();
        });
    }

    /// For networks that block discovery: pair with a device by its IP address.
    fn ask_pair_address(self: &Rc<Self>) {
        let dialog = adw::AlertDialog::new(Some(t!("Pair by Address")), Some(t!("If a device doesn't show up, enter the address shown in its Linksy Preferences.")));
        let entry = gtk::Entry::builder().placeholder_text("192.168.1.20").activates_default(true).input_purpose(gtk::InputPurpose::Url).build();
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", t!("Cancel")), ("pair", t!("Pair"))]);
        dialog.set_response_appearance("pair", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("pair"));
        dialog.set_close_response("cancel");
        dialog.set_response_enabled("pair", false);
        let d = dialog.clone();
        entry.connect_changed(move |e| d.set_response_enabled("pair", fmt::peer_address(&e.text()).is_some()));
        let (w, e) = (self.clone(), entry.clone());
        dialog.connect_response(Some("pair"), move |_, _| {
            let Some(addr) = fmt::peer_address(&e.text()) else { return };
            let Some(c) = w.client() else { return };
            let call = w.bridge.call(&c, move |c| async move { c.pair(addr.to_string()).await });
            let w = w.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = call.await
                    && !e.contains("declined")
                {
                    w.toast_error(&e, None);
                }
                w.refresh_devices();
            });
        });
        dialog.present(Some(&self.window));
        entry.grab_focus();
    }

    fn show_pairing(self: &Rc<Self>, request: Uuid, device: DeviceInfo, code: String, incoming: bool, older: bool) {
        let (heading, body) = if incoming {
            (t!("{name} Wants to Pair", name = device.name), t!("Pair only if {name} shows this same code.", name = device.name))
        } else {
            (t!("Pair with {name}", name = device.name), t!("Check that {name} shows this same code, then confirm on both devices.", name = device.name))
        };
        let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
        let code_label = gtk::Label::builder().label(fmt::spaced_code(&code)).css_classes(["pair-code"]).build();
        let extra = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        extra.append(&code_label);
        if older {
            let hint = t!("Updating Linksy on {name} makes pairing even safer.", name = device.name);
            extra.append(&gtk::Label::builder().label(hint).wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label", "caption"]).build());
        }
        dialog.set_extra_child(Some(&extra));
        dialog.add_responses(&[("decline", if incoming { t!("Decline") } else { t!("Cancel") }), ("accept", t!("Pair"))]);
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("accept"));
        dialog.set_close_response("decline");
        let w = Rc::downgrade(self);
        let id = device.id;
        dialog.connect_response(None, move |_, response| {
            let Some(w) = w.upgrade() else { return };
            // Closed by us because the request ended elsewhere: nothing to answer.
            if w.state.borrow_mut().pair_dialogs.remove(&id).is_none() {
                return;
            }
            w.respond(request, response == "accept");
            if response == "accept" {
                w.toast(t!("Waiting for the other device to confirm…"));
            }
        });
        self.state.borrow_mut().pair_dialogs.insert(id, dialog.clone());
        dialog.present(Some(&self.window));
    }

    fn respond(self: &Rc<Self>, request: Uuid, accept: bool) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.respond(request, accept).await });
        glib::spawn_future_local(async move {
            let _ = call.await;
        });
    }

    // ---- incoming offers ----

    fn show_offer(self: &Rc<Self>, transfer: Uuid, from: &DeviceInfo, files: &[FileEntry], total: u64) {
        let (id, from) = (from.id, from.name.as_str());
        let what = match files {
            [one] => fmt::file_name(&one.path),
            many => fmt::plural(many.len(), "item"),
        };
        let dialog = adw::AlertDialog::new(Some(&t!("{name} Wants to Share", name = from)), Some(&format!("{what} · {}", fmt::size(total))));
        if files.len() > 1 {
            let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["offer-list", "card"]).build();
            for f in files.iter().take(6) {
                list.append(&gtk::Label::builder().label(&f.path).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["caption"]).build());
            }
            if files.len() > 6 {
                list.append(&gtk::Label::builder().label(t!("and {count} more", count = files.len() - 6)).xalign(0.0).css_classes(["caption", "dim-label"]).build());
            }
            dialog.set_extra_child(Some(&list));
        }
        dialog.add_responses(&[("decline", t!("Decline")), ("always", t!("Always Accept")), ("accept", t!("Accept"))]);
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("accept"));
        dialog.set_close_response("decline");
        let w = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            let Some(w) = w.upgrade() else { return };
            if w.state.borrow_mut().offer_dialogs.remove(&transfer).is_none() {
                return;
            }
            if response == "always" {
                w.auto_accept(id, true);
            }
            w.respond(transfer, response != "decline");
        });
        self.state.borrow_mut().offer_dialogs.insert(transfer, dialog.clone());
        dialog.present(Some(&self.window));
    }

    /// Close a prompt whose request was answered or ended elsewhere. The dialog leaves the
    /// map first (and the borrow ends) because closing fires its response handler.
    fn close_pair_dialog(&self, id: &Fingerprint) {
        let dialog = self.state.borrow_mut().pair_dialogs.remove(id);
        if let Some(d) = dialog {
            d.force_close();
        }
    }

    fn close_offer_dialog(&self, transfer: &Uuid) {
        let dialog = self.state.borrow_mut().offer_dialogs.remove(transfer);
        if let Some(d) = dialog {
            d.force_close();
        }
    }

    // ---- feedback ----

    /// Errors from the service, worded for people: names the device when it can.
    fn toast_error(&self, error: &str, about: Option<Fingerprint>) {
        let name = about.and_then(|id| self.device(&id).map(|d| d.device.name));
        let text = match (error, name) {
            ("device isn't reachable right now", Some(n)) => t!("{name} isn't reachable. Check that it's on and on this network.", name = n),
            (e, _) => fmt::capitalize(e),
        };
        self.toast(&text);
    }

    pub fn toast(&self, text: &str) {
        let toast = adw::Toast::new(&glib::markup_escape_text(text));
        toast.set_timeout(3);
        self.toasts.add_toast(toast);
    }

    /// Linksy crashed last time: offer the logs.
    pub fn crash_toast(&self) {
        let toast = adw::Toast::builder().title(t!("Linksy Quit Unexpectedly")).button_label(t!("Save Logs…")).action_name("app.save-logs").timeout(0).build();
        self.toasts.add_toast(toast);
    }

    /// Save logs.txt where the person chooses.
    pub fn save_logs(&self) {
        let dialog = gtk::FileDialog::builder().title(t!("Save Logs")).initial_name("logs.txt").modal(true).build();
        let toasts = self.toasts.clone();
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |file| {
            let Some(path) = file.ok().and_then(|f| f.path()) else { return };
            glib::spawn_future_local(async move {
                let logs = gio::spawn_blocking(|| linksy_logs::collect(&linksy_upgrade::revision())).await.unwrap_or_default();
                let text = match std::fs::write(&path, logs) {
                    Ok(()) => t!("Logs saved. Attach logs.txt to your bug report.").to_string(),
                    Err(e) => t!("Couldn't save the logs: {error}", error = e.to_string()),
                };
                toasts.add_toast(adw::Toast::new(&text));
            });
        });
    }

    /// Something failed for a part this computer lacks: say so, with Finish Setup.
    pub(super) fn missing_part_toast(&self, text: &str) {
        let toast = adw::Toast::builder().title(glib::markup_escape_text(text)).button_label(t!("Finish Setup")).action_name("win.setup").timeout(8).build();
        self.toasts.add_toast(toast);
    }
}

/// A spinning spinner (GTK's: libadwaita's needs 1.6, newer than Ubuntu 24.04's).
pub(super) fn spinner() -> gtk::Spinner {
    gtk::Spinner::builder().spinning(true).build()
}

fn main_menu() -> gtk::MenuButton {
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    section.append(Some(t!("Add Device…")), Some("win.add-device"));
    section.append(Some(t!("Pair by Address…")), Some("win.pair-address"));
    section.append(Some(t!("Connect by USB Cable…")), Some("win.usb-cable"));
    section.append(Some(t!("Open Received Files")), Some("app.open-downloads"));
    menu.append_section(None, &section);
    let section = gio::Menu::new();
    section.append(Some(t!("Preferences")), Some("win.preferences"));
    section.append(Some(t!("Finish Setup…")), Some("win.setup"));
    section.append(Some(t!("What's New")), Some("win.news"));
    section.append(Some(t!("Check for Updates…")), Some("win.upgrade"));
    section.append(Some(t!("Keyboard Shortcuts")), Some("win.show-help-overlay"));
    section.append(Some(t!("About Linksy")), Some("app.about"));
    menu.append_section(None, &section);
    gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).tooltip_text(t!("Main Menu")).build()
}

/// A paired device in the sidebar: avatar with a presence dot, name, state.
fn sidebar_row(d: &DeviceView, picture: Option<&gdk::Texture>) -> gtk::ListBoxRow {
    let available = d.connected || d.nearby;
    let avatar = adw::Avatar::builder().size(36).text(&d.device.name).show_initials(true).build();
    avatar.set_custom_image(picture);
    let badge = gtk::Overlay::builder().child(&avatar).build();
    let dot = gtk::Box::builder().halign(gtk::Align::End).valign(gtk::Align::End).css_classes(["status-dot"]).build();
    if !available {
        dot.add_css_class("away");
    }
    badge.add_overlay(&dot);
    let name = gtk::Label::builder().label(&d.device.name).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    let state = match fmt::battery(d) {
        Some(b) => format!("{} · {b}", status_text(d)),
        None => status_text(d).to_string(),
    };
    let status = gtk::Label::builder().label(&state).xalign(0.0).css_classes(["caption", "dim-label"]).build();
    let labels = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).hexpand(true).build();
    labels.append(&name);
    labels.append(&status);
    let b = gtk::Box::builder().spacing(10).margin_top(4).margin_bottom(4).build();
    b.append(&badge);
    b.append(&labels);
    let row = gtk::ListBoxRow::builder().child(&b).build();
    row.set_widget_name(&d.device.id.to_string());
    if !available {
        row.add_css_class("away-row");
    }
    row
}

fn status_text(d: &DeviceView) -> &'static str {
    match (d.trusted, d.connected, d.nearby) {
        (true, true, _) if d.usb => t!("Connected by USB"),
        (true, true, _) if d.away => t!("Connected away from home"),
        (true, true, _) => t!("Connected"),
        (true, false, true) => t!("Nearby"),
        (true, false, false) => t!("Not nearby"),
        (false, _, _) => t!("Nearby · Not paired"),
    }
}

/// Bytes a second, over the last few seconds.
fn speed(p: &Progress) -> f64 {
    match (p.samples.first(), p.samples.last()) {
        (Some((t0, b0)), Some((t1, b1))) if t1 > t0 => (b1 - b0) as f64 / t1.duration_since(*t0).as_secs_f64(),
        _ => 0.0,
    }
}

fn progress_text(p: &Progress) -> String {
    let speed = speed(p);
    let mut text = t!("{done} of {total}", done = fmt::size(p.bytes), total = fmt::size(p.total));
    if speed > 0.0 && p.bytes < p.total {
        let left = Duration::from_secs_f64((p.total - p.bytes) as f64 / speed);
        text.push_str(&format!(" · {} · {}", t!("{size}/s", size = fmt::size(speed as u64)), fmt::remaining(left)));
    }
    text
}

fn kind_icon(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Desktop | DeviceKind::Laptop => "computer-symbolic",
        DeviceKind::Phone => "phone-symbolic",
        DeviceKind::Tablet => "tablet-symbolic",
    }
}

/// "Linux laptop", "Android phone"…
fn kind_name(d: &DeviceInfo) -> String {
    let kind = match d.kind {
        DeviceKind::Desktop => t!("desktop"),
        DeviceKind::Laptop => t!("laptop"),
        DeviceKind::Phone => t!("phone"),
        DeviceKind::Tablet => t!("tablet"),
    };
    let os = match d.os.as_str() {
        "linux" => "Linux",
        "android" => "Android",
        "macos" => "macOS",
        "windows" => "Windows",
        "ios" => "iOS",
        other => other,
    };
    if os.is_empty() { fmt::capitalize(kind) } else { t!("{os} {kind}", os, kind) }
}

fn file_list_paths(value: &glib::Value) -> Vec<PathBuf> {
    value.get::<gdk::FileList>().map(|l| l.files().iter().filter_map(|f| f.path()).collect()).unwrap_or_default()
}

/// Lets received files be dragged into a file manager, an email, a chat…
fn drag_files(paths: &[PathBuf]) -> gtk::DragSource {
    let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
    let source = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
    source.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value())));
    source.connect_drag_begin(|source, _| {
        let icon = gtk::IconTheme::for_display(&source.widget().expect("drag source widget").display())
            .lookup_icon("text-x-generic", &[], 32, 1, gtk::TextDirection::None, gtk::IconLookupFlags::empty());
        source.set_icon(Some(&icon), 0, 0);
    });
    source
}

fn shortcuts_window() -> gtk::ShortcutsWindow {
    const UI: &str = r#"<interface>
  <object class="GtkShortcutsWindow" id="shortcuts">
    <property name="modal">1</property>
    <child>
      <object class="GtkShortcutsSection">
        <property name="section-name">shortcuts</property>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">Sharing</property>
            <child><object class="GtkShortcutsShortcut"><property name="title">Send Files to the Open Device</property><property name="accelerator">&lt;Control&gt;o</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">View the Open Device's Screen</property><property name="accelerator">&lt;Control&gt;&lt;Shift&gt;s</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Add Device</property><property name="accelerator">&lt;Control&gt;n</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Show or Hide the Device List</property><property name="accelerator">F9</property></object></child>
          </object>
        </child>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">General</property>
            <child><object class="GtkShortcutsShortcut"><property name="title">Preferences</property><property name="accelerator">&lt;Control&gt;comma</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Keyboard Shortcuts</property><property name="accelerator">&lt;Control&gt;question</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Close Window</property><property name="accelerator">&lt;Control&gt;w</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Quit</property><property name="accelerator">&lt;Control&gt;q</property></object></child>
          </object>
        </child>
      </object>
    </child>
  </object>
</interface>"#;
    // The titles in the language in use, as XML text.
    let titles = [
        ("Sharing", t!("Sharing")),
        ("Send Files to the Open Device", t!("Send Files to the Open Device")),
        ("View the Open Device's Screen", t!("View the Open Device's Screen")),
        ("Add Device", t!("Add Device")),
        ("Show or Hide the Device List", t!("Show or Hide the Device List")),
        ("General", t!("General")),
        ("Preferences", t!("Preferences")),
        ("Keyboard Shortcuts", t!("Keyboard Shortcuts")),
        ("Close Window", t!("Close Window")),
        ("Quit", t!("Quit")),
    ];
    let mut ui = UI.to_string();
    for (english, title) in titles {
        ui = ui.replace(&format!(">{english}<"), &format!(">{}<", glib::markup_escape_text(title)));
    }
    gtk::Builder::from_string(&ui).object("shortcuts").expect("shortcuts window")
}

/// Largest file that goes on another device's clipboard (bigger ones go as a transfer).
const MAX_CLIPBOARD_FILE: u64 = 10 << 20;

/// Whether a device does `feature` for others: what it says, or for devices that don't say,
/// everything except what an Android phone can't (Linksy for Android ships outside the
/// Play Store, where texting, notification access and input control are blocked).
pub(crate) fn can(d: &DeviceInfo, feature: &str) -> bool {
    if !d.features.is_empty() {
        return d.features.iter().any(|f| f == feature);
    }
    !(d.os == "android" && matches!(feature, "sms" | "notifications" | "media_state" | "control" | "clipboard_file" | "clipboard_watch" | "microphone" | "locate"))
}
