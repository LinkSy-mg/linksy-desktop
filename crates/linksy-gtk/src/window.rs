//! The Linksy window. One screen, AirDrop-style: drop or choose files, pick a device.
//! Devices and transfers update live from the service's events.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use linksy_ipc::Client;
use linksy_ipc::linksy_core::{DeviceKind, DeviceView, Direction, Event, FileEntry, Fingerprint, Uuid};

use crate::bridge::Bridge;
use crate::{fmt, mark};

struct TransferRow {
    row: gtk::ListBoxRow,
    detail: gtk::Label,
    bar: gtk::ProgressBar,
    action: gtk::Button,
    total: u64,
    direction: Direction,
    /// (time, bytes) samples for a smoothed speed estimate.
    samples: Vec<(Instant, u64)>,
    saved: Vec<PathBuf>,
    done: bool,
}

#[derive(Default)]
struct State {
    client: Option<Arc<Client>>,
    devices: Vec<DeviceView>,
    staged: Vec<PathBuf>,
    transfers: HashMap<Uuid, TransferRow>,
    /// Open consent dialogs, closed if the request ends elsewhere.
    pair_dialogs: HashMap<Fingerprint, adw::AlertDialog>,
    offer_dialogs: HashMap<Uuid, adw::AlertDialog>,
    download_dir: Option<PathBuf>,
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    bridge: Bridge,
    state: RefCell<State>,
    toasts: adw::ToastOverlay,
    title: adw::WindowTitle,
    stack: gtk::Stack,
    drop_zone: gtk::Box,
    drop_title: gtk::Label,
    drop_subtitle: gtk::Label,
    clear_button: gtk::Button,
    devices: gtk::ListBox,
    searching: gtk::Box,
    transfers_group: gtk::Box,
    transfers: gtk::ListBox,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Linksy")
            .default_width(460)
            .default_height(680)
            .width_request(360)
            .height_request(420)
            .build();

        // Header
        let title = adw::WindowTitle::new("Linksy", "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        let menu = gio::Menu::new();
        menu.append(Some("Open Downloads Folder"), Some("app.open-downloads"));
        menu.append(Some("About Linksy"), Some("app.about"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text("Main Menu")
            .build();
        header.pack_end(&menu_button);

        // Drop zone
        let drop_title = gtk::Label::builder().label("Drop files to send").css_classes(["title-2"]).wrap(true).justify(gtk::Justification::Center).build();
        let drop_subtitle = gtk::Label::builder()
            .label("Or choose files, then pick a device below.")
            .css_classes(["dim-label"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        let choose = gtk::Button::builder().label("Choose Files…").css_classes(["pill"]).halign(gtk::Align::Center).build();
        choose.set_action_name(Some("win.choose-files"));
        let clear_button = gtk::Button::builder().label("Clear").css_classes(["pill", "flat"]).halign(gtk::Align::Center).visible(false).build();
        let buttons = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).build();
        buttons.append(&choose);
        buttons.append(&clear_button);
        let drop_zone = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["drop-zone"]).build();
        drop_zone.append(&mark::new(72, false));
        drop_zone.append(&drop_title);
        drop_zone.append(&drop_subtitle);
        drop_zone.append(&buttons);

        // Devices
        let devices_heading = gtk::Label::builder().label("Devices").xalign(0.0).css_classes(["heading"]).build();
        let devices = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let searching = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["searching"]).build();
        searching.append(&mark::new(56, true));
        searching.append(&gtk::Label::builder().label("Looking for devices…").css_classes(["title-4"]).build());
        searching.append(
            &gtk::Label::builder()
                .label("Open Linksy on another device connected to the same network.")
                .css_classes(["dim-label"])
                .wrap(true)
                .justify(gtk::Justification::Center)
                .build(),
        );

        // Transfers
        let transfers = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let transfers_group = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).visible(false).build();
        transfers_group.append(&gtk::Label::builder().label("Transfers").xalign(0.0).css_classes(["heading"]).build());
        transfers_group.append(&transfers);

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(14)
            .margin_top(18)
            .margin_bottom(24)
            .margin_start(16)
            .margin_end(16)
            .build();
        content.append(&drop_zone);
        content.append(&devices_heading);
        content.append(&devices);
        content.append(&searching);
        content.append(&transfers_group);
        let clamp = adw::Clamp::builder().maximum_size(560).child(&content).build();
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build();

        // Offline page
        let start = gtk::Button::builder().label("Start Linksy").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).build();
        start.set_action_name(Some("win.start-service"));
        let offline = adw::StatusPage::builder()
            .icon_name("network-offline-symbolic")
            .title("Linksy Isn't Running")
            .description("The background service keeps your devices connected and receives files, even when this window is closed.")
            .child(&start)
            .build();

        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        stack.add_named(&scroller, Some("main"));
        stack.add_named(&offline, Some("offline"));
        stack.add_named(&adw::Spinner::builder().width_request(32).height_request(32).halign(gtk::Align::Center).valign(gtk::Align::Center).build(), Some("loading"));
        stack.set_visible_child_name("loading");

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&toasts));
        window.set_content(Some(&toolbar));

        let win = Rc::new(Self {
            window,
            bridge: Bridge::new(),
            state: RefCell::default(),
            toasts,
            title,
            stack,
            drop_zone,
            drop_title,
            drop_subtitle,
            clear_button,
            devices,
            searching,
            transfers_group,
            transfers,
        });
        win.setup_actions();
        win.setup_drop();
        win.connect();
        win
    }

    // ---- setup ----

    fn setup_actions(self: &Rc<Self>) {
        let choose = gio::SimpleAction::new("choose-files", None);
        let w = Rc::downgrade(self);
        choose.connect_activate(move |_, _| {
            if let Some(w) = w.upgrade() {
                w.choose_files(None);
            }
        });
        self.window.add_action(&choose);

        let start = gio::SimpleAction::new("start-service", None);
        let w = Rc::downgrade(self);
        start.connect_activate(move |_, _| {
            if let Some(w) = w.upgrade() {
                w.start_service();
            }
        });
        self.window.add_action(&start);

        let w = Rc::downgrade(self);
        self.clear_button.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.set_staged(Vec::new());
            }
        });
    }

    fn setup_drop(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let zone = self.drop_zone.clone();
        target.connect_enter(move |_, _, _| {
            zone.add_css_class("drop-active");
            gdk::DragAction::COPY
        });
        let zone = self.drop_zone.clone();
        target.connect_leave(move |_| zone.remove_css_class("drop-active"));
        let w = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let Some(w) = w.upgrade() else { return false };
            w.drop_zone.remove_css_class("drop-active");
            let paths = file_list_paths(value);
            if paths.is_empty() {
                return false;
            }
            w.set_staged(paths);
            true
        });
        // The whole window accepts drops into the staging area.
        self.window.add_controller(target);
    }

    // ---- service connection ----

    fn connect(self: &Rc<Self>) {
        let w = self.clone();
        glib::spawn_future_local(async move {
            match w.bridge.connect().await {
                Ok((client, events)) => {
                    w.state.borrow_mut().client = Some(client);
                    w.stack.set_visible_child_name("main");
                    w.refresh_status();
                    w.refresh_devices();
                    while let Ok(ev) = events.recv().await {
                        w.handle_event(ev);
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
        let spawned = std::process::Command::new("linksyd")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
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
                w.title.set_subtitle(&format!("Visible as {}", s.device.name));
                w.state.borrow_mut().download_dir = Some(s.download_dir);
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
                w.render_devices();
            }
        });
    }

    pub fn download_dir(&self) -> Option<PathBuf> {
        self.state.borrow().download_dir.clone()
    }

    // ---- events ----

    fn handle_event(self: &Rc<Self>, ev: Event) {
        match ev {
            Event::DeviceFound { .. } | Event::DeviceLost { .. } | Event::Connected { .. } | Event::Disconnected { .. } => self.refresh_devices(),
            Event::PairingRequest { request, device, code, incoming } => self.show_pairing(request, device, code, incoming),
            Event::Paired { device } => {
                self.close_pair_dialog(&device.id);
                self.toast(&format!("Paired with {}", device.name));
                self.refresh_devices();
            }
            Event::PairingFailed { device, reason } => {
                self.close_pair_dialog(&device.id);
                if reason.contains("other device") {
                    self.toast(&format!("{} didn't confirm the code", device.name));
                }
            }
            Event::TransferRequest { transfer, from, files, total } => self.show_offer(transfer, &from.name, &files, total),
            Event::TransferStarted { transfer, peer, direction, files, total } => {
                // Accepted elsewhere (desktop notification, another window): drop the prompt.
                self.close_offer_dialog(&transfer);
                self.add_transfer(transfer, &peer.name, direction, &files, total)
            }
            Event::TransferProgress { transfer, bytes, total } => self.update_transfer(transfer, bytes, total),
            Event::TransferComplete { transfer, saved } => self.finish_transfer(transfer, Ok(saved)),
            Event::TransferFailed { transfer, reason } => {
                self.close_offer_dialog(&transfer);
                self.finish_transfer(transfer, Err(reason));
            }
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

    // ---- staging ----

    pub fn set_staged(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let has = !paths.is_empty();
        if has {
            let (count, bytes) = fmt::measure(&paths);
            let name = paths.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.drop_title.set_label(&if paths.len() == 1 { name } else { fmt::plural(count, "item") });
            self.drop_subtitle.set_label(&format!("{} · Pick a device to send to", fmt::size(bytes)));
            self.drop_zone.add_css_class("staged");
        } else {
            self.drop_title.set_label("Drop files to send");
            self.drop_subtitle.set_label("Or choose files, then pick a device below.");
            self.drop_zone.remove_css_class("staged");
        }
        self.clear_button.set_visible(has);
        self.state.borrow_mut().staged = paths;
        self.render_devices();
    }

    /// Open the file chooser; with `to`, send the chosen files there directly.
    fn choose_files(self: &Rc<Self>, to: Option<Fingerprint>) {
        let dialog = gtk::FileDialog::builder().title("Choose Files to Send").modal(true).build();
        let w = self.clone();
        dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let Ok(list) = res else { return };
            let paths: Vec<PathBuf> = (0..list.n_items())
                .filter_map(|i| list.item(i).and_downcast::<gio::File>())
                .filter_map(|f| f.path())
                .collect();
            if paths.is_empty() {
                return;
            }
            match to {
                Some(id) => w.send(id, paths),
                None => w.set_staged(paths),
            }
        });
    }

    fn send(self: &Rc<Self>, to: Fingerprint, paths: Vec<PathBuf>) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.send(to.to_string(), paths).await });
        glib::spawn_future_local(async move {
            match call.await {
                Ok(_) => w.set_staged(Vec::new()),
                Err(e) => w.toast(&e),
            }
        });
    }

    // ---- devices ----

    fn render_devices(self: &Rc<Self>) {
        let state = self.state.borrow();
        self.devices.remove_all();
        for d in &state.devices {
            self.devices.append(&self.device_row(d, !state.staged.is_empty()));
        }
        let empty = state.devices.is_empty();
        self.devices.set_visible(!empty);
        self.searching.set_visible(empty);
    }

    fn device_row(self: &Rc<Self>, d: &DeviceView, staged: bool) -> adw::ActionRow {
        let subtitle = match (d.trusted, d.connected, d.nearby) {
            (true, true, _) => "Connected",
            (true, false, true) => "Connecting…",
            (true, false, false) => "Not nearby",
            (false, _, _) => "Nearby · Not paired",
        };
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&d.device.name).as_str()).subtitle(subtitle).build();

        let icon = gtk::Image::builder().icon_name(kind_icon(d.device.kind)).pixel_size(28).css_classes(["device-icon"]).build();
        let badge = gtk::Overlay::new();
        badge.set_child(Some(&icon));
        let dot = gtk::Box::builder().halign(gtk::Align::End).valign(gtk::Align::End).css_classes(["status-dot"]).build();
        if !d.connected {
            dot.add_css_class("away");
        }
        if d.trusted {
            badge.add_overlay(&dot);
        }
        row.add_prefix(&badge);

        let id = d.device.id;
        if !d.trusted {
            let pair = gtk::Button::builder().label("Pair").valign(gtk::Align::Center).build();
            let w = Rc::downgrade(self);
            pair.connect_clicked(move |b| {
                if let Some(w) = w.upgrade() {
                    b.set_sensitive(false);
                    w.pair(id);
                }
            });
            row.add_suffix(&pair);
            row.set_activatable_widget(Some(&pair));
        } else if d.connected {
            let send = gtk::Button::builder()
                .label(if staged { "Send" } else { "Send Files…" })
                .valign(gtk::Align::Center)
                .css_classes(if staged { vec!["suggested-action"] } else { vec![] })
                .build();
            let w = Rc::downgrade(self);
            send.connect_clicked(move |_| {
                let Some(w) = w.upgrade() else { return };
                let staged = w.state.borrow().staged.clone();
                if staged.is_empty() { w.choose_files(Some(id)) } else { w.send(id, staged) }
            });
            row.add_suffix(&send);
            row.set_activatable_widget(Some(&send));

            // Dropping files straight onto a device sends them there.
            let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
            let r = row.clone();
            target.connect_enter(move |_, _, _| {
                r.add_css_class("drop-active");
                gdk::DragAction::COPY
            });
            let r = row.clone();
            target.connect_leave(move |_| r.remove_css_class("drop-active"));
            let w = Rc::downgrade(self);
            let r = row.clone();
            target.connect_drop(move |_, value, _, _| {
                r.remove_css_class("drop-active");
                let Some(w) = w.upgrade() else { return false };
                let paths = file_list_paths(value);
                if paths.is_empty() {
                    return false;
                }
                w.send(id, paths);
                true
            });
            row.add_controller(target);
        } else {
            row.add_css_class("dim-label");
        }

        if d.trusted {
            row.add_suffix(&self.device_menu(d));
        }
        row
    }

    fn device_menu(self: &Rc<Self>, d: &DeviceView) -> gtk::MenuButton {
        let popover_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-box"]).build();
        let clip = gtk::Button::builder().label("Send Clipboard").css_classes(["flat"]).sensitive(d.connected).build();
        let ping = gtk::Button::builder().label("Send a Ping").css_classes(["flat"]).sensitive(d.connected).build();
        let forget = gtk::Button::builder().label("Forget Device").css_classes(["flat", "destructive-label"]).build();
        popover_box.append(&clip);
        popover_box.append(&ping);
        popover_box.append(&forget);
        let popover = gtk::Popover::builder().child(&popover_box).build();
        let button = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .popover(&popover)
            .tooltip_text("More")
            .build();

        let (id, name) = (d.device.id, d.device.name.clone());
        let w = Rc::downgrade(self);
        let p = popover.clone();
        clip.connect_clicked(move |_| {
            p.popdown();
            let Some(w) = w.upgrade() else { return };
            let name = name.clone();
            glib::spawn_future_local(async move {
                let text = match w.window.clipboard().read_text_future().await {
                    Ok(Some(t)) if !t.is_empty() => t.to_string(),
                    _ => return w.toast("There's no text on the clipboard"),
                };
                let Some(c) = w.client() else { return };
                match w.bridge.call(&c, move |c| async move { c.send_clipboard(id.to_string(), text).await }).await {
                    Ok(()) => w.toast(&format!("Clipboard sent to {name}")),
                    Err(e) => w.toast(&e),
                }
            });
        });

        let name = d.device.name.clone();
        let w = Rc::downgrade(self);
        let p = popover.clone();
        ping.connect_clicked(move |_| {
            p.popdown();
            let Some(w) = w.upgrade() else { return };
            let Some(c) = w.client() else { return };
            let call = w.bridge.call(&c, move |c| async move { c.notify(id.to_string(), "Ping", "").await });
            let name = name.clone();
            glib::spawn_future_local(async move {
                match call.await {
                    Ok(()) => w.toast(&format!("Pinged {name}")),
                    Err(e) => w.toast(&e),
                }
            });
        });

        let w = Rc::downgrade(self);
        let name = d.device.name.clone();
        forget.connect_clicked(move |_| {
            popover.popdown();
            if let Some(w) = w.upgrade() {
                w.confirm_forget(id, &name);
            }
        });
        button
    }

    fn confirm_forget(self: &Rc<Self>, id: Fingerprint, name: &str) {
        let dialog = adw::AlertDialog::new(
            Some(&format!("Forget {name}?")),
            Some("You'll need to pair again before sharing with this device."),
        );
        dialog.add_responses(&[("cancel", "Cancel"), ("forget", "Forget")]);
        dialog.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let w = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "forget" {
                return;
            }
            let Some(c) = w.client() else { return };
            let call = w.bridge.call(&c, move |c| async move { c.forget(id.to_string()).await });
            let w = w.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = call.await {
                    w.toast(&e);
                }
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
                && !e.contains("declined") {
                    w.toast(&e);
                }
            w.refresh_devices();
        });
    }

    fn show_pairing(self: &Rc<Self>, request: Uuid, device: linksy_ipc::linksy_core::DeviceInfo, code: String, incoming: bool) {
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
        self.present_attention(&dialog);
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
        self.present_attention(&dialog);
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

    /// Show a consent dialog; the service's desktop notification covers the case where
    /// this window is hidden, so don't steal focus.
    fn present_attention(&self, dialog: &adw::AlertDialog) {
        dialog.present(Some(&self.window));
    }

    // ---- transfers ----

    fn add_transfer(self: &Rc<Self>, id: Uuid, peer: &str, direction: Direction, files: &[FileEntry], total: u64) {
        let icon = gtk::Image::builder()
            .icon_name(if direction == Direction::Incoming { "folder-download-symbolic" } else { "send-to-symbolic" })
            .pixel_size(20)
            .css_classes(["transfer-icon"])
            .build();
        let what = match files {
            [one] => fmt::file_name(&one.path),
            many => fmt::plural(many.len(), "item"),
        };
        let title = gtk::Label::builder()
            .label(if direction == Direction::Incoming { format!("{what} from {peer}") } else { format!("{what} to {peer}") })
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["heading"])
            .build();
        let detail = gtk::Label::builder().label(format!("Starting… · {}", fmt::size(total))).xalign(0.0).css_classes(["caption", "dim-label", "numeric"]).build();
        let labels = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        labels.append(&title);
        labels.append(&detail);
        let action = gtk::Button::builder().icon_name("process-stop-symbolic").tooltip_text("Cancel").valign(gtk::Align::Center).css_classes(["flat", "circular"]).build();
        let top = gtk::Box::builder().spacing(12).build();
        top.append(&icon);
        top.append(&labels);
        top.append(&action);
        let bar = gtk::ProgressBar::builder().css_classes(["transfer-progress"]).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).build();
        body.append(&top);
        body.append(&bar);
        let row = gtk::ListBoxRow::builder().child(&body).activatable(false).build();

        let w = Rc::downgrade(self);
        action.connect_clicked(move |_| {
            let Some(w) = w.upgrade() else { return };
            w.transfer_action(id);
        });

        self.transfers.prepend(&row);
        self.transfers_group.set_visible(true);
        self.state.borrow_mut().transfers.insert(
            id,
            TransferRow { row, detail, bar, action, total, direction, samples: Vec::new(), saved: Vec::new(), done: false },
        );
    }

    fn update_transfer(&self, id: Uuid, bytes: u64, total: u64) {
        let mut state = self.state.borrow_mut();
        let Some(t) = state.transfers.get_mut(&id) else { return };
        let now = Instant::now();
        t.samples.push((now, bytes));
        t.samples.retain(|(at, _)| now.duration_since(*at) < Duration::from_secs(3));
        let speed = match (t.samples.first(), t.samples.last()) {
            (Some((t0, b0)), Some((t1, b1))) if t1 > t0 => (b1 - b0) as f64 / t1.duration_since(*t0).as_secs_f64(),
            _ => 0.0,
        };
        let mut detail = format!("{} of {}", fmt::size(bytes), fmt::size(total));
        if speed > 0.0 && bytes < total {
            let left = Duration::from_secs_f64((total - bytes) as f64 / speed);
            detail.push_str(&format!(" · {}/s · {}", fmt::size(speed as u64), fmt::remaining(left)));
        }
        t.detail.set_label(&detail);
        mark::ease_progress(&t.bar, if total == 0 { 1.0 } else { bytes as f64 / total as f64 });
    }

    fn finish_transfer(self: &Rc<Self>, id: Uuid, result: Result<Vec<PathBuf>, String>) {
        let mut state = self.state.borrow_mut();
        let Some(t) = state.transfers.get_mut(&id) else { return };
        t.done = true;
        t.bar.set_visible(false);
        match result {
            Ok(saved) => {
                t.detail.set_label(&format!("Done · {}", fmt::size(t.total)));
                t.row.add_css_class("transfer-done");
                if t.direction == Direction::Incoming {
                    t.action.set_icon_name("folder-open-symbolic");
                    t.action.set_tooltip_text(Some("Show in Folder"));
                    t.saved = saved;
                } else {
                    t.action.set_icon_name("object-select-symbolic");
                    t.action.set_tooltip_text(Some("Sent"));
                    t.action.set_sensitive(false);
                }
            }
            Err(reason) => {
                let text = match reason.as_str() {
                    "cancelled" => "Cancelled".to_string(),
                    "declined" | "request was declined" => "Declined".to_string(),
                    r => fmt::capitalize(r),
                };
                t.detail.set_label(&text);
                t.row.add_css_class("transfer-failed");
                t.action.set_icon_name("window-close-symbolic");
                t.action.set_tooltip_text(Some("Remove"));
            }
        }
    }

    fn transfer_action(self: &Rc<Self>, id: Uuid) {
        let (done, saved) = {
            let state = self.state.borrow();
            let Some(t) = state.transfers.get(&id) else { return };
            (t.done, t.saved.clone())
        };
        if !done {
            let Some(c) = self.client() else { return };
            let call = self.bridge.call(&c, move |c| async move { c.cancel(id).await });
            glib::spawn_future_local(async move {
                let _ = call.await;
            });
        } else if let Some(first) = saved.first() {
            let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(first)));
            launcher.open_containing_folder(Some(&self.window), gio::Cancellable::NONE, |_| {});
        } else {
            let removed = self.state.borrow_mut().transfers.remove(&id);
            if let Some(t) = removed {
                self.transfers.remove(&t.row);
                self.transfers_group.set_visible(self.transfers.first_child().is_some());
            }
        }
    }

    pub fn toast(&self, text: &str) {
        let toast = adw::Toast::new(&glib::markup_escape_text(text));
        toast.set_timeout(3);
        self.toasts.add_toast(toast);
    }
}

fn kind_icon(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Desktop | DeviceKind::Laptop => "computer-symbolic",
        DeviceKind::Phone => "phone-symbolic",
        DeviceKind::Tablet => "tablet-symbolic",
    }
}

fn file_list_paths(value: &glib::Value) -> Vec<PathBuf> {
    value.get::<gdk::FileList>().map(|l| l.files().iter().filter_map(|f| f.path()).collect()).unwrap_or_default()
}
