//! A device's page: who it is and how it's connected, big buttons for what you do most,
//! then tabs: what to use it as, its inbox, its files, the timeline of everything shared
//! with it (grouped by day, with live progress), and its settings. A tab's content is
//! built when it shows.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use linksy_ipc::linksy_core::{ControlsChange, DeviceKind, DeviceView, Direction, Fingerprint, Uuid};
use linksy_i18n::t;
use linksy_ipc::{HistoryEntry, HistoryStatus, PhoneControlsInfo, Shared, TerminalStatus};

use super::controls::ControlsPanel;
use super::{LiveRow, Progress, drag_files, kind_icon, kind_name, progress_text, status_text};
use crate::fmt;

/// Things the page asks the window to do.
#[derive(Clone)]
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
    MicStop,
    /// What this phone's deck may do here.
    Deck,
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
    /// Let this device see and control the screen without asking.
    AllowScreen(bool),
    Locate,
    /// Ask a phone to open its camera and send the picture.
    Snap,
    /// Change some of the phone's controls, or keep its Do Not Disturb in step with this computer's.
    Controls(ControlsChange),
    DndSync(bool),
    /// Show this computer's notifications on the phone, or stop.
    ForwardNotifications(bool),
    /// Ask the phone to turn on `notifications` (the Inbox) or `gallery`.
    TurnOn(&'static str),
    /// A tab shows, to fill.
    Tab(Tab),
    /// The timeline's filter changed.
    FilterActivity,
    /// The page reached the end of the timeline: older entries.
    MoreActivity,
    /// The page reached the end of a folder's files: more of them.
    MoreFiles,
    /// Let this phone open terminals here (with the code), or stop it.
    AllowTerminal(bool),
    /// No code yet: choose one, then allow this phone.
    SetUpTerminal,
    ChangeTerminalCode,
    UnlockTerminal,
    EndTerminal(String),
}

type Handler = Rc<RefCell<Option<Rc<dyn Fn(Action)>>>>;
type NarrowHandler = Rc<RefCell<Option<Rc<dyn Fn(bool)>>>>;
type Later = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Use,
    Inbox,
    Gallery,
    Files,
    Transfers,
    History,
    Settings,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Tab::Use => "use",
            Tab::Inbox => "inbox",
            Tab::Gallery => "gallery",
            Tab::Files => "files",
            Tab::Transfers => "transfers",
            Tab::History => "history",
            Tab::Settings => "settings",
        }
    }
}

/// Most entries shown at once; older ones are still in `linksy history`.
const SHOWN: usize = 150;
/// Entries built at first, and added each time the page scrolls to the end: each row is a
/// dozen widgets, about 200 KB with their styles and text (desktop#26).
const FIRST: usize = 20;

pub struct DevicePage {
    /// Lays itself out by its own width (not the window's), so it fits beside the sidebar.
    pub root: adw::BreakpointBin,
    /// Header menu with the device's less common actions.
    pub menu: gtk::MenuButton,
    pub avatar: adw::Avatar,
    kind: gtk::Image,
    name: gtk::Label,
    /// How it's connected and how fast ("Wi-Fi · 4 ms"), as a chip.
    link_chip: gtk::Box,
    link_icon: gtk::Image,
    link: gtk::Label,
    /// Its battery: level and whether it charges, drawn, and the percentage.
    battery: Rc<std::cell::Cell<(f64, bool)>>,
    battery_chip: gtk::Box,
    battery_icon: gtk::DrawingArea,
    battery_label: gtk::Label,
    battery_bolt: gtk::Image,
    e2e: gtk::Box,
    reach: gtk::Box,
    reach_until: gtk::Label,
    /// Send Files beside the smaller actions; stacked when the page is narrow.
    actions: gtk::Box,
    send_tile: gtk::Button,
    ring_tile: gtk::Widget,
    locate_tile: gtk::Widget,
    snap_tile: gtk::Widget,
    ping_tile: gtk::Widget,
    /// "Use the phone as": a card per feature, two by two.
    use_as: gtk::Box,
    use_as_title: gtk::Label,
    /// Connect by USB Cable (faster for all of these), or that it is.
    use_as_hint: gtk::Button,
    use_as_hint_content: adw::ButtonContent,
    display_card: gtk::Box,
    webcam_card: gtk::Box,
    mic_card: gtk::Box,
    deck_card: gtk::Box,
    controls: ControlsPanel,
    deck_desc: gtk::Label,
    webcam_desc: gtk::Label,
    webcam_live: gtk::Label,
    webcam_start: gtk::Button,
    webcam_running: gtk::Box,
    mic_desc: gtk::Label,
    mic_live: gtk::Label,
    mic_start: gtk::Button,
    mic_running: gtk::Box,
    mic_wave: Wave,
    /// The tabs under the actions, and what each shows.
    tabs: gtk::ScrolledWindow,
    tab_buttons: Vec<(Tab, gtk::ToggleButton)>,
    /// The same tabs, pinned at the top once the page scrolls past them.
    sticky: gtk::Box,
    sticky_buttons: Vec<(Tab, gtk::ToggleButton)>,
    sticky_badge: gtk::Label,
    tab_stack: gtk::Stack,
    tab: Rc<std::cell::Cell<Tab>>,
    inbox_badge: gtk::Label,
    /// The window's inbox and files views go here.
    pub inbox_slot: gtk::Box,
    pub gallery_slot: gtk::Box,
    /// The Inbox and Gallery tabs: their view, or what to turn on on the phone ("off"), with
    /// a button that asks it when it can be asked.
    inbox_view: gtk::Stack,
    gallery_view: gtk::Stack,
    inbox_ask: gtk::Button,
    gallery_ask: gtk::Button,
    /// Whose Inbox and Gallery were on, to fill a tab when its phone turns it on.
    turned_on: std::cell::Cell<Option<(Fingerprint, bool, bool)>>,
    pub files_slot: gtk::Box,
    pub transfers_slot: gtk::Box,
    /// The device's switches, as plain rows.
    settings: gtk::ListBox,
    /// "Lock When Away", for phones; `syncing` while it's set from the service's state.
    lock_row: adw::SwitchRow,
    /// "Allow Everything From Away" (docs/protocol.md §4.13).
    away_row: adw::SwitchRow,
    /// This computer's notifications on the phone.
    forward_row: adw::SwitchRow,
    /// "Always Accept Files".
    accept_row: adw::SwitchRow,
    /// "Always Allow Screen and Control".
    screen_row: adw::SwitchRow,
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
    /// Which kind of entries the timeline shows.
    filter: Rc<std::cell::Cell<Filter>>,
    /// One heading and timeline per day.
    activity: gtk::Box,
    /// How many entries are built, and whether older ones are left to build.
    shown: Rc<std::cell::Cell<usize>>,
    more: Rc<std::cell::Cell<bool>>,
    grow: Rc<dyn Fn()>,
    scroller: gtk::ScrolledWindow,
    /// Opens a finished incoming row's files.
    on_activate: Rc<dyn Fn(&gtk::ListBoxRow)>,
    empty: gtk::Box,
    handler: Handler,
    /// Told when the page gets narrow, or wide again.
    on_narrow: NarrowHandler,
    /// Development aid (screenshots): LINKSY_SCREENSHOT_TAB=inbox opens that tab once it shows.
    wanted: std::cell::Cell<Option<Tab>>,
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

        // Hero: who this is, how it's connected and how fast, its battery.
        let avatar = adw::Avatar::builder().size(80).show_initials(true).build();
        let kind = gtk::Image::builder().pixel_size(14).css_classes(["hero-kind"]).halign(gtk::Align::End).valign(gtk::Align::End).build();
        let avatar_overlay = gtk::Overlay::builder().child(&avatar).halign(gtk::Align::Start).valign(gtk::Align::Center).build();
        avatar_overlay.add_overlay(&kind);
        let name = gtk::Label::builder().css_classes(["title-1"]).xalign(0.0).wrap(true).build();
        let link_icon = gtk::Image::builder().pixel_size(14).build();
        let link = gtk::Label::builder().css_classes(["numeric"]).build();
        let link_chip = gtk::Box::builder().spacing(6).valign(gtk::Align::Center).css_classes(["link-chip"]).build();
        link_chip.append(&link_icon);
        link_chip.append(&link);
        let battery: Rc<std::cell::Cell<(f64, bool)>> = Rc::default();
        let battery_icon = gtk::DrawingArea::builder().content_width(22).content_height(12).valign(gtk::Align::Center).build();
        let level = battery.clone();
        battery_icon.set_draw_func(move |area, cr, w, h| {
            let (level, charging) = level.get();
            let (w, h) = (w as f64, h as f64);
            let fg = area.color();
            let body = w - 3.0;
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, fg.alpha() as f64);
            cr.set_line_width(1.4);
            rounded(cr, 0.7, 0.7, body - 1.4, h - 1.4, 3.0);
            let _ = cr.stroke();
            cr.rectangle(body + 0.6, h / 2.0 - 2.2, 1.8, 4.4);
            let _ = cr.fill();
            if level <= 0.15 && !charging {
                cr.set_source_rgb(0.88, 0.11, 0.14);
            }
            let inner = (body - 4.6) * level.clamp(0.0, 1.0);
            if inner > 0.0 {
                rounded(cr, 2.3, 2.3, inner, h - 4.6, 1.4);
                let _ = cr.fill();
            }
        });
        let battery_label = gtk::Label::builder().css_classes(["numeric"]).build();
        let bolt = gtk::Image::builder().icon_name("thunderbolt-symbolic").pixel_size(12).build();
        let battery_chip = gtk::Box::builder().spacing(6).valign(gtk::Align::Center).css_classes(["battery-chip"]).build();
        battery_chip.append(&battery_icon);
        battery_chip.append(&battery_label);
        battery_chip.append(&bolt);
        let chips = gtk::Box::builder().spacing(8).build();
        chips.append(&link_chip);
        chips.append(&battery_chip);
        let e2e = gtk::Box::builder().spacing(5).css_classes(["dim-label"]).build();
        e2e.append(&gtk::Image::builder().icon_name("channel-secure-symbolic").pixel_size(12).build());
        e2e.append(&gtk::Label::builder().label(t!("Encrypted end to end")).xalign(0.0).css_classes(["caption"]).build());
        let who = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).hexpand(true).valign(gtk::Align::Center).build();
        who.append(&name);
        who.append(&chips);
        who.append(&e2e);
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
        let snap_tile = tile("camera-photo-symbolic", t!("Snap from Phone"), t!("Take a picture, get it here"), false, || Action::Snap);
        let ping_tile = tile("preferences-system-notifications-symbolic", t!("Ping"), t!("Show a notification"), false, || Action::Ping);
        // Not expanding: their insides push text to the bottom, not the page.
        let actions = gtk::Box::builder().spacing(10).homogeneous(true).vexpand(false).build();
        actions.append(&send_tile);
        actions.append(&small);

        // "Use the phone as": the big features, each a card of its own.
        let card = |icon: &str, title: &str, desc: &str| -> (gtk::Box, gtk::Box, gtk::Label, gtk::Label) {
            // The title beside its icon, the description small under them
            let image = gtk::Image::builder().icon_name(icon).pixel_size(22).valign(gtk::Align::Start).css_classes(["card-icon", "small"]).build();
            let badge = gtk::Label::builder().label(t!("Live")).halign(gtk::Align::Start).visible(false).css_classes(["live-badge"]).build();
            let heading = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).hexpand(true).valign(gtk::Align::Center).build();
            heading.append(&gtk::Label::builder().label(title).xalign(0.0).wrap(true).css_classes(["title-4"]).build());
            heading.append(&badge);
            let top = gtk::Box::builder().spacing(10).build();
            top.append(&heading);
            top.append(&image);
            let description = gtk::Label::builder().label(desc).xalign(0.0).wrap(true).css_classes(["caption", "dim-label"]).build();
            let buttons = gtk::Box::builder().spacing(8).vexpand(true).valign(gtk::Align::End).build();
            let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).hexpand(true).css_classes(["feature-card"]).build();
            b.append(&top);
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
        // While it's on: what it hears, and Stop.
        let mic_wave = Wave::new();
        let mic_stop = gtk::Button::builder().label(t!("Stop")).build();
        let e = emit.clone();
        mic_stop.connect_clicked(move |_| e(Action::MicStop));
        let mic_running = gtk::Box::builder().spacing(8).hexpand(true).visible(false).build();
        mic_running.append(&mic_wave.area);
        mic_running.append(&mic_stop);
        buttons.append(&mic_start);
        buttons.append(&mic_running);
        let (deck_card, buttons, deck_desc, _) = card("view-grid-symbolic", t!("Deck"), "");
        buttons.append(&start(t!("Choose Actions…"), || Action::Deck));
        let cards = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(4)
            .max_children_per_line(4)
            .column_spacing(10)
            .row_spacing(10)
            .vexpand(false)
            .css_classes(["tiles"])
            .build();
        for card in [&display_card, &webcam_card, &mic_card, &deck_card] {
            let child = gtk::FlowBoxChild::builder().child(card).focusable(false).build();
            // A hidden card leaves no gap
            card.bind_property("visible", &child, "visible").sync_create().build();
            cards.append(&child);
        }
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

        // The device's switches: plain rows.
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
        let forward_row = adw::SwitchRow::builder().title(t!("This Computer's Notifications on the Phone")).subtitle_lines(3).visible(false).build();
        forward_row.add_prefix(&gtk::Image::builder().icon_name("preferences-system-notifications-symbolic").build());
        let (e, s) = (emit.clone(), syncing.clone());
        forward_row.connect_active_notify(move |row| {
            if !s.get() {
                e(Action::ForwardNotifications(row.is_active()));
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
        let screen_row = adw::SwitchRow::builder().title(t!("Always Allow Screen and Control")).subtitle_lines(3).build();
        screen_row.add_prefix(&gtk::Image::builder().icon_name("video-display-symbolic").build());
        let (e, s) = (emit.clone(), syncing.clone());
        screen_row.connect_active_notify(move |row| {
            if !s.get() {
                e(Action::AllowScreen(row.is_active()));
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
        settings.append(&lock_row);
        settings.append(&accept_row);
        settings.append(&screen_row);
        settings.append(&forward_row);
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
        let filters = gtk::Box::builder().spacing(6).hexpand(true).build();
        let activity_header = gtk::Box::builder().spacing(6).build();
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
        activity_header.append(&clear);
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

        // Tabs: a line under them all, the one showing underlined in the accent.
        let use_page = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(22).build();
        use_page.append(&use_as);
        let controls = {
            let (e1, e2) = (emit.clone(), emit.clone());
            ControlsPanel::new(Rc::new(move |c| e1(Action::Controls(c))), Rc::new(move |on| e2(Action::DndSync(on))))
        };
        use_page.append(&controls.root);
        use_page.append(&terminal_box);
        use_page.append(&terminals);
        let inbox_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let gallery_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let off = |slot: &gtk::Box, icon: &str, title: &str, how: &str, feature: &'static str, help: &str| {
            let page = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).halign(gtk::Align::Center).margin_top(36).margin_bottom(36).css_classes(["feature-off"]).build();
            page.append(&gtk::Image::builder().icon_name(icon).pixel_size(64).css_classes(["dim-label"]).build());
            page.append(&gtk::Label::builder().label(title).wrap(true).justify(gtk::Justification::Center).css_classes(["title-2"]).build());
            page.append(&gtk::Label::builder().label(how).wrap(true).max_width_chars(48).justify(gtk::Justification::Center).css_classes(["dim-label"]).build());
            let ask = gtk::Button::builder().label(t!("Ask the Phone")).halign(gtk::Align::Center).margin_top(12).css_classes(["pill", "suggested-action"]).build();
            let e = emit.clone();
            ask.connect_clicked(move |_| e(Action::TurnOn(feature)));
            let how_to = gtk::Button::builder().label(t!("How to Turn It On")).halign(gtk::Align::Center).css_classes(["pill"]).build();
            let (e, help) = (emit.clone(), help.to_string());
            how_to.connect_clicked(move |_| e(Action::OpenLink(help.clone())));
            page.append(&ask);
            page.append(&how_to);
            let view = gtk::Stack::builder().vhomogeneous(false).build();
            view.add_named(slot, Some("on"));
            view.add_named(&page, Some("off"));
            (view, ask)
        };
        let (inbox_view, inbox_ask) = off(
            &inbox_slot,
            "mail-unread-symbolic",
            t!("The Inbox Is Off"),
            t!("Turn on “Show this phone's notifications” in Linksy on the phone, under Settings › With your computers."),
            "notifications",
            "https://linksy.diary.mg/help/inbox",
        );
        let (gallery_view, gallery_ask) = off(
            &gallery_slot,
            "image-x-generic-symbolic",
            t!("The Gallery Is Off"),
            t!("Turn on “Photos on your computers” in Linksy on the phone, under Settings › Files, and let it see all photos."),
            "gallery",
            "https://linksy.diary.mg/help/gallery",
        );
        let files_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let transfers_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let forget_button = gtk::Button::builder().label(t!("Forget Device…")).halign(gtk::Align::Start).css_classes(["error"]).build();
        let e = emit.clone();
        forget_button.connect_clicked(move |_| e(Action::Forget));
        let settings_page = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(22).build();
        settings_page.append(&settings);
        settings_page.append(&forget_button);
        let tab_stack = gtk::Stack::builder().vhomogeneous(false).hhomogeneous(false).transition_type(gtk::StackTransitionType::Crossfade).transition_duration(120).build();
        tab_stack.add_named(&use_page, Some(Tab::Use.name()));
        tab_stack.add_named(&inbox_view, Some(Tab::Inbox.name()));
        tab_stack.add_named(&gallery_view, Some(Tab::Gallery.name()));
        tab_stack.add_named(&files_slot, Some(Tab::Files.name()));
        tab_stack.add_named(&transfers_slot, Some(Tab::Transfers.name()));
        tab_stack.add_named(&activity_box, Some(Tab::History.name()));
        tab_stack.add_named(&settings_page, Some(Tab::Settings.name()));
        let tab = Rc::new(std::cell::Cell::new(Tab::Use));
        let badge = || gtk::Label::builder().css_classes(["count-badge", "numeric"]).valign(gtk::Align::Center).visible(false).build();
        let (inbox_badge, sticky_badge) = (badge(), badge());
        let tab_row = gtk::Box::builder().spacing(24).css_classes(["page-tabs"]).build();
        // Scrolled past, the tabs stay at the top of the page: a copy, kept in step.
        let sticky_row = gtk::Box::builder().spacing(24).css_classes(["page-tabs"]).build();
        let (mut tab_buttons, mut sticky_buttons) = (Vec::new(), Vec::new());
        let (mut group, mut sticky_group): (Option<gtk::ToggleButton>, Option<gtk::ToggleButton>) = (None, None);
        let to_tabs: Later = Rc::default();
        for (which, icon, label) in [
            (Tab::Use, "video-joined-displays-symbolic", t!("Use As")),
            (Tab::Inbox, "mail-unread-symbolic", t!("Inbox")),
            (Tab::Gallery, "image-x-generic-symbolic", t!("Gallery")),
            (Tab::Files, "folder-symbolic", t!("Files")),
            (Tab::Transfers, "network-transmit-receive-symbolic", t!("Transfers")),
            (Tab::History, "document-open-recent-symbolic", t!("History")),
            (Tab::Settings, "emblem-system-symbolic", t!("Settings")),
        ] {
            let button = |badge: &gtk::Label| {
                let inner = gtk::Box::builder().spacing(7).build();
                inner.append(&gtk::Image::from_icon_name(icon));
                inner.append(&gtk::Label::new(Some(label)));
                if which == Tab::Inbox {
                    inner.append(badge);
                }
                gtk::ToggleButton::builder().child(&inner).css_classes(["page-tab"]).active(which == Tab::Use).build()
            };
            let (b, s) = (button(&inbox_badge), button(&sticky_badge));
            b.set_group(group.as_ref());
            group.get_or_insert(b.clone());
            s.set_group(sticky_group.as_ref());
            sticky_group.get_or_insert(s.clone());
            let (e, current, stack, twin) = (emit.clone(), tab.clone(), tab_stack.clone(), s.clone());
            b.connect_toggled(move |b| {
                if b.is_active() {
                    twin.set_active(true);
                    stack.set_visible_child_name(which.name());
                    if current.replace(which) != which {
                        e(Action::Tab(which));
                    }
                }
            });
            let (twin, to_tabs) = (b.clone(), to_tabs.clone());
            s.connect_toggled(move |s| {
                if s.is_active() {
                    twin.set_active(true);
                    let f = to_tabs.borrow().clone();
                    if let Some(f) = f {
                        f();
                    }
                }
            });
            tab_row.append(&b);
            sticky_row.append(&s);
            tab_buttons.push((which, b));
            sticky_buttons.push((which, s));
        }
        let tabs = gtk::ScrolledWindow::builder().child(&tab_row).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).margin_top(10).css_classes(["inner-scroll"]).build();
        let tabbed = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(20).build();
        tabbed.append(&tabs);
        tabbed.append(&tab_stack);

        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(26).margin_top(6).margin_bottom(32).margin_start(16).margin_end(16).build();
        content.append(&hero);
        content.append(&actions);
        content.append(&pair_box);
        content.append(&tabbed);
        let clamp = adw::Clamp::builder().maximum_size(860).tightening_threshold(600).child(&content).build();
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        column.append(&staged);
        column.append(&clamp);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&column).build();
        let (shown, more) = (Rc::new(std::cell::Cell::new(FIRST)), Rc::new(std::cell::Cell::new(false)));
        let (e, s, m, t) = (emit.clone(), shown.clone(), more.clone(), tab.clone());
        let grow: Rc<dyn Fn()> = Rc::new(move || {
            if t.get() == Tab::History && m.replace(false) {
                s.set((s.get() + FIRST).min(SHOWN));
                let e = e.clone();
                glib::idle_add_local_once(move || e(Action::MoreActivity));
            }
        });
        let (g, e, t) = (grow.clone(), emit.clone(), tab.clone());
        scroller.connect_edge_reached(move |_, pos| {
            if pos == gtk::PositionType::Bottom {
                g();
                if t.get() == Tab::Files {
                    e(Action::MoreFiles);
                }
            }
        });
        // A window taller than the entries built never scrolls to their end: more once it's
        // laid out, and after each batch (`show_history`)
        let g = grow.clone();
        scroller.vadjustment().connect_changed(move |a| {
            if a.page_size() > 0.0 && a.upper() <= a.page_size() {
                g();
            }
        });
        let sticky_tabs = gtk::ScrolledWindow::builder().child(&sticky_row).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).css_classes(["inner-scroll"]).halign(gtk::Align::Start).build();
        let sticky = gtk::Box::builder().valign(gtk::Align::Start).css_classes(["sticky-tabs"]).visible(false).build();
        sticky.append(&sticky_tabs);
        let overlay = gtk::Overlay::builder().child(&scroller).build();
        overlay.add_overlay(&sticky);
        let top_of_tabs = {
            let (bar, column) = (tabs.clone(), column.clone());
            move || bar.compute_point(&column, &gtk::graphene::Point::new(0.0, 0.0)).map(|p| p.y() as f64)
        };
        // Shown once they're scrolled past, exactly where they are across the page.
        let place = {
            let (pinned, pinned_tabs, bar, over, top) = (sticky.clone(), sticky_tabs.clone(), tabs.clone(), overlay.clone(), top_of_tabs.clone());
            move |a: &gtk::Adjustment| {
                let show = bar.is_visible() && top().is_some_and(|y| a.value() > y);
                if show && let Some(p) = bar.compute_point(&over, &gtk::graphene::Point::new(0.0, 0.0)) {
                    pinned_tabs.set_margin_start(p.x().max(0.0) as i32);
                    pinned_tabs.set_size_request(bar.width(), -1);
                }
                pinned.set_visible(show);
            }
        };
        let p = place.clone();
        scroller.vadjustment().connect_value_changed(move |a| p(a));
        scroller.vadjustment().connect_changed(move |a| place(a));
        // A tab chosen up there: its top just under them.
        let (pinned, adjustment) = (sticky.clone(), scroller.vadjustment());
        *to_tabs.borrow_mut() = Some(Rc::new(move || {
            if pinned.is_visible()
                && let Some(y) = top_of_tabs()
            {
                let a = adjustment.clone();
                glib::idle_add_local_once(move || a.set_value(y.min(a.upper() - a.page_size())));
            }
        }));
        // Development aid (screenshots): LINKSY_SCREENSHOT_SCROLL=600 scrolls the page down.
        // Again until the tab's content is long enough to get there.
        if let Some(y) = std::env::var("LINKSY_SCREENSHOT_SCROLL").ok().and_then(|y| y.parse::<f64>().ok()) {
            let (a, mut tries) = (scroller.vadjustment(), 0);
            glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                a.set_value(y);
                tries += 1;
                if a.value() >= y - 1.0 || tries > 20 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
            });
        }
        let root = adw::BreakpointBin::builder().width_request(300).height_request(300).child(&overlay).build();
        // Narrower and narrower: stack Send Files over the tiles, then the cards, then the
        // header, the "Use as" heading and the timeline's filters, then the tiles. (Only
        // the last matching breakpoint applies, so each repeats the ones before.)
        // Responsive design, written by hand, in Rust. Be gentle with it.
        let vertical = gtk::Orientation::Vertical.to_value();
        let (one, two) = (1u32.to_value(), 2u32.to_value());
        let changes: [(&gtk::Widget, &str, &glib::Value); 6] = [
            (actions.upcast_ref(), "orientation", &vertical),
            (hero.upcast_ref(), "orientation", &vertical),
            (use_as_head.upcast_ref(), "orientation", &vertical),
            (activity_header.upcast_ref(), "orientation", &vertical),
            (small.upcast_ref(), "min-children-per-line", &one),
            (small.upcast_ref(), "max-children-per-line", &one),
        ];
        let mut narrow = Vec::new();
        // The Use As cards: four per line, then two, then one
        for (width, count, per_line) in [("760sp", 1, &two), ("680sp", 1, &two), ("540sp", 4, &one), ("400sp", 6, &one)] {
            let b = adw::Breakpoint::new(adw::BreakpointCondition::parse(&format!("max-width: {width}")).expect("breakpoint"));
            for (widget, property, value) in &changes[..count] {
                b.add_setter(*widget, property, Some(value));
            }
            b.add_setter(&cards, "min-children-per-line", Some(per_line));
            b.add_setter(&cards, "max-children-per-line", Some(per_line));
            if width != "760sp" {
                narrow.push(b.clone());
            }
            root.add_breakpoint(b);
        }
        // The inbox shows one side at a time from the second one on.
        let on_narrow: NarrowHandler = Rc::default();
        let tell = on_narrow.clone();
        root.connect_current_breakpoint_notify(move |bin| {
            let is = bin.current_breakpoint().is_some_and(|b| narrow.contains(&b));
            let f = tell.borrow().clone();
            if let Some(f) = f {
                f(is);
            }
        });

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
            name,
            link_chip,
            link_icon,
            link,
            battery,
            battery_chip,
            battery_icon,
            battery_label,
            battery_bolt: bolt,
            e2e,
            reach,
            reach_until,
            actions,
            send_tile,
            ring_tile,
            locate_tile,
            snap_tile,
            ping_tile,
            use_as,
            use_as_title,
            use_as_hint,
            use_as_hint_content,
            display_card,
            webcam_card,
            mic_card,
            deck_card,
            controls,
            deck_desc,
            mic_desc,
            mic_live,
            mic_start,
            mic_running,
            mic_wave,
            webcam_desc,
            webcam_live,
            webcam_start,
            webcam_running,
            tabs,
            tab_buttons,
            sticky,
            sticky_buttons,
            sticky_badge,
            tab_stack,
            tab,
            inbox_badge,
            inbox_slot,
            gallery_slot,
            inbox_view,
            gallery_view,
            inbox_ask,
            gallery_ask,
            turned_on: std::cell::Cell::new(None),
            files_slot,
            transfers_slot,
            settings,
            lock_row,
            away_row,
            forward_row,
            accept_row,
            screen_row,
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
            filter,
            activity,
            shown,
            more,
            grow,
            scroller,
            on_activate,
            empty,
            handler,
            on_narrow,
            wanted: std::cell::Cell::new(std::env::var("LINKSY_SCREENSHOT_TAB").ok().and_then(|t| [Tab::Use, Tab::Inbox, Tab::Gallery, Tab::Files, Tab::Transfers, Tab::History, Tab::Settings].into_iter().find(|w| w.name() == t))),
            received,
        }
    }

    pub fn connect_narrow(&self, f: impl Fn(bool) + 'static) {
        *self.on_narrow.borrow_mut() = Some(Rc::new(f));
    }

    pub fn tab(&self) -> Tab {
        self.tab.get()
    }

    pub fn select_tab(&self, tab: Tab) {
        if let Some((_, b)) = self.tab_buttons.iter().find(|(t, b)| *t == tab && b.is_visible()) {
            b.set_active(true);
        }
    }

    /// Back up to the tabs, when the page is scrolled past them.
    pub fn scroll_to_tabs(&self) {
        let a = self.scroller.vadjustment();
        let Some(column) = self.scroller.child().and_then(|viewport| viewport.first_child()) else { return };
        if let Some(p) = self.tabs.compute_point(&column, &gtk::graphene::Point::new(0.0, 0.0))
            && a.value() > p.y() as f64
        {
            a.set_value(p.y() as f64);
        }
    }

    /// Unread conversations and other notifications, on the Inbox tab.
    pub fn set_inbox_count(&self, count: u32) {
        for badge in [&self.inbox_badge, &self.sticky_badge] {
            badge.set_label(&count.to_string());
            badge.set_visible(count > 0);
        }
    }

    /// Whether this computer's notifications go to the phone; none hides the switch (the phone
    /// can't show them, or this computer can't listen for them).
    pub fn show_forward(&self, device: &str, info: Option<&linksy_ipc::ComputerNotifications>) {
        let shown = info.is_some_and(|i| i.available);
        self.forward_row.set_visible(shown);
        self.forward_row.set_subtitle(&glib::markup_escape_text(&t!("What comes up here also comes up on {device}, except Linksy's own and while Do Not Disturb is on. Off until you turn it on.", device)));
        self.syncing.set(true);
        self.forward_row.set_active(info.is_some_and(|i| i.on));
        self.syncing.set(false);
        self.settings_visible(true);
    }

    /// The phone's controls as last known; none hides them.
    pub fn show_controls(&self, info: Option<&PhoneControlsInfo>) {
        self.controls.show(info);
    }

    fn any_card(&self) -> bool {
        [&self.display_card, &self.webcam_card, &self.mic_card, &self.deck_card].iter().any(|c| c.is_visible())
    }

    /// Show the tabs this device has, and one of them.
    fn update_tabs(&self, d: &DeviceView) {
        let phone = matches!(d.device.kind, DeviceKind::Phone | DeviceKind::Tablet);
        let any_use = self.any_card() || self.terminal_box.is_visible() || self.terminals.is_visible() || (d.trusted && super::can(&d.device, "phone_controls"));
        // Shown while off too, saying how to turn them on
        let has = |t: Tab| match t {
            Tab::Use => any_use,
            Tab::Inbox => d.device.kind == DeviceKind::Phone,
            Tab::Gallery => phone,
            Tab::Files => phone && super::can(&d.device, "fs"),
            Tab::Transfers | Tab::History | Tab::Settings => true,
        };
        let (inbox_on, gallery_on) = (super::inbox_on(&d.device), super::can(&d.device, "gallery"));
        self.inbox_view.set_visible_child_name(if inbox_on { "on" } else { "off" });
        self.gallery_view.set_visible_child_name(if gallery_on { "on" } else { "off" });
        let askable = super::can(&d.device, "turn_on");
        self.inbox_ask.set_visible(askable);
        self.gallery_ask.set_visible(askable);
        if let Some((id, inbox_was, gallery_was)) = self.turned_on.replace(Some((d.device.id, inbox_on, gallery_on)))
            && id == d.device.id
        {
            let tab = self.tab.get();
            let handler = self.handler.borrow().clone();
            if let Some(f) = handler.filter(|_| (tab == Tab::Inbox && inbox_on && !inbox_was) || (tab == Tab::Gallery && gallery_on && !gallery_was)) {
                f(Action::Tab(tab));
            }
        }
        for (t, b) in self.tab_buttons.iter().chain(&self.sticky_buttons) {
            b.set_visible(d.trusted && has(*t));
        }
        if !d.trusted {
            self.sticky.set_visible(false);
        }
        self.tabs.set_visible(d.trusted);
        self.tab_stack.set_visible(d.trusted);
        if let Some(want) = self.wanted.take() {
            match self.tab_buttons.iter().find(|(t, _)| *t == want && d.trusted && has(*t)) {
                Some((_, b)) => b.set_active(true),
                None => self.wanted.set(Some(want)),
            }
        }
        if !has(self.tab.get())
            && let Some((_, b)) = self.tab_buttons.iter().find(|(t, _)| has(*t))
        {
            b.set_active(true);
        }
    }

    /// How it's connected and how fast, and its battery, which change while it shows.
    pub fn show_link(&self, d: &DeviceView) {
        let available = d.connected || d.nearby;
        let phone = matches!(d.device.kind, DeviceKind::Phone | DeviceKind::Tablet);
        let (icon, what) = if d.connected && d.away {
            ("network-wireless-hotspot-symbolic", t!("Away from home").to_string())
        } else if d.connected && d.usb {
            ("media-removable-symbolic", t!("USB").to_string())
        } else if d.connected && phone {
            ("network-wireless-symbolic", t!("Wi-Fi").to_string())
        } else if d.connected {
            ("network-wired-symbolic", t!("Local network").to_string())
        } else {
            ("network-offline-symbolic", status_text(d).to_string())
        };
        self.link_icon.set_icon_name(Some(icon));
        match d.latency_ms.filter(|_| d.connected) {
            Some(ms) => self.link.set_label(&t!("{link} · {ms} ms", link = what, ms = ms.max(1))),
            None => self.link.set_label(&what),
        }
        for class in ["away", "usb", "offline"] {
            self.link_chip.remove_css_class(class);
        }
        if d.connected && d.away {
            self.link_chip.add_css_class("away");
        } else if d.connected && d.usb {
            self.link_chip.add_css_class("usb");
        } else if !d.connected {
            self.link_chip.add_css_class("offline");
        }
        let battery = d.battery.filter(|_| available);
        self.battery_chip.set_visible(battery.is_some());
        if let Some(b) = battery {
            self.battery.set((b.level as f64 / 100.0, b.charging));
            self.battery_label.set_label(&format!("{}%", b.level));
            self.battery_bolt.set_visible(b.charging);
            self.battery_chip.set_tooltip_text(Some(&if b.charging { t!("Battery {level}%, charging", level = b.level) } else { t!("Battery {level}%", level = b.level) }));
            self.battery_icon.queue_draw();
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
        self.update_tabs(d);
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
        if !live {
            self.mic_wave.clear();
        }
        if live {
            self.mic_card.add_css_class("live");
        } else {
            self.mic_card.remove_css_class("live");
        }
    }

    /// The microphone's wave, for whatever measures it.
    pub fn mic_wave(&self) -> Wave {
        self.mic_wave.clone()
    }

    pub fn set_drop_active(&self, on: bool) {
        if on {
            self.send_tile.add_css_class("drop-active");
        } else {
            self.send_tile.remove_css_class("drop-active");
        }
    }

    pub fn show_device(&self, d: &DeviceView, staged: &[PathBuf], locks: bool) {
        let phone = matches!(d.device.kind, DeviceKind::Phone | DeviceKind::Tablet);
        self.avatar.set_text(Some(&d.device.name));
        self.kind.set_icon_name(Some(kind_icon(d.device.kind)));
        self.kind.set_tooltip_text(Some(&kind_name(&d.device)));
        self.name.set_label(&d.device.name);
        self.show_link(d);
        self.e2e.set_visible(d.trusted);
        self.reach.set_visible(d.trusted && d.away_until.is_some());
        self.reach_until.set_label(&d.away_until.map(fmt::day).unwrap_or_default());

        let ring = phone && super::can(&d.device, "ring");
        self.actions.set_visible(d.trusted);
        self.ring_tile.set_visible(ring);
        self.locate_tile.set_visible(d.device.kind == DeviceKind::Phone && super::can(&d.device, "locate"));
        self.snap_tile.set_visible(d.trusted && super::can(&d.device, "snap"));
        self.ping_tile.set_visible(!ring);

        self.display_card.set_visible(super::can(&d.device, "display"));
        self.webcam_card.set_visible(super::can(&d.device, "camera"));
        self.mic_card.set_visible(super::can(&d.device, "microphone"));
        // Whatever the phone says it can do: the deck is this computer's side
        self.deck_card.set_visible(phone);
        self.deck_desc.set_label(&t!("Buttons for this computer on {device}", device = d.device.name));
        self.use_as.set_visible(d.trusted && self.any_card());
        self.use_as_title.set_label(&t!("Use {device} as", device = d.device.name));
        self.use_as_hint.set_visible(d.device.os == "android");
        self.use_as_hint_content.set_label(if d.connected && d.usb { t!("Connected by USB") } else { t!("Faster by USB Cable") });

        self.lock_row.set_visible(d.device.kind == DeviceKind::Phone && super::can(&d.device, "heartbeat"));
        self.lock_row.set_subtitle(&glib::markup_escape_text(&t!("Lock this computer when {device} leaves. It never unlocks it.", device = d.device.name)));
        // Away from home: only for devices that told this one how to reach them.
        self.away_row.set_visible(d.trusted && d.away_until.is_some());
        self.away_row.set_subtitle(&glib::markup_escape_text(&t!(
            "Away from home, {device} can always send the clipboard, links and files and ring. On: screens, control, its files, camera and microphone too.",
            device = d.device.name
        )));
        self.accept_row.set_subtitle(&glib::markup_escape_text(&t!("Files from {device} are saved without asking.", device = d.device.name)));
        self.screen_row.set_subtitle(&glib::markup_escape_text(&t!(
            "{device} can see and control this screen without asking, on the same network or by cable. Linksy still tells you when it starts.",
            device = d.device.name
        )));
        self.settings_visible(d.trusted);
        self.syncing.set(true);
        self.accept_row.set_active(d.auto_accept);
        self.screen_row.set_active(d.allow_screen);
        self.lock_row.set_active(locks);
        self.away_row.set_active(d.away_all);
        self.syncing.set(false);
        self.pair_box.set_visible(!d.trusted);
        self.menu.set_visible(d.trusted);
        self.update_tabs(d);

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

    /// Another device: its timeline starts again from the newest entries.
    pub(super) fn first_entries(&self) {
        self.shown.set(FIRST);
    }

    /// Rebuild the timeline. Returns the widgets of running transfers, for progress.
    pub(super) fn show_history(&self, history: &[HistoryEntry], progress: &HashMap<Uuid, Progress>) -> HashMap<Uuid, LiveRow> {
        while let Some(child) = self.activity.first_child() {
            self.activity.remove(&child);
        }
        self.received.borrow_mut().clear();
        if self.tab.get() != Tab::History {
            return HashMap::new();
        }
        let filter = self.filter.get();
        let mut days: Vec<(String, Vec<&HistoryEntry>)> = Vec::new();
        let matching: Vec<&HistoryEntry> = history.iter().take(SHOWN).filter(|e| filter.shows(&e.what)).collect();
        self.more.set(matching.len() > self.shown.get());
        for e in matching.into_iter().take(self.shown.get()) {
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
        if self.more.get() {
            let (scroller, grow) = (self.scroller.clone(), self.grow.clone());
            glib::idle_add_local_full(glib::Priority::LOW, move || {
                let a = scroller.vadjustment();
                if a.page_size() > 0.0 && a.upper() <= a.page_size() {
                    grow();
                }
                glib::ControlFlow::Break
            });
        }
        live
    }

    /// One entry: its time, a node on the timeline, and what happened.
    fn entry_row(&self, e: &HistoryEntry, progress: Option<&Progress>, last: bool) -> (gtk::ListBoxRow, Option<LiveRow>) {
        let incoming = e.direction == Direction::Incoming;
        let (icon, title) = match &e.what {
            Shared::Files { import: true, .. } => ("image-x-generic-symbolic", fmt::plural(e.what.file_count(), "photo")),
            Shared::Files { files, .. } => (
                if incoming { "folder-download-symbolic" } else { "document-send-symbolic" },
                match (files.as_slice(), e.what.file_count()) {
                    ([one], 1) => fmt::file_name(one),
                    (_, count) => fmt::plural(count, "item"),
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
            (HistoryStatus::Done, Shared::Files { saved, more, .. }) if incoming && !saved.is_empty() => {
                let first = saved[0].clone();
                action(t!("Show in Folder"), Box::new(move || Action::ShowInFolder(first.clone())));
                row.set_activatable(true);
                row.set_widget_name(&e.id.to_string());
                // The history kept only the first of many: it opens their folder
                let opens = match saved[0].parent() {
                    Some(folder) if *more > 0 => vec![folder.to_path_buf()],
                    _ => {
                        row.set_tooltip_text(Some(if saved.len() == 1 { t!("Open, or drag it anywhere") } else { t!("Open the folder, or drag the files anywhere") }));
                        row.add_controller(drag_files(saved));
                        saved.clone()
                    }
                };
                self.received.borrow_mut().insert(e.id.to_string(), opens);
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

/// The microphone's sound as it arrives: its latest levels as bars, newest on the right.
#[derive(Clone)]
pub struct Wave {
    area: gtk::DrawingArea,
    levels: Rc<RefCell<VecDeque<f64>>>,
}

impl Wave {
    const BAR: f64 = 3.0;
    const STEP: f64 = 6.0;
    const KEEP: usize = 160;

    fn new() -> Self {
        let area = gtk::DrawingArea::builder().content_height(34).hexpand(true).css_classes(["mic-wave"]).build();
        let levels: Rc<RefCell<VecDeque<f64>>> = Rc::default();
        let l = levels.clone();
        area.set_draw_func(move |area, cr, w, h| {
            let (c, levels) = (area.color(), l.borrow());
            let (w, h) = (w as f64, h as f64);
            let bars = (w / Self::STEP) as usize;
            let room = h - Self::BAR - 12.0;
            cr.set_line_width(Self::BAR);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            for i in 0..bars {
                let level = (levels.len() + i).checked_sub(bars).and_then(|j| levels.get(j)).copied().unwrap_or(0.0);
                let (x, half) = (w - (bars - i) as f64 * Self::STEP + Self::STEP / 2.0, level * room / 2.0);
                cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.35 + 0.65 * (i + 1) as f64 / bars as f64);
                cr.move_to(x, h / 2.0 - half);
                cr.line_to(x, h / 2.0 + half);
                let _ = cr.stroke();
            }
        });
        Self { area, levels }
    }

    /// The latest level, 0 (silent) to 1 (full scale).
    pub fn push(&self, level: f64) {
        let mut levels = self.levels.borrow_mut();
        if levels.len() == Self::KEEP {
            levels.pop_front();
        }
        levels.push_back(level);
        drop(levels);
        self.area.queue_draw();
    }

    fn clear(&self) {
        self.levels.borrow_mut().clear();
        self.area.queue_draw();
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
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
pub(super) fn day_label(at_ms: u64) -> String {
    day_heading(at_ms, |_| "%A %e %B")
}

/// "Today", "Yesterday", then the date as `far` formats it, given whether it falls in
/// this year.
pub(super) fn day_heading(at_ms: u64, far: impl Fn(bool) -> &'static str) -> String {
    let (Some(then), Ok(now)) = (local(at_ms), glib::DateTime::now_local()) else { return String::new() };
    let key = |d: &glib::DateTime| (d.year(), d.day_of_year());
    if key(&then) == key(&now) {
        t!("Today").into()
    } else if now.add_days(-1).is_ok_and(|y| key(&y) == key(&then)) {
        t!("Yesterday").into()
    } else {
        then.format(far(then.year() == now.year())).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
    }
}

pub(super) fn time_label(at_ms: u64) -> String {
    local(at_ms).and_then(|t| t.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}
