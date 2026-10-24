//! A device's page: who it is, big buttons for what you can do with it, and a timeline of
//! everything shared with it, grouped by day, with live progress for running transfers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use linksy_ipc::linksy_core::{DeviceKind, DeviceView, Direction, Uuid};
use linksy_i18n::t;
use linksy_ipc::{HistoryEntry, HistoryStatus, Shared, TerminalStatus};

use super::{LiveRow, Progress, drag_files, kind_icon, kind_name, progress_text, status_text};
use crate::fmt;

/// Things the page asks the window to do.
pub enum Action {
    SendFiles,
    SendClipboard,
    Ping,
    ViewScreen,
    Extend,
    Webcam,
    Microphone,
    CameraPreview,
    CameraStop,
    /// Hear the phone's microphone on this computer, or stop hearing it.
    MicListen(bool),
    MicStop,
    /// The Connect by USB Cable guide.
    UsbCable,
    Pair,
    SendStaged,
    Forget,
    ClearHistory,
    Cancel(Uuid),
    Open(Vec<PathBuf>),
    ShowInFolder(PathBuf),
    Copy(String),
    OpenLink(String),
    Ring,
    LockWhenAway(bool),
    /// Allow everything from away for this device, or only the small things.
    AwayAll(bool),
    /// Accept files from this device without asking.
    AutoAccept(bool),
    Locate,
    Messages,
    BrowseFiles,
    /// The timeline's filter changed.
    FilterActivity,
    /// Let this phone open terminals here (with the code), or stop it.
    AllowTerminal(bool),
    /// No code yet: choose one, then allow this phone.
    SetUpTerminal,
    ChangeTerminalCode,
    UnlockTerminal,
    EndTerminal(String),
}

type Handler = Rc<RefCell<Option<Rc<dyn Fn(Action)>>>>;

/// Most entries shown at once; older ones are still in `linksy history`.
const SHOWN: usize = 150;

pub struct DevicePage {
    /// Lays itself out by its own width (not the window's), so it fits beside the sidebar.
    pub root: adw::BreakpointBin,
    /// Header menu with the device's less common actions.
    pub menu: gtk::MenuButton,
    pub avatar: adw::Avatar,
    kind: gtk::Image,
    /// The battery, as a ring around the avatar.
    battery: Rc<std::cell::Cell<Option<f64>>>,
    ring: gtk::DrawingArea,
    name: gtk::Label,
    /// How it's connected ("Connected away from home"), as a chip.
    link: gtk::Label,
    status: gtk::Label,
    reach: gtk::Box,
    reach_until: gtk::Label,
    /// Send Files beside the smaller actions; stacked when the page is narrow.
    actions: gtk::Box,
    send_tile: gtk::Button,
    ring_tile: gtk::Widget,
    locate_tile: gtk::Widget,
    ping_tile: gtk::Widget,
    /// "Use the phone as": a card per feature, side by side.
    use_as: gtk::Box,
    use_as_title: gtk::Label,
    /// Connect by USB Cable (faster for all of these), or that it is.
    use_as_hint: gtk::Button,
    use_as_hint_content: adw::ButtonContent,
    display_card: gtk::Box,
    webcam_card: gtk::Box,
    mic_card: gtk::Box,
    webcam_desc: gtk::Label,
    webcam_live: gtk::Label,
    webcam_start: gtk::Button,
    webcam_running: gtk::Box,
    mic_desc: gtk::Label,
    mic_live: gtk::Label,
    mic_start: gtk::Button,
    mic_running: gtk::Box,
    mic_level: gtk::LevelBar,
    mic_listen: gtk::ToggleButton,
    /// Settings and places, as plain rows.
    settings: gtk::ListBox,
    messages_row: adw::ActionRow,
    files_row: adw::ActionRow,
    /// "Lock When Away", for phones; `syncing` while it's set from the service's state.
    lock_row: adw::SwitchRow,
    /// "Allow Everything From Away" (docs/protocol.md §4.13).
    away_row: adw::SwitchRow,
    /// "Always Accept Files".
    accept_row: adw::SwitchRow,
    /// Terminals (docs/protocol.md §4.15): allowed for this phone, locked out, and running.
    /// The Terminal card: what to do next (set a code, allow, unlock), or that it's ready.
    terminal_box: gtk::Box,
    terminal_title: gtk::Label,
    terminal_desc: gtk::Label,
    terminal_badge: gtk::Label,
    terminal_setup: gtk::Button,
    terminal_allow: gtk::Button,
    terminal_unlock: gtk::Button,
    terminal_ready: gtk::Box,
    terminals: gtk::ListBox,
    syncing: Rc<std::cell::Cell<bool>>,
    pair_box: gtk::Box,
    staged: adw::Banner,
    activity_box: gtk::Box,
    /// Which kind of entries the timeline shows.
    filter: Rc<std::cell::Cell<Filter>>,
    /// One heading and timeline per day.
    activity: gtk::Box,
    /// Opens a finished incoming row's files.
    on_activate: Rc<dyn Fn(&gtk::ListBoxRow)>,
    empty: gtk::Box,
    handler: Handler,
    /// Received files of finished rows, by entry id (the row's widget name), to open.
    received: Rc<RefCell<HashMap<String, Vec<PathBuf>>>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Files,
    Clipboard,
    Links,
}

impl Filter {
    fn shows(self, what: &Shared) -> bool {
        match self {
            Filter::All => true,
            Filter::Files => matches!(what, Shared::Files { .. }),
            Filter::Clipboard => matches!(what, Shared::Clipboard { .. }),
            Filter::Links => matches!(what, Shared::Link { .. }),
        }
    }
}

impl DevicePage {
    pub fn new() -> Self {
        let handler: Handler = Rc::default();
        let emit = {
            let h = handler.clone();
            move |a: Action| {
                let f = h.borrow().clone();
                if let Some(f) = f {
                    f(a);
                }
            }
        };

        // Hero: who this is, how it's connected, its battery as a ring.
        let avatar = adw::Avatar::builder().size(76).show_initials(true).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        let battery: Rc<std::cell::Cell<Option<f64>>> = Rc::default();
        let ring = gtk::DrawingArea::builder().content_width(94).content_height(94).build();
        let level = battery.clone();
        ring.set_draw_func(move |area, cr, w, h| {
            let Some(level) = level.get() else { return };
            let (cx, cy, r) = (w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 2.0 - 3.0);
            let fg = area.color();
            cr.set_line_width(4.5);
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.12);
            cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            // Green, then amber and red as it runs low.
            let (red, green, blue) = if level > 0.3 { (0.18, 0.76, 0.49) } else if level > 0.15 { (0.90, 0.65, 0.04) } else { (0.88, 0.11, 0.14) };
            cr.set_source_rgb(red, green, blue);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            let start = -std::f64::consts::FRAC_PI_2;
            cr.arc(cx, cy, r, start, start + level.clamp(0.0, 1.0) * std::f64::consts::TAU);
            let _ = cr.stroke();
        });
        let kind = gtk::Image::builder().pixel_size(14).css_classes(["hero-kind"]).halign(gtk::Align::End).valign(gtk::Align::End).build();
        let avatar_overlay = gtk::Overlay::builder().child(&ring).halign(gtk::Align::Start).valign(gtk::Align::Center).build();
        avatar_overlay.add_overlay(&avatar);
        avatar_overlay.add_overlay(&kind);
        let name = gtk::Label::builder().css_classes(["title-1"]).xalign(0.0).wrap(true).build();
        let link = gtk::Label::builder().valign(gtk::Align::Center).css_classes(["link-chip"]).build();
        let status = gtk::Label::builder().css_classes(["dim-label"]).xalign(0.0).wrap(true).build();
        let status_line = gtk::Box::builder().spacing(10).build();
        status_line.append(&link);
        status_line.append(&status);
        let who = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).hexpand(true).valign(gtk::Align::Center).build();
        who.append(&name);
        who.append(&status_line);
        let reach_until = gtk::Label::builder().xalign(1.0).css_classes(["heading"]).build();
        let reach = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).valign(gtk::Align::Center).build();
        reach.append(&gtk::Label::builder().label(t!("Reachable from away until")).xalign(1.0).css_classes(["caption", "dim-label"]).build());
        reach.append(&reach_until);
        let hero = gtk::Box::builder().spacing(20).css_classes(["hero"]).build();
        hero.append(&avatar_overlay);
        hero.append(&who);
        hero.append(&reach);

        // What you do most: Send Files (also where files are dropped), then the rest.
        let send_icon = gtk::Image::builder().icon_name("document-send-symbolic").pixel_size(24).halign(gtk::Align::Start).css_classes(["send-icon"]).build();
        let send_text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).vexpand(true).valign(gtk::Align::End).build();
        send_text.append(&gtk::Label::builder().label(t!("Send Files")).xalign(0.0).css_classes(["title-2"]).build());
        send_text.append(&gtk::Label::builder().label(t!("Click to choose, or drop them anywhere on this page")).xalign(0.0).wrap(true).css_classes(["dim-label", "tile-hint"]).build());
        let send_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).build();
        send_box.append(&send_icon);
        send_box.append(&send_text);
        let send_overlay = gtk::Overlay::builder().child(&send_box).build();
        send_overlay.add_overlay(&gtk::Label::builder().label("Ctrl+O").halign(gtk::Align::End).valign(gtk::Align::Start).css_classes(["caption-heading", "dim-label"]).build());
        let send_tile = gtk::Button::builder().child(&send_overlay).hexpand(true).css_classes(["send-tile"]).tooltip_text(t!("Choose files to send (Ctrl+O)")).build();
        let e = emit.clone();
        send_tile.connect_clicked(move |_| e(Action::SendFiles));

        let small = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(2)
            .max_children_per_line(2)
            .column_spacing(10)
            .row_spacing(10)
            .hexpand(true)
            .valign(gtk::Align::Fill)
            .css_classes(["tiles"])
            .build();
        let tile = |icon: &str, title: &str, hint: &str, warm: bool, action: fn() -> Action| -> gtk::Widget {
            let image = gtk::Image::builder().icon_name(icon).pixel_size(22).css_classes([if warm { "tile-icon-warm" } else { "tile-icon" }]).build();
            let labels = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).valign(gtk::Align::Center).build();
            labels.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["heading"]).build());
            labels.append(&gtk::Label::builder().label(hint).xalign(0.0).wrap(true).css_classes(["caption", "dim-label", "tile-hint"]).build());
            let inner = gtk::Box::builder().spacing(13).build();
            inner.append(&image);
            inner.append(&labels);
            let b = gtk::Button::builder().child(&inner).css_classes(["soft-tile"]).build();
            let e = emit.clone();
            b.connect_clicked(move |_| e(action()));
            let child = gtk::FlowBoxChild::builder().child(&b).focusable(false).build();
            small.append(&child);
            child.upcast()
        };
        tile("edit-paste-symbolic", t!("Clipboard"), t!("Send what you copied"), false, || Action::SendClipboard);
        tile("video-display-symbolic", t!("View Screen"), t!("Watch or take control"), false, || Action::ViewScreen);
        let ring_tile = tile("audio-volume-high-symbolic", t!("Ring"), t!("Loud, even on silent"), true, || Action::Ring);
        let locate_tile = tile("find-location-symbolic", t!("Locate"), t!("Asked once, never tracked"), true, || Action::Locate);
        let ping_tile = tile("preferences-system-notifications-symbolic", t!("Ping"), t!("Show a notification"), false, || Action::Ping);
        // Not expanding: their insides push text to the bottom, not the page.
        let actions = gtk::Box::builder().spacing(10).homogeneous(true).vexpand(false).build();
        actions.append(&send_tile);
        actions.append(&small);

        // "Use the phone as": the big features, each a card of its own.
        let card = |icon: &str, title: &str, desc: &str| -> (gtk::Box, gtk::Box, gtk::Label, gtk::Label) {
            let image = gtk::Image::builder().icon_name(icon).pixel_size(28).halign(gtk::Align::Start).css_classes(["card-icon"]).build();
            let badge = gtk::Label::builder().label(t!("Live")).halign(gtk::Align::End).valign(gtk::Align::Start).hexpand(true).visible(false).css_classes(["live-badge"]).build();
            let top = gtk::Box::builder().build();
            top.append(&image);
            top.append(&badge);
            let description = gtk::Label::builder().label(desc).xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
            let buttons = gtk::Box::builder().spacing(8).vexpand(true).valign(gtk::Align::End).build();
            let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).hexpand(true).css_classes(["feature-card"]).build();
            b.append(&top);
            b.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["title-4"]).build());
            b.append(&description);
            b.append(&buttons);
            (b, buttons, description, badge)
        };
        let start = |label: &str, action: fn() -> Action| -> gtk::Button {
            let b = gtk::Button::builder().label(label).hexpand(true).css_classes(["suggested-action"]).build();
            let e = emit.clone();
            b.connect_clicked(move |_| e(action()));
            b
        };
        let (display_card, buttons, _, _) = card("video-joined-displays-symbolic", t!("Second Display"), t!("Extend this computer's desktop onto it"));
        buttons.append(&start(t!("Extend Desktop"), || Action::Extend));
        let (webcam_card, buttons, webcam_desc, webcam_live) = card("camera-web-symbolic", t!("Webcam"), &t!("“{camera}” in your apps", camera = "Linksy Camera"));
        let webcam_start = start(t!("Start Webcam"), || Action::Webcam);
        let preview = gtk::Button::builder().label(t!("Preview")).hexpand(true).build();
        let e = emit.clone();
        preview.connect_clicked(move |_| e(Action::CameraPreview));
        let stop = gtk::Button::builder().label(t!("Stop")).build();
        let e = emit.clone();
        stop.connect_clicked(move |_| e(Action::CameraStop));
        let webcam_running = gtk::Box::builder().spacing(8).hexpand(true).visible(false).build();
        webcam_running.append(&preview);
        webcam_running.append(&stop);
        buttons.append(&webcam_start);
        buttons.append(&webcam_running);
        let (mic_card, buttons, mic_desc, mic_live) = card("audio-input-microphone-symbolic", t!("Microphone"), &t!("“{microphone}” in calls and recorders", microphone = "Linksy Microphone"));
        let mic_start = start(t!("Start Microphone"), || Action::Microphone);
        // While it's on: how loud it is right now, Listen to hear it here, and Stop.
        let mic_level = gtk::LevelBar::builder().min_value(0.0).max_value(1.0).visible(false).css_classes(["mic-level"]).build();
        for offset in [gtk::LEVEL_BAR_OFFSET_LOW, gtk::LEVEL_BAR_OFFSET_HIGH, gtk::LEVEL_BAR_OFFSET_FULL] {
            mic_level.remove_offset_value(Some(offset));
        }
        mic_card.insert_child_after(&mic_level, Some(&mic_desc));
        let mic_listen = gtk::ToggleButton::builder().label(t!("Listen")).hexpand(true).tooltip_text(t!("Hear it on this computer (headphones avoid an echo)")).build();
        let e = emit.clone();
        mic_listen.connect_toggled(move |b| e(Action::MicListen(b.is_active())));
        let mic_stop = gtk::Button::builder().label(t!("Stop")).build();
        let e = emit.clone();
        mic_stop.connect_clicked(move |_| e(Action::MicStop));
        let mic_running = gtk::Box::builder().spacing(8).hexpand(true).visible(false).build();
        mic_running.append(&mic_listen);
        mic_running.append(&mic_stop);
        buttons.append(&mic_start);
        buttons.append(&mic_running);
        let cards = gtk::Box::builder().spacing(10).homogeneous(true).vexpand(false).build();
        cards.append(&display_card);
        cards.append(&webcam_card);
        cards.append(&mic_card);
        let use_as_title = gtk::Label::builder().xalign(0.0).hexpand(true).wrap(true).css_classes(["title-4"]).build();
        let use_as_hint_content = adw::ButtonContent::builder().icon_name("media-removable-symbolic").label(t!("Faster by USB Cable")).build();
        let use_as_hint = gtk::Button::builder().child(&use_as_hint_content).tooltip_text(t!("Connect by USB Cable")).valign(gtk::Align::Center).css_classes(["flat", "usb-cable"]).build();
        let e = emit.clone();
        use_as_hint.connect_clicked(move |_| e(Action::UsbCable));
        let use_as_head = gtk::Box::builder().spacing(4).build();
        use_as_head.append(&use_as_title);
        use_as_head.append(&use_as_hint);
        let use_as = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        use_as.append(&use_as_head);
        use_as.append(&cards);

        // Settings and places: plain rows.
        let messages_row = adw::ActionRow::builder().title(t!("Messages")).activatable(true).build();
        messages_row.add_prefix(&gtk::Image::builder().icon_name("chat-message-new-symbolic").build());
        messages_row.add_suffix(&gtk::Image::builder().icon_name("go-next-symbolic").build());
        let e = emit.clone();
        messages_row.connect_activated(move |_| e(Action::Messages));
        let files_row = adw::ActionRow::builder().title(t!("Browse Files")).activatable(true).build();
        files_row.add_prefix(&gtk::Image::builder().icon_name("folder-remote-symbolic").build());
        files_row.add_suffix(&gtk::Image::builder().icon_name("go-next-symbolic").build());
        let e = emit.clone();
        files_row.connect_activated(move |_| e(Action::BrowseFiles));
        let lock_row = adw::SwitchRow::builder().title(t!("Lock When Away")).build();
        lock_row.add_prefix(&gtk::Image::builder().icon_name("system-lock-screen-symbolic").build());
        let syncing: Rc<std::cell::Cell<bool>> = Rc::default();
        let (e, s) = (emit.clone(), syncing.clone());
        lock_row.connect_active_notify(move |row| {
            if !s.get() {
                e(Action::LockWhenAway(row.is_active()));
            }
        });
        let away_row = adw::SwitchRow::builder().title(t!("Allow Everything From Away")).subtitle_lines(3).build();
        away_row.add_prefix(&gtk::Image::builder().icon_name("network-wireless-hotspot-symbolic").build());
        let (e, s) = (emit.clone(), syncing.clone());
        away_row.connect_active_notify(move |row| {
            if !s.get() {
                e(Action::AwayAll(row.is_active()));
            }
        });
        let accept_row = adw::SwitchRow::builder().title(t!("Always Accept Files")).subtitle_lines(2).build();
        accept_row.add_prefix(&gtk::Image::builder().icon_name("folder-download-symbolic").build());
        let (e, s) = (emit.clone(), syncing.clone());
        accept_row.connect_active_notify(move |row| {
            if !s.get() {
                e(Action::AutoAccept(row.is_active()));
            }
        });
        // Terminal: a card that says what's next, right here rather than in Preferences.
        let terminal_icon = gtk::Image::builder().icon_name("utilities-terminal-symbolic").pixel_size(28).valign(gtk::Align::Start).css_classes(["card-icon"]).build();
        let terminal_title = gtk::Label::builder().xalign(0.0).wrap(true).hexpand(true).css_classes(["title-4"]).build();
        let terminal_badge = gtk::Label::builder().label(t!("Ready")).valign(gtk::Align::Start).css_classes(["live-badge"]).visible(false).build();
        let terminal_head = gtk::Box::builder().spacing(8).build();
        terminal_head.append(&terminal_title);
        terminal_head.append(&terminal_badge);
        let terminal_desc = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
        let button = |label: &str, suggested: bool, action: fn() -> Action| {
            let b = gtk::Button::builder().label(label).css_classes(if suggested { vec!["suggested-action"] } else { vec![] }).build();
            let e = emit.clone();
            b.connect_clicked(move |_| e(action()));
            b
        };
        let terminal_setup = button(t!("Set Up Terminal…"), true, || Action::SetUpTerminal);
        let terminal_allow = button(t!("Allow"), true, || Action::AllowTerminal(true));
        let terminal_unlock = button(t!("Unlock"), true, || Action::UnlockTerminal);
        let terminal_ready = gtk::Box::builder().spacing(8).build();
        terminal_ready.append(&button(t!("Turn Off"), false, || Action::AllowTerminal(false)));
        terminal_ready.append(&button(t!("Change Code…"), false, || Action::ChangeTerminalCode));
        let terminal_buttons = gtk::Box::builder().spacing(8).margin_top(4).build();
        for b in [terminal_setup.upcast_ref::<gtk::Widget>(), terminal_allow.upcast_ref(), terminal_unlock.upcast_ref(), terminal_ready.upcast_ref()] {
            terminal_buttons.append(b);
        }
        let terminal_text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).hexpand(true).build();
        terminal_text.append(&terminal_head);
        terminal_text.append(&terminal_desc);
        terminal_text.append(&terminal_buttons);
        let terminal_card = gtk::Box::builder().spacing(16).css_classes(["feature-card"]).build();
        terminal_card.append(&terminal_icon);
        terminal_card.append(&terminal_text);
        let terminal_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).visible(false).build();
        terminal_box.append(&gtk::Label::builder().label(t!("Terminal")).xalign(0.0).css_classes(["title-4"]).build());
        terminal_box.append(&terminal_card);
        let settings = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["plain-list"]).build();
        settings.append(&messages_row);
        settings.append(&files_row);
        settings.append(&lock_row);
        settings.append(&accept_row);
        settings.append(&away_row);
        let terminals = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).visible(false).css_classes(["boxed-list"]).build();

        // For devices that aren't paired yet.
        let pair = gtk::Button::builder().label(t!("Pair")).css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).build();
        let e = emit.clone();
        pair.connect_clicked(move |_| e(Action::Pair));
        let pair_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).build();
        pair_box.append(
            &gtk::Label::builder()
                .label(t!("Pair once to share with this device. Both screens will show a code; check they match."))
                .wrap(true)
                .justify(gtk::Justification::Center)
                .css_classes(["dim-label"])
                .build(),
        );
        pair_box.append(&pair);

        // Files opened with Linksy before a device was picked.
        let staged = adw::Banner::builder().button_label(t!("Send")).use_markup(false).revealed(false).build();
        let e = emit.clone();
        staged.connect_button_clicked(move |_| e(Action::SendStaged));

        // Timeline.
        let filter: Rc<std::cell::Cell<Filter>> = Rc::new(std::cell::Cell::new(Filter::All));
        let filters = gtk::Box::builder().spacing(6).build();
        let activity_header = gtk::Box::builder().spacing(6).build();
        activity_header.append(&gtk::Label::builder().label(t!("Activity")).xalign(0.0).hexpand(true).css_classes(["title-4"]).build());
        activity_header.append(&filters);
        let mut first: Option<gtk::ToggleButton> = None;
        for (label, which) in [(t!("All"), Filter::All), (t!("Files"), Filter::Files), (t!("Clipboard"), Filter::Clipboard), (t!("Links"), Filter::Links)] {
            let b = gtk::ToggleButton::builder().label(label).valign(gtk::Align::Center).css_classes(["filter-pill"]).active(which == Filter::All).build();
            b.set_group(first.as_ref());
            first.get_or_insert(b.clone());
            let (e, f) = (emit.clone(), filter.clone());
            b.connect_toggled(move |b| {
                if b.is_active() && f.get() != which {
                    f.set(which);
                    e(Action::FilterActivity);
                }
            });
            filters.append(&b);
        }
        let clear = gtk::Button::builder().label(t!("Clear")).css_classes(["flat"]).valign(gtk::Align::Center).build();
        let e = emit.clone();
        clear.connect_clicked(move |_| e(Action::ClearHistory));
        filters.append(&clear);
        let activity = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        let received: Rc<RefCell<HashMap<String, Vec<PathBuf>>>> = Rc::default();
        let (e, r) = (emit.clone(), received.clone());
        let on_activate: Rc<dyn Fn(&gtk::ListBoxRow)> = Rc::new(move |row| {
            // Finished incoming files open.
            let paths = r.borrow().get(row.widget_name().as_str()).cloned();
            if let Some(paths) = paths {
                e(Action::Open(paths));
            }
        });
        let empty = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["activity-empty"]).build();
        empty.append(&gtk::Label::builder().label(t!("Nothing shared yet")).css_classes(["heading", "dim-label"]).build());
        empty.append(&gtk::Label::builder().label(t!("Files, clipboard text and pings you exchange show up here.")).wrap(true).justify(gtk::Justification::Center).css_classes(["caption", "dim-label"]).build());
        let activity_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        activity_box.append(&activity_header);
        activity_box.append(&activity);
        activity_box.append(&empty);

        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(26).margin_top(6).margin_bottom(32).margin_start(16).margin_end(16).build();
        content.append(&hero);
        content.append(&actions);
        content.append(&use_as);
        content.append(&terminal_box);
        content.append(&settings);
        content.append(&terminals);
        content.append(&pair_box);
        content.append(&activity_box);
        let clamp = adw::Clamp::builder().maximum_size(860).tightening_threshold(600).child(&content).build();
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        column.append(&staged);
        column.append(&clamp);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&column).build();
        let root = adw::BreakpointBin::builder().width_request(300).height_request(300).child(&scroller).build();
        // Narrower and narrower: stack Send Files over the tiles, then the cards, then the
        // header, the "Use as" heading and the timeline's filters, then the tiles. (Only
        // the last matching breakpoint applies, so each repeats the ones before.)
        // Responsive design, written by hand, in Rust. Be gentle with it.
        let vertical = gtk::Orientation::Vertical.to_value();
        let one = 1u32.to_value();
        let changes: [(&gtk::Widget, &str, &glib::Value); 7] = [
            (actions.upcast_ref(), "orientation", &vertical),
            (cards.upcast_ref(), "orientation", &vertical),
            (hero.upcast_ref(), "orientation", &vertical),
            (use_as_head.upcast_ref(), "orientation", &vertical),
            (activity_header.upcast_ref(), "orientation", &vertical),
            (small.upcast_ref(), "min-children-per-line", &one),
            (small.upcast_ref(), "max-children-per-line", &one),
        ];
        for (width, count) in [("760sp", 1), ("680sp", 2), ("540sp", 5), ("400sp", 7)] {
            let b = adw::Breakpoint::new(adw::BreakpointCondition::parse(&format!("max-width: {width}")).expect("breakpoint"));
            for (widget, property, value) in &changes[..count] {
                b.add_setter(*widget, property, Some(value));
            }
            root.add_breakpoint(b);
        }

        // Header menu.
        let forget = gtk::Button::builder().label(t!("Forget Device…")).css_classes(["flat", "destructive-label"]).build();
        let popover_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-box"]).build();
        popover_box.append(&forget);
        let popover = gtk::Popover::builder().child(&popover_box).build();
        let menu = gtk::MenuButton::builder().icon_name("view-more-symbolic").popover(&popover).tooltip_text(t!("Device Menu")).build();
        let e = emit.clone();
        forget.connect_clicked(move |_| {
            popover.popdown();
            e(Action::Forget);
        });

        Self {
            root,
            menu,
            avatar,
            kind,
            battery,
            ring,
            name,
            link,
            status,
            reach,
            reach_until,
            actions,
            send_tile,
            ring_tile,
            locate_tile,
            ping_tile,
            use_as,
            use_as_title,
            use_as_hint,
            use_as_hint_content,
            display_card,
            webcam_card,
            mic_card,
            mic_desc,
            mic_live,
            mic_start,
            mic_running,
            mic_level,
            mic_listen,
            webcam_desc,
            webcam_live,
            webcam_start,
            webcam_running,
            settings,
            messages_row,
            files_row,
            lock_row,
            away_row,
            accept_row,
            terminal_box,
            terminal_title,
            terminal_desc,
            terminal_badge,
            terminal_setup,
            terminal_allow,
            terminal_unlock,
            terminal_ready,
            terminals,
            syncing,
            pair_box,
            staged,
            activity_box,
            filter,
            activity,
            on_activate,
            empty,
            handler,
            received,
        }
    }

    /// Show the settings rows only when one of them is.
    fn settings_visible(&self, trusted: bool) {
        let mut any = false;
        let mut row = self.settings.first_child();
        while let Some(r) = row {
            any |= r.is_visible();
            row = r.next_sibling();
        }
        self.settings.set_visible(trusted && any);
    }

    /// Terminals for this device: whether it may open them, whether it's locked out, and
    /// the ones running now, each with End.
    pub fn show_terminals(&self, d: &DeviceView, status: Option<&TerminalStatus>) {
        let phone = matches!(d.device.kind, DeviceKind::Phone | DeviceKind::Tablet);
        let shown = d.trusted && phone && status.is_some();
        let status = status.cloned().unwrap_or_default();
        self.terminal_box.set_visible(shown);
        let name = &d.device.name;
        let (allowed, locked) = (status.allowed.contains(&d.device.id), status.locked.contains(&d.device.id));
        self.terminal_title.set_label(&t!("Your computer's terminal on {name}", name));
        let (desc, step) = if locked {
            (t!("Locked: 5 wrong codes in a row were typed on {name}. Unlock it if that was you.", name), 2)
        } else if !status.code_set {
            (t!("Run Claude Code, neovim or a build on this computer from {name}. Choose a code first: {name} asks for it every time, and only this computer knows it.", name), 0)
        } else if !allowed {
            (t!("Let {name} open a shell on this computer after typing the terminal code. It runs as you.", name), 1)
        } else {
            (t!("On {name}, open Linksy, pick this computer and choose Terminal, then type the code. This computer asks you once a day. (Needs the Linksy for Android update.)", name), 3)
        };
        self.terminal_desc.set_label(&desc);
        self.terminal_setup.set_visible(step == 0);
        self.terminal_allow.set_visible(step == 1);
        self.terminal_allow.set_label(&t!("Allow on {name}", name));
        self.terminal_unlock.set_visible(step == 2);
        self.terminal_ready.set_visible(step == 3);
        self.terminal_badge.set_visible(step == 3);
        while let Some(row) = self.terminals.first_child() {
            self.terminals.remove(&row);
        }
        let mine: Vec<_> = status.running.iter().filter(|t| t.device == d.device.id).collect();
        for t in &mine {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&t.title))
                .subtitle(if t.attached { t!("Terminal open on the phone now") } else { t!("Terminal still running · not open on the phone") })
                .build();
            row.add_prefix(&gtk::Image::builder().icon_name("utilities-terminal-symbolic").css_classes([if t.attached { "success" } else { "dim-label" }]).build());
            let end = gtk::Button::builder().label(t!("End")).valign(gtk::Align::Center).css_classes(["destructive-action"]).build();
            let (h, id) = (self.handler.clone(), t.id.clone());
            end.connect_clicked(move |_| {
                let f = h.borrow().clone();
                if let Some(f) = f {
                    f(Action::EndTerminal(id.clone()));
                }
            });
            row.add_suffix(&end);
            self.terminals.append(&row);
        }
        self.terminals.set_visible(d.trusted && !mine.is_empty());
    }

    pub fn connect_actions(&self, f: impl Fn(Action) + 'static) {
        *self.handler.borrow_mut() = Some(Rc::new(f));
    }

    /// Show the webcam card as live for `camera` ("Linksy Camera (…)"), or as ready.
    pub fn set_camera(&self, camera: Option<&str>) {
        let live = camera.is_some();
        match camera {
            Some(name) => self.webcam_desc.set_label(&t!("On now · Your apps list it as “{name}”", name)),
            None => self.webcam_desc.set_label(&t!("“{camera}” in your apps", camera = "Linksy Camera")),
        }
        self.webcam_live.set_visible(live);
        self.webcam_start.set_visible(!live);
        self.webcam_running.set_visible(live);
        if live {
            self.webcam_card.add_css_class("live");
        } else {
            self.webcam_card.remove_css_class("live");
        }
    }

    /// The phone's microphone in use here, as apps name it, or none.
    pub fn set_microphone(&self, mic: Option<&str>) {
        let live = mic.is_some();
        match mic {
            Some(name) => self.mic_desc.set_label(&t!("On now · Your apps list it as “{name}”", name)),
            None => self.mic_desc.set_label(&t!("“{microphone}” in calls and recorders", microphone = "Linksy Microphone")),
        }
        self.mic_live.set_visible(live);
        self.mic_start.set_visible(!live);
        self.mic_running.set_visible(live);
        self.mic_level.set_visible(live);
        if !live {
            self.mic_level.set_value(0.0);
            self.mic_listen.set_active(false);
        }
        if live {
            self.mic_card.add_css_class("live");
        } else {
            self.mic_card.remove_css_class("live");
        }
    }

    /// The microphone's level meter (0 to 1), for whatever measures it.
    pub fn mic_level_bar(&self) -> gtk::LevelBar {
        self.mic_level.clone()
    }

    pub fn set_drop_active(&self, on: bool) {
        if on {
            self.send_tile.add_css_class("drop-active");
        } else {
            self.send_tile.remove_css_class("drop-active");
        }
    }

    pub fn show_device(&self, d: &DeviceView, staged: &[PathBuf], locks: bool) {
        let available = d.connected || d.nearby;
        let phone = matches!(d.device.kind, DeviceKind::Phone | DeviceKind::Tablet);
        self.avatar.set_text(Some(&d.device.name));
        self.kind.set_icon_name(Some(kind_icon(d.device.kind)));
        self.name.set_label(&d.device.name);
        self.battery.set(d.battery.filter(|_| available).map(|b| b.level as f64 / 100.0));
        self.ring.queue_draw();
        self.link.set_label(status_text(d));
        for class in ["away", "usb", "offline"] {
            self.link.remove_css_class(class);
        }
        if d.connected && d.away {
            self.link.add_css_class("away");
        } else if d.connected && d.usb {
            self.link.add_css_class("usb");
        } else if !available {
            self.link.add_css_class("offline");
        }
        let mut status = vec![kind_name(&d.device).to_string()];
        status.extend(fmt::battery(d));
        if d.trusted {
            status.push(t!("Encrypted end to end").into());
        }
        self.status.set_label(&status.join(" · "));
        self.reach.set_visible(d.trusted && d.away_until.is_some());
        self.reach_until.set_label(&d.away_until.map(fmt::day).unwrap_or_default());

        let ring = phone && super::can(&d.device, "ring");
        self.actions.set_visible(d.trusted);
        self.ring_tile.set_visible(ring);
        self.locate_tile.set_visible(d.device.kind == DeviceKind::Phone && super::can(&d.device, "locate"));
        self.ping_tile.set_visible(!ring);

        self.display_card.set_visible(super::can(&d.device, "display"));
        self.webcam_card.set_visible(super::can(&d.device, "camera"));
        self.mic_card.set_visible(super::can(&d.device, "microphone"));
        self.use_as.set_visible(d.trusted && (self.display_card.is_visible() || self.webcam_card.is_visible() || self.mic_card.is_visible()));
        self.use_as_title.set_label(&t!("Use {device} as", device = d.device.name));
        self.use_as_hint.set_visible(d.device.os == "android");
        self.use_as_hint_content.set_label(if d.connected && d.usb { t!("Connected by USB") } else { t!("Faster by USB Cable") });

        self.lock_row.set_visible(d.device.kind == DeviceKind::Phone && super::can(&d.device, "heartbeat"));
        self.messages_row.set_visible(d.device.kind == DeviceKind::Phone && super::can(&d.device, "sms"));
        self.files_row.set_visible(phone && super::can(&d.device, "fs"));
        self.files_row.set_subtitle(&glib::markup_escape_text(&t!("Open the folder {device} shares in Files, like a USB drive", device = d.device.name)));
        self.messages_row.set_subtitle(&glib::markup_escape_text(&t!("Read and send {device}'s texts here", device = d.device.name)));
        self.lock_row.set_subtitle(&glib::markup_escape_text(&t!("Lock this computer when {device} leaves. It never unlocks it.", device = d.device.name)));
        // Away from home: only for devices that told this one how to reach them.
        self.away_row.set_visible(d.trusted && d.away_until.is_some());
        self.away_row.set_subtitle(&glib::markup_escape_text(&t!(
            "Away from home, {device} can always send the clipboard, links and files and ring. On: screens, control, its files, camera and microphone too.",
            device = d.device.name
        )));
        self.accept_row.set_subtitle(&glib::markup_escape_text(&t!("Files from {device} are saved without asking.", device = d.device.name)));
        self.settings_visible(d.trusted);
        self.syncing.set(true);
        self.accept_row.set_active(d.auto_accept);
        self.lock_row.set_active(locks);
        self.away_row.set_active(d.away_all);
        self.syncing.set(false);
        self.pair_box.set_visible(!d.trusted);
        self.menu.set_visible(d.trusted);
        self.activity_box.set_visible(d.trusted);

        let ready = d.trusted && !staged.is_empty();
        if ready {
            let (count, bytes) = fmt::measure(staged);
            let what = match staged {
                [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                _ => fmt::plural(count, "item"),
            };
            self.staged.set_title(&t!("{what} ({size}) ready to send to {device}", what, size = fmt::size(bytes), device = d.device.name));
        }
        self.staged.set_revealed(ready);
    }

    /// Rebuild the timeline. Returns the widgets of running transfers, for progress.
    pub(super) fn show_history(&self, history: &[HistoryEntry], progress: &HashMap<Uuid, Progress>) -> HashMap<Uuid, LiveRow> {
        while let Some(child) = self.activity.first_child() {
            self.activity.remove(&child);
        }
        self.received.borrow_mut().clear();
        let filter = self.filter.get();
        let mut days: Vec<(String, Vec<&HistoryEntry>)> = Vec::new();
        for e in history.iter().take(SHOWN).filter(|e| filter.shows(&e.what)) {
            let day = day_label(e.at);
            match days.last_mut() {
                Some((d, entries)) if *d == day => entries.push(e),
                _ => days.push((day, vec![e])),
            }
        }
        let mut live = HashMap::new();
        for (day, entries) in &days {
            let header = gtk::Label::builder().label(day).xalign(0.0).css_classes(["caption-heading", "dim-label", "day-header"]).build();
            let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["timeline"]).build();
            let open = self.on_activate.clone();
            list.connect_row_activated(move |_, row| open(row));
            for (i, e) in entries.iter().enumerate() {
                let (row, live_row) = self.entry_row(e, progress.get(&e.id), i + 1 == entries.len());
                list.append(&row);
                if let Some(l) = live_row {
                    live.insert(e.id, l);
                }
            }
            self.activity.append(&header);
            self.activity.append(&list);
        }
        let empty = days.is_empty();
        self.activity.set_visible(!empty);
        self.empty.set_visible(empty);
        live
    }

    /// One entry: its time, a node on the timeline, and what happened.
    fn entry_row(&self, e: &HistoryEntry, progress: Option<&Progress>, last: bool) -> (gtk::ListBoxRow, Option<LiveRow>) {
        let incoming = e.direction == Direction::Incoming;
        let (icon, title) = match &e.what {
            Shared::Files { files, import: true, .. } => ("image-x-generic-symbolic", fmt::plural(files.len(), "photo")),
            Shared::Files { files, .. } => (
                if incoming { "folder-download-symbolic" } else { "document-send-symbolic" },
                match files.as_slice() {
                    [one] => fmt::file_name(one),
                    many => fmt::plural(many.len(), "item"),
                },
            ),
            Shared::Clipboard { text } if text.is_empty() => ("edit-paste-symbolic", t!("Clipboard text").to_string()),
            Shared::Clipboard { text } => ("edit-paste-symbolic", format!("“{}”", one_line(text, 80))),
            Shared::Ping { title, body } if body.is_empty() => ("preferences-system-notifications-symbolic", title.clone()),
            Shared::Ping { title, body } => ("preferences-system-notifications-symbolic", format!("{title}: {body}")),
            Shared::Link { url } => ("web-browser-symbolic", one_line(url, 120)),
            Shared::Locked => ("system-lock-screen-symbolic", t!("Locked this computer").to_string()),
            Shared::Remote { .. } => ("input-touchpad-symbolic", t!("Touchpad and keyboard").to_string()),
            Shared::Camera { .. } => ("camera-web-symbolic", if incoming { t!("Their camera").to_string() } else { t!("Your camera").to_string() }),
            Shared::Microphone { .. } => ("audio-input-microphone-symbolic", if incoming { t!("Their microphone").to_string() } else { t!("Your microphone").to_string() }),
            Shared::Terminal { .. } => ("utilities-terminal-symbolic", t!("Terminal").to_string()),
            Shared::Screen { display: true, .. } => ("video-joined-displays-symbolic", if incoming { t!("Their second display").to_string() } else { t!("Second display").to_string() }),
            Shared::Screen { .. } => ("video-display-symbolic", if incoming { t!("Their screen").to_string() } else { t!("Your screen").to_string() }),
        };
        let size = match &e.what {
            Shared::Files { total, .. } => Some(fmt::size(*total)),
            Shared::Screen { seconds, .. } | Shared::Camera { seconds } | Shared::Microphone { seconds } | Shared::Remote { seconds } | Shared::Terminal { seconds } if *seconds > 0 => Some(fmt::duration(*seconds)),
            _ => None,
        };
        let verb = match (&e.what, incoming) {
            (Shared::Files { import: true, .. }, _) => t!("Imported"),
            (Shared::Files { .. }, true) => t!("Received"),
            (Shared::Files { .. }, false) => t!("Sent"),
            (Shared::Camera { .. } | Shared::Microphone { .. }, true) => t!("Used"),
            (Shared::Camera { .. } | Shared::Microphone { .. }, false) => t!("Shared"),
            (Shared::Screen { .. }, true) => t!("Watched"),
            (Shared::Screen { .. }, false) => t!("Shared"),
            (Shared::Link { .. }, true) => t!("Opened here"),
            (Shared::Link { .. }, false) => t!("Opened there"),
            (Shared::Locked, _) => t!("When it left"),
            (Shared::Terminal { .. }, _) => t!("Opened from the phone"),
            (Shared::Remote { .. }, _) => t!("Used"),
            (_, true) => t!("From them"),
            (_, false) => t!("From you"),
        };
        let state = match &e.status {
            HistoryStatus::InProgress => None,
            HistoryStatus::Done => Some(verb.to_string()),
            HistoryStatus::Declined => Some(t!("Declined").into()),
            HistoryStatus::Cancelled => Some(t!("Cancelled").into()),
            HistoryStatus::Failed { reason } => Some(t!("Failed: {reason}", reason)),
        };
        let detail_text = match &state {
            Some(s) => [Some(s.clone()), size].into_iter().flatten().collect::<Vec<_>>().join(" · "),
            None => match progress {
                _ if matches!(e.what, Shared::Screen { .. } | Shared::Camera { .. } | Shared::Microphone { .. } | Shared::Remote { .. } | Shared::Terminal { .. }) => t!("Live now").into(),
                Some(p) if p.total > 0 => progress_text(p),
                _ if !incoming => t!("Waiting for them to accept…").into(),
                _ => t!("Starting…").into(),
            },
        };
        let running = matches!(e.status, HistoryStatus::InProgress);

        // Time · node · what happened.
        let time = gtk::Label::builder().label(time_label(e.at)).xalign(1.0).yalign(0.0).width_chars(5).margin_top(5).css_classes(["caption", "dim-label", "numeric"]).build();
        let node = gtk::Image::builder().icon_name(icon).pixel_size(14).css_classes(["tl-node", if running { "live" } else if incoming { "incoming" } else { "outgoing" }]).build();
        let line = gtk::Box::builder().vexpand(true).halign(gtk::Align::Center).css_classes(["tl-line"]).visible(!last).build();
        let rail = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        rail.append(&node);
        rail.append(&line);

        let title_label = gtk::Label::builder().label(&title).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["heading"]).build();
        let detail = gtk::Label::builder().label(&detail_text).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label", "numeric"]).build();
        let head = gtk::Box::builder().spacing(8).build();
        head.append(&title_label);
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).hexpand(true).margin_bottom(if last { 6 } else { 20 }).build();
        body.append(&head);
        body.append(&detail);

        let entry = gtk::Box::builder().spacing(12).build();
        entry.append(&time);
        entry.append(&rail);
        entry.append(&body);
        let row = gtk::ListBoxRow::builder().child(&entry).activatable(false).build();
        if matches!(e.status, HistoryStatus::Failed { .. } | HistoryStatus::Declined | HistoryStatus::Cancelled) {
            row.add_css_class("activity-failed");
        }

        let action = |label: &str, act: Box<dyn Fn() -> Action>| {
            let b = gtk::Button::builder().label(label).valign(gtk::Align::Center).css_classes(["flat", "tl-action"]).build();
            let h = self.handler.clone();
            b.connect_clicked(move |_| {
                let f = h.borrow().clone();
                if let Some(f) = f {
                    f(act());
                }
            });
            head.append(&b);
        };

        let mut live = None;
        match (&e.status, &e.what) {
            (HistoryStatus::InProgress, Shared::Screen { .. } | Shared::Camera { .. } | Shared::Microphone { .. } | Shared::Remote { .. } | Shared::Terminal { .. }) => {}
            (HistoryStatus::InProgress, _) => {
                // A running transfer is the one entry with a card around it.
                let id = e.id;
                action(t!("Cancel"), Box::new(move || Action::Cancel(id)));
                let bar = gtk::ProgressBar::builder().margin_top(6).css_classes(["transfer-progress"]).build();
                if let Some(p) = progress.filter(|p| p.total > 0) {
                    bar.set_fraction(p.bytes as f64 / p.total as f64);
                }
                body.append(&bar);
                body.add_css_class("tl-card");
                live = Some(LiveRow { detail, bar });
            }
            (HistoryStatus::Done, Shared::Files { saved, .. }) if incoming && !saved.is_empty() => {
                let first = saved[0].clone();
                action(t!("Show in Folder"), Box::new(move || Action::ShowInFolder(first.clone())));
                row.set_activatable(true);
                row.set_tooltip_text(Some(if saved.len() == 1 { t!("Open, or drag it anywhere") } else { t!("Open the folder, or drag the files anywhere") }));
                row.add_controller(drag_files(saved));
                row.set_widget_name(&e.id.to_string());
                self.received.borrow_mut().insert(e.id.to_string(), saved.clone());
            }
            (_, Shared::Link { url }) => {
                let url = url.clone();
                action(t!("Open Link"), Box::new(move || Action::OpenLink(url.clone())));
            }
            (_, Shared::Clipboard { text }) if !text.is_empty() => {
                let text = text.clone();
                action(t!("Copy Again"), Box::new(move || Action::Copy(text.clone())));
            }
            _ => {}
        }
        (row, live)
    }
}

/// First line of `text`, at most `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut out: String = line.chars().take(max).collect();
    if out.len() < line.len() || text.trim().lines().count() > 1 {
        out.push('…');
    }
    out
}

fn local(at_ms: u64) -> Option<glib::DateTime> {
    glib::DateTime::from_unix_local((at_ms / 1000) as i64).ok()
}

/// "Today", "Yesterday", "Monday 13 October".
fn day_label(at_ms: u64) -> String {
    let (Some(then), Ok(now)) = (local(at_ms), glib::DateTime::now_local()) else { return String::new() };
    let key = |d: &glib::DateTime| (d.year(), d.day_of_year());
    if key(&then) == key(&now) {
        t!("Today").into()
    } else if now.add_days(-1).is_ok_and(|y| key(&y) == key(&then)) {
        t!("Yesterday").into()
    } else {
        then.format("%A %e %B").map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
    }
}

fn time_label(at_ms: u64) -> String {
    local(at_ms).and_then(|t| t.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}
