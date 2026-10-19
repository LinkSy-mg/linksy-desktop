//! The Linksy window: devices in a sidebar, and a page per device with what you can do
//! with it and everything shared with it so far. Everything updates live from the
//! service's events and history.

mod invite;
mod page;
mod prefs;
mod screen;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use linksy_ipc::linksy_core::{DeviceInfo, DeviceKind, DeviceView, Event, FileEntry, Fingerprint, Uuid};
use linksy_ipc::{Client, HistoryEntry};

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
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    bridge: Bridge,
    state: RefCell<State>,
    toasts: adw::ToastOverlay,
    stack: gtk::Stack,
    /// "Studio is watching your screen" with Stop.
    sharing_banner: adw::Banner,
    split: adw::NavigationSplitView,
    // Sidebar
    paired_list: gtk::ListBox,
    nearby_group: gtk::Box,
    nearby_list: gtk::ListBox,
    no_devices: gtk::Box,
    me_label: gtk::Label,
    // Content
    content: adw::NavigationPage,
    content_stack: gtk::Stack,
    page: page::DevicePage,
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
        let sidebar_header = adw::HeaderBar::builder().title_widget(&brand).build();
        sidebar_header.pack_end(&main_menu());

        let paired_list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        let nearby_list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["navigation-sidebar"]).build();
        let nearby_group = gtk::Box::builder().orientation(gtk::Orientation::Vertical).visible(false).build();
        nearby_group.append(&gtk::Label::builder().label("Nearby, Not Paired").xalign(0.0).css_classes(["sidebar-heading"]).build());
        nearby_group.append(&nearby_list);

        let no_devices = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["sidebar-empty"]).visible(false).build();
        no_devices.append(&mark::new(40, true));
        no_devices.append(&gtk::Label::builder().label("Looking for devices…").css_classes(["heading"]).build());
        no_devices.append(
            &gtk::Label::builder()
                .label("Open Linksy on another device on this network, or add one.")
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

        let add = gtk::Button::builder().action_name("win.add-device").css_classes(["pill"]).build();
        add.set_child(Some(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label("Add Device").build()));
        let me_label = gtk::Label::builder().css_classes(["dim-label", "caption"]).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let footer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["sidebar-footer"]).build();
        footer.append(&add);
        footer.append(&me_label);

        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&sidebar_header);
        sidebar_view.set_content(Some(&sidebar_scroll));
        sidebar_view.add_bottom_bar(&footer);
        let sidebar = adw::NavigationPage::builder().title("Linksy").tag("sidebar").child(&sidebar_view).build();

        // ---- content ----
        let page = page::DevicePage::new();
        let welcome_body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).halign(gtk::Align::Center).build();
        welcome_body.append(&gtk::Button::builder().label("Add Device").action_name("win.add-device").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).build());
        let welcome_mark = mark::new(96, false);
        welcome_mark.set_halign(gtk::Align::Center);
        let welcome_inner = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).valign(gtk::Align::Center).build();
        welcome_inner.append(&welcome_mark);
        welcome_inner.append(
            &adw::StatusPage::builder().title("Your Devices, Connected").description("Pick a device to share with it, or add a new one.").child(&welcome_body).build(),
        );
        let content_stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        content_stack.add_named(&welcome_inner, Some("welcome"));
        content_stack.add_named(&page.root, Some("device"));
        let content_header = adw::HeaderBar::builder().show_title(false).build();
        content_header.pack_end(&page.menu);
        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&content_header);
        content_view.set_content(Some(&content_stack));
        let content = adw::NavigationPage::builder().title("Linksy").tag("content").child(&content_view).build();

        let split = adw::NavigationSplitView::builder().sidebar(&sidebar).content(&content).min_sidebar_width(240.0).max_sidebar_width(300.0).build();

        // ---- offline / loading ----
        let start = gtk::Button::builder().label("Start Linksy").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).action_name("win.start-service").build();
        let offline = adw::StatusPage::builder()
            .icon_name("network-offline-symbolic")
            .title("Linksy Isn't Running")
            .description("The background service lets nearby devices find this one and receives files, even when this window is closed.")
            .child(&start)
            .build();
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        stack.add_named(&split, Some("main"));
        stack.add_named(&offline, Some("offline"));
        stack.add_named(&adw::Spinner::builder().width_request(32).height_request(32).halign(gtk::Align::Center).valign(gtk::Align::Center).build(), Some("loading"));
        stack.set_visible_child_name("loading");

        let sharing_banner = adw::Banner::builder().button_label("Stop Sharing").revealed(false).build();
        let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        outer.append(&sharing_banner);
        stack.set_vexpand(true);
        outer.append(&stack);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&outer));
        window.set_content(Some(&toasts));

        // Medium windows put the device actions two by two; narrow ones also show one
        // pane at a time. (The last matching breakpoint applies, so narrow repeats medium.)
        let two_by_two = |b: &adw::Breakpoint| {
            b.add_setter(&page.tiles, "min-children-per-line", Some(&2u32.to_value()));
            b.add_setter(&page.tiles, "max-children-per-line", Some(&2u32.to_value()));
        };
        let medium = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 820sp").expect("breakpoint"));
        two_by_two(&medium);
        window.add_breakpoint(medium);
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 620sp").expect("breakpoint"));
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        two_by_two(&narrow);
        window.add_breakpoint(narrow);

        let win = Rc::new(Self {
            window,
            bridge: Bridge::new(),
            state: RefCell::default(),
            toasts,
            stack,
            sharing_banner,
            split,
            paired_list,
            nearby_group,
            nearby_list,
            no_devices,
            me_label,
            content,
            content_stack,
            page,
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
        win.setup_drop();
        win.connect();
        win
    }

    // ---- setup ----

    fn setup_actions(self: &Rc<Self>) {
        type Run = fn(&Rc<Window>);
        let actions: [(&str, Run); 6] = [
            ("choose-files", |w| w.choose_files()),
            ("start-service", |w| w.start_service()),
            ("preferences", |w| w.show_preferences()),
            ("add-device", |w| w.show_invite()),
            ("pair-address", |w| w.ask_pair_address()),
            ("view-screen", |w| match w.selected_device().filter(|d| d.trusted) {
                Some(d) => w.view_screen(d.device.id),
                None => w.toast("Pick a device first"),
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

    fn setup_sidebar(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.paired_list.connect_row_selected(move |_, row| {
            let Some(w) = w.upgrade() else { return };
            let Some(id) = row.and_then(|r| r.widget_name().parse::<Fingerprint>().ok()) else { return };
            w.select(Some(id));
        });
        // Clicking the selected row again (collapsed layout) should still open its page.
        let w = Rc::downgrade(self);
        self.paired_list.connect_row_activated(move |_, _| {
            if let Some(w) = w.upgrade() {
                w.split.set_show_content(true);
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
                page::Action::ViewScreen => w.view_screen(id),
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
                page::Action::Cancel(t) => w.cancel(t),
                page::Action::Open(paths) => w.open_received(&paths),
                page::Action::ShowInFolder(path) => {
                    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
                    launcher.open_containing_folder(Some(&w.window), gio::Cancellable::NONE, |_| {});
                }
                page::Action::Copy(text) => {
                    w.window.clipboard().set_text(&text);
                    w.toast("Copied");
                }
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
                    while let Ok(update) = updates.recv().await {
                        match update {
                            Update::Event(ev) => w.handle_event(ev),
                            Update::History(h) => w.history_changed(h),
                        }
                    }
                    // Service went away.
                    w.state.borrow_mut().client = None;
                    w.stack.set_visible_child_name("offline");
                }
                Err(_) => w.stack.set_visible_child_name("offline"),
            }
        });
    }

    fn start_service(self: &Rc<Self>) {
        use std::process::{Command, Stdio};
        // The installed user service first, so it stays managed (restarts, logs); else
        // run linksyd from next to this app, or from PATH.
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
            self.toast(&format!("Could not start the service: {e}"));
            return;
        }
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
        let call = self.bridge.call(&c, |c| async move { c.status().await });
        glib::spawn_future_local(async move {
            if let Ok(s) = call.await {
                w.me_label.set_label(&format!("This device: {}", s.device.name));
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
                w.state.borrow_mut().devices = devices;
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
            Event::PairingRequest { request, device, code, incoming } => self.show_pairing(request, device, code, incoming),
            Event::Paired { device } => {
                self.close_pair_dialog(&device.id);
                self.close_invite_dialog();
                self.toast(&format!("Paired with {}", device.name));
                // Open the new device's page.
                self.state.borrow_mut().selected = Some(device.id);
                self.refresh_devices();
                self.refresh_history();
            }
            Event::PairingFailed { device, reason } => {
                self.close_pair_dialog(&device.id);
                if reason.contains("other device") {
                    self.toast(&format!("{} didn't confirm the code", device.name));
                }
                self.render_page();
            }
            Event::TransferRequest { transfer, from, files, total } => self.show_offer(transfer, &from.name, &files, total),
            Event::TransferStarted { transfer, total, .. } => {
                // Accepted elsewhere (desktop notification, another window): drop the prompt.
                self.close_offer_dialog(&transfer);
                self.state.borrow_mut().progress.insert(transfer, Progress { total, ..Default::default() });
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
            Event::SessionStarted { session, peer, sharing: true, .. } => self.sharing_started(session, &peer),
            Event::SessionStarted { .. } => {}
            Event::SessionEnded { session } => self.session_ended(session),
            Event::Notification { from, title, body } => {
                let text = if body.is_empty() { format!("{}: {title}", from.name) } else { format!("{}: {title} — {body}", from.name) };
                self.toast(&text);
            }
            Event::Clipboard { from, text } => {
                // The service copies it too; a focused window can also do it directly.
                if self.window.is_active() {
                    self.window.clipboard().set_text(&text);
                }
                self.toast(&format!("Copied from {}", from.name));
            }
        }
    }

    fn history_changed(self: &Rc<Self>, entry: HistoryEntry) {
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
    }

    // ---- sidebar ----

    fn render_sidebar(self: &Rc<Self>) {
        let (paired, nearby, selected): (Vec<DeviceView>, Vec<DeviceView>, Option<Fingerprint>) = {
            let state = self.state.borrow();
            let (p, n) = state.devices.iter().cloned().partition(|d| d.trusted);
            (p, n, state.selected)
        };
        self.paired_list.remove_all();
        let mut to_select = None;
        for d in &paired {
            let row = sidebar_row(d);
            self.paired_list.append(&row);
            if Some(d.device.id) == selected {
                to_select = Some(row);
            }
        }
        self.nearby_list.remove_all();
        for d in &nearby {
            self.nearby_list.append(&self.nearby_row(d));
        }
        self.render_invite_nearby();
        self.paired_list.set_visible(!paired.is_empty());
        self.nearby_group.set_visible(!nearby.is_empty());
        self.no_devices.set_visible(paired.is_empty() && nearby.is_empty());
        match to_select {
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

    fn nearby_row(self: &Rc<Self>, d: &DeviceView) -> gtk::ListBoxRow {
        let avatar = adw::Avatar::builder().size(32).text(&d.device.name).show_initials(true).build();
        let name = gtk::Label::builder().label(&d.device.name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let kind = gtk::Label::builder().label(kind_name(&d.device)).xalign(0.0).css_classes(["caption", "dim-label"]).build();
        let labels = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).hexpand(true).build();
        labels.append(&name);
        labels.append(&kind);
        let pair = gtk::Button::builder().label("Pair").valign(gtk::Align::Center).css_classes(["pill", "small"]).build();
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
            self.render_page();
            self.render_activity();
            self.refresh_history();
        }
        if id.is_some() {
            self.split.set_show_content(true);
        }
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
        let staged = self.state.borrow().staged.clone();
        match self.selected_device() {
            Some(d) => {
                self.content.set_title(&d.device.name);
                self.page.show_device(&d, &staged);
                self.content_stack.set_visible_child_name("device");
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
            self.toast("Pick a device to send to");
        }
    }

    /// Choose files and send them to the selected device.
    fn choose_files(self: &Rc<Self>) {
        let Some(d) = self.selected_device().filter(|d| d.trusted) else {
            self.toast("Pick a device first");
            return;
        };
        let dialog = gtk::FileDialog::builder().title(format!("Send to {}", d.device.name)).accept_label("Send").modal(true).build();
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
        glib::spawn_future_local(async move {
            let text = match w.window.clipboard().read_text_future().await {
                Ok(Some(t)) if !t.is_empty() => t.to_string(),
                _ => return w.toast("There's no text on the clipboard"),
            };
            let Some(c) = w.client() else { return };
            match w.bridge.call(&c, move |c| async move { c.send_clipboard(id.to_string(), text).await }).await {
                Ok(()) => w.toast(&format!("Clipboard sent to {name}")),
                Err(e) => w.toast_error(&e, Some(id)),
            }
        });
    }

    fn ping(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        let call = self.bridge.call(&c, move |c| async move { c.notify(id.to_string(), "Ping", "").await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(()) => w.toast(&format!("Pinged {name}")),
                Err(e) => w.toast_error(&e, Some(id)),
            }
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
        let dialog = adw::AlertDialog::new(Some(&format!("Forget {name}?")), Some("You'll need to pair again before sharing with this device."));
        dialog.add_responses(&[("cancel", "Cancel"), ("forget", "Forget")]);
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
                w.split.set_show_content(false);
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
        let dialog = adw::AlertDialog::new(Some("Pair by Address"), Some("If a device doesn't show up, enter the address shown in its Linksy Preferences."));
        let entry = gtk::Entry::builder().placeholder_text("192.168.1.20").activates_default(true).input_purpose(gtk::InputPurpose::Url).build();
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Cancel"), ("pair", "Pair")]);
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

    fn show_pairing(self: &Rc<Self>, request: Uuid, device: DeviceInfo, code: String, incoming: bool) {
        let (heading, body) = if incoming {
            (format!("{} Wants to Pair", device.name), format!("Pair only if {} shows this same code.", device.name))
        } else {
            (format!("Pair with {}", device.name), format!("Check that {} shows this same code, then confirm on both devices.", device.name))
        };
        let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
        let code_label = gtk::Label::builder().label(fmt::spaced_code(&code)).css_classes(["pair-code"]).build();
        dialog.set_extra_child(Some(&code_label));
        dialog.add_responses(&[("decline", if incoming { "Decline" } else { "Cancel" }), ("accept", "Pair")]);
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
                w.toast("Waiting for the other device to confirm…");
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

    fn show_offer(self: &Rc<Self>, transfer: Uuid, from: &str, files: &[FileEntry], total: u64) {
        let what = match files {
            [one] => fmt::file_name(&one.path),
            many => fmt::plural(many.len(), "item"),
        };
        let dialog = adw::AlertDialog::new(Some(&format!("{from} Wants to Share")), Some(&format!("{what} · {}", fmt::size(total))));
        if files.len() > 1 {
            let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["offer-list", "card"]).build();
            for f in files.iter().take(6) {
                list.append(&gtk::Label::builder().label(&f.path).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["caption"]).build());
            }
            if files.len() > 6 {
                list.append(&gtk::Label::builder().label(format!("and {} more", files.len() - 6)).xalign(0.0).css_classes(["caption", "dim-label"]).build());
            }
            dialog.set_extra_child(Some(&list));
        }
        dialog.add_responses(&[("decline", "Decline"), ("accept", "Accept")]);
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("accept"));
        dialog.set_close_response("decline");
        let w = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            let Some(w) = w.upgrade() else { return };
            if w.state.borrow_mut().offer_dialogs.remove(&transfer).is_none() {
                return;
            }
            w.respond(transfer, response == "accept");
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
            ("device isn't reachable right now", Some(n)) => format!("{n} isn't reachable. Check that it's on and on this network."),
            (e, _) => fmt::capitalize(e),
        };
        self.toast(&text);
    }

    pub fn toast(&self, text: &str) {
        let toast = adw::Toast::new(&glib::markup_escape_text(text));
        toast.set_timeout(3);
        self.toasts.add_toast(toast);
    }
}

fn main_menu() -> gtk::MenuButton {
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    section.append(Some("Add Device…"), Some("win.add-device"));
    section.append(Some("Pair by Address…"), Some("win.pair-address"));
    section.append(Some("Open Downloads Folder"), Some("app.open-downloads"));
    menu.append_section(None, &section);
    let section = gio::Menu::new();
    section.append(Some("Preferences"), Some("win.preferences"));
    section.append(Some("Keyboard Shortcuts"), Some("win.show-help-overlay"));
    section.append(Some("About Linksy"), Some("app.about"));
    menu.append_section(None, &section);
    gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).tooltip_text("Main Menu").build()
}

/// A paired device in the sidebar: avatar with a presence dot, name, state.
fn sidebar_row(d: &DeviceView) -> gtk::ListBoxRow {
    let available = d.connected || d.nearby;
    let avatar = adw::Avatar::builder().size(36).text(&d.device.name).show_initials(true).build();
    let badge = gtk::Overlay::builder().child(&avatar).build();
    let dot = gtk::Box::builder().halign(gtk::Align::End).valign(gtk::Align::End).css_classes(["status-dot"]).build();
    if !available {
        dot.add_css_class("away");
    }
    badge.add_overlay(&dot);
    let name = gtk::Label::builder().label(&d.device.name).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    let status = gtk::Label::builder().label(status_text(d)).xalign(0.0).css_classes(["caption", "dim-label"]).build();
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
        (true, true, _) => "Connected",
        (true, false, true) => "Nearby",
        (true, false, false) => "Not nearby",
        (false, _, _) => "Nearby · Not paired",
    }
}

fn progress_text(p: &Progress) -> String {
    let speed = match (p.samples.first(), p.samples.last()) {
        (Some((t0, b0)), Some((t1, b1))) if t1 > t0 => (b1 - b0) as f64 / t1.duration_since(*t0).as_secs_f64(),
        _ => 0.0,
    };
    let mut text = format!("{} of {}", fmt::size(p.bytes), fmt::size(p.total));
    if speed > 0.0 && p.bytes < p.total {
        let left = Duration::from_secs_f64((p.total - p.bytes) as f64 / speed);
        text.push_str(&format!(" · {}/s · {}", fmt::size(speed as u64), fmt::remaining(left)));
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
        DeviceKind::Desktop => "desktop",
        DeviceKind::Laptop => "laptop",
        DeviceKind::Phone => "phone",
        DeviceKind::Tablet => "tablet",
    };
    let os = match d.os.as_str() {
        "linux" => "Linux",
        "android" => "Android",
        "macos" => "macOS",
        "windows" => "Windows",
        "ios" => "iOS",
        other => other,
    };
    if os.is_empty() { fmt::capitalize(kind) } else { format!("{os} {kind}") }
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
    gtk::Builder::from_string(UI).object("shortcuts").expect("shortcuts window")
}
