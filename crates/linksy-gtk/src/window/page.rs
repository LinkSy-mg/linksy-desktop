//! A device's page: who it is, big buttons for what you can do with it, and a timeline of
//! everything shared with it, grouped by day, with live progress for running transfers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use linksy_ipc::linksy_core::{DeviceView, Direction, Uuid};
use linksy_ipc::{HistoryEntry, HistoryStatus, Shared};

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
    CameraPreview,
    CameraStop,
    Pair,
    SendStaged,
    Forget,
    ClearHistory,
    Cancel(Uuid),
    Open(Vec<PathBuf>),
    ShowInFolder(PathBuf),
    Copy(String),
}

type Handler = Rc<RefCell<Option<Rc<dyn Fn(Action)>>>>;

/// Most entries shown at once; older ones are still in `linksy history`.
const SHOWN: usize = 150;

pub struct DevicePage {
    pub root: gtk::ScrolledWindow,
    /// Header menu with the device's less common actions.
    pub menu: gtk::MenuButton,
    avatar: adw::Avatar,
    kind: gtk::Image,
    name: gtk::Label,
    status: gtk::Label,
    dot: gtk::Box,
    /// Four across, or two by two (the window's breakpoints switch it).
    pub tiles: gtk::FlowBox,
    /// "Use as Second Display" and "Use as Webcam".
    extend: gtk::ListBox,
    extend_row: adw::ActionRow,
    webcam_row: adw::ActionRow,
    pair_box: gtk::Box,
    staged: adw::Banner,
    drop_hint: gtk::Box,
    drop_label: gtk::Label,
    activity_box: gtk::Box,
    /// One heading and card per day.
    activity: gtk::Box,
    /// Opens a finished incoming row's files.
    on_activate: Rc<dyn Fn(&gtk::ListBoxRow)>,
    empty: gtk::Box,
    handler: Handler,
    /// Received files of finished rows, by entry id (the row's widget name), to open.
    received: Rc<RefCell<HashMap<String, Vec<PathBuf>>>>,
    /// "Camera is on", while this device's camera is in use here, with Preview and Stop.
    camera: gtk::ListBox,
    camera_row: adw::ActionRow,
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

        // Hero: who this is.
        let avatar = adw::Avatar::builder().size(84).show_initials(true).build();
        let kind = gtk::Image::builder().pixel_size(16).css_classes(["hero-kind"]).halign(gtk::Align::End).valign(gtk::Align::End).build();
        let avatar_overlay = gtk::Overlay::builder().child(&avatar).halign(gtk::Align::Center).build();
        avatar_overlay.add_overlay(&kind);
        let name = gtk::Label::builder().css_classes(["title-1"]).wrap(true).justify(gtk::Justification::Center).build();
        let dot = gtk::Box::builder().valign(gtk::Align::Center).css_classes(["status-dot", "inline"]).build();
        let status = gtk::Label::builder().css_classes(["dim-label"]).build();
        let status_line = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).build();
        status_line.append(&dot);
        status_line.append(&status);
        let hero = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["hero"]).build();
        hero.append(&avatar_overlay);
        hero.append(&name);
        hero.append(&status_line);

        // What you can do with it.
        let tiles = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(4)
            .max_children_per_line(4)
            .column_spacing(12)
            .row_spacing(12)
            .halign(gtk::Align::Center)
            .css_classes(["tiles"])
            .build();
        for (icon, label, tip, action) in [
            ("document-send-symbolic", "Send Files", "Choose files to send (Ctrl+O), or drop them anywhere", 0),
            ("edit-paste-symbolic", "Send Clipboard", "Send the text you copied", 1),
            ("preferences-system-notifications-symbolic", "Ping", "Make it show a notification", 2),
            ("video-display-symbolic", "View Screen", "See its screen live, once it allows it", 3),
        ] {
            let b = gtk::Button::builder().css_classes(["action-tile"]).tooltip_text(tip).build();
            let inner = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
            inner.append(&gtk::Image::builder().icon_name(icon).pixel_size(24).build());
            inner.append(&gtk::Label::builder().label(label).build());
            b.set_child(Some(&inner));
            let emit = emit.clone();
            b.connect_clicked(move |_| {
                emit(match action {
                    0 => Action::SendFiles,
                    1 => Action::SendClipboard,
                    2 => Action::Ping,
                    _ => Action::ViewScreen,
                })
            });
            tiles.append(&gtk::FlowBoxChild::builder().child(&b).focusable(false).build());
        }

        // Sidecar, as a settings-style row under the main actions.
        let extend_row = adw::ActionRow::builder().title("Use as Second Display").activatable(true).build();
        extend_row.add_prefix(&gtk::Image::builder().icon_name("video-joined-displays-symbolic").build());
        extend_row.add_suffix(&gtk::Image::builder().icon_name("go-next-symbolic").build());
        let e = emit.clone();
        extend_row.connect_activated(move |_| e(Action::Extend));
        let webcam_row = adw::ActionRow::builder().title("Use as Webcam").activatable(true).build();
        webcam_row.add_prefix(&gtk::Image::builder().icon_name("camera-web-symbolic").build());
        webcam_row.add_suffix(&gtk::Image::builder().icon_name("go-next-symbolic").build());
        let e = emit.clone();
        webcam_row.connect_activated(move |_| e(Action::Webcam));
        let extend = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        extend.append(&extend_row);
        extend.append(&webcam_row);

        let camera_row = adw::ActionRow::builder().title("Camera Is On").css_classes(["camera-on"]).activatable(true).build();
        camera_row.add_prefix(&gtk::Image::builder().icon_name("camera-web-symbolic").css_classes(["accent"]).build());
        let preview = gtk::Button::builder().label("Preview").valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
        let stop = gtk::Button::builder().label("Stop").valign(gtk::Align::Center).build();
        camera_row.add_suffix(&preview);
        camera_row.add_suffix(&stop);
        let e = emit.clone();
        preview.connect_clicked(move |_| e(Action::CameraPreview));
        let e = emit.clone();
        camera_row.connect_activated(move |_| e(Action::CameraPreview));
        let e = emit.clone();
        stop.connect_clicked(move |_| e(Action::CameraStop));
        let camera = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).visible(false).build();
        camera.append(&camera_row);

        // For devices that aren't paired yet.
        let pair = gtk::Button::builder().label("Pair").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).build();
        let e = emit.clone();
        pair.connect_clicked(move |_| e(Action::Pair));
        let pair_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).build();
        pair_box.append(
            &gtk::Label::builder()
                .label("Pair once to share with this device. Both screens will show a code; check they match.")
                .wrap(true)
                .justify(gtk::Justification::Center)
                .css_classes(["dim-label"])
                .build(),
        );
        pair_box.append(&pair);

        // Files opened with Linksy before a device was picked.
        let staged = adw::Banner::builder().button_label("Send").use_markup(false).revealed(false).build();
        let e = emit.clone();
        staged.connect_button_clicked(move |_| e(Action::SendStaged));

        let drop_label = gtk::Label::builder().wrap(true).xalign(0.0).hexpand(true).css_classes(["dim-label"]).build();
        let drop_hint = gtk::Box::builder().spacing(10).halign(gtk::Align::Fill).css_classes(["drop-zone"]).build();
        drop_hint.append(&gtk::Image::builder().icon_name("folder-download-symbolic").pixel_size(20).css_classes(["dim-label"]).build());
        drop_hint.append(&drop_label);

        // Timeline.
        let clear = gtk::Button::builder().label("Clear").css_classes(["flat"]).valign(gtk::Align::Center).build();
        let e = emit.clone();
        clear.connect_clicked(move |_| e(Action::ClearHistory));
        let activity_header = gtk::Box::builder().build();
        activity_header.append(&gtk::Label::builder().label("Activity").xalign(0.0).hexpand(true).css_classes(["heading"]).build());
        activity_header.append(&clear);
        let activity = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
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
        empty.append(&gtk::Label::builder().label("Nothing shared yet").css_classes(["heading", "dim-label"]).build());
        empty.append(&gtk::Label::builder().label("Files, clipboard text and pings you exchange show up here.").wrap(true).justify(gtk::Justification::Center).css_classes(["caption", "dim-label"]).build());
        let activity_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        activity_box.append(&activity_header);
        activity_box.append(&activity);
        activity_box.append(&empty);

        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(22).margin_top(8).margin_bottom(28).margin_start(16).margin_end(16).build();
        content.append(&hero);
        content.append(&tiles);
        content.append(&camera);
        content.append(&extend);
        content.append(&pair_box);
        content.append(&drop_hint);
        content.append(&activity_box);
        let clamp = adw::Clamp::builder().maximum_size(620).child(&content).build();
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        column.append(&staged);
        column.append(&clamp);
        let root = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&column).build();

        // Header menu.
        let forget = gtk::Button::builder().label("Forget Device…").css_classes(["flat", "destructive-label"]).build();
        let popover_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-box"]).build();
        popover_box.append(&forget);
        let popover = gtk::Popover::builder().child(&popover_box).build();
        let menu = gtk::MenuButton::builder().icon_name("view-more-symbolic").popover(&popover).tooltip_text("Device Menu").build();
        let e = emit.clone();
        forget.connect_clicked(move |_| {
            popover.popdown();
            e(Action::Forget);
        });

        Self { root, menu, avatar, kind, name, status, dot, tiles, extend, extend_row, webcam_row, pair_box, staged, drop_hint, drop_label, activity_box, activity, on_activate, empty, handler, received , camera, camera_row }
    }

    pub fn connect_actions(&self, f: impl Fn(Action) + 'static) {
        *self.handler.borrow_mut() = Some(Rc::new(f));
    }

    /// Show the camera card for `camera` ("Linksy Camera (…)"), or hide it.
    pub fn set_camera(&self, camera: Option<&str>) {
        if let Some(name) = camera {
            self.camera_row.set_subtitle(&glib::markup_escape_text(&format!("Click to preview · Your apps list it as “{name}”")));
        }
        self.camera.set_visible(camera.is_some());
    }

    pub fn set_drop_active(&self, on: bool) {
        if on {
            self.drop_hint.add_css_class("drop-active");
        } else {
            self.drop_hint.remove_css_class("drop-active");
        }
    }

    pub fn show_device(&self, d: &DeviceView, staged: &[PathBuf]) {
        let available = d.connected || d.nearby;
        self.avatar.set_text(Some(&d.device.name));
        self.kind.set_icon_name(Some(kind_icon(d.device.kind)));
        self.name.set_label(&d.device.name);
        self.status.set_label(&format!("{} · {}", status_text(d), kind_name(&d.device)));
        if available {
            self.dot.remove_css_class("away");
        } else {
            self.dot.add_css_class("away");
        }
        self.tiles.set_visible(d.trusted);
        self.extend.set_visible(d.trusted);
        self.extend_row.set_subtitle(&glib::markup_escape_text(&format!("Extend this computer's desktop onto {}", d.device.name)));
        self.webcam_row.set_subtitle(&glib::markup_escape_text(&format!("{}'s camera appears in your apps as “Linksy Camera”", d.device.name)));
        self.pair_box.set_visible(!d.trusted);
        self.menu.set_visible(d.trusted);
        self.drop_hint.set_visible(d.trusted);
        self.activity_box.set_visible(d.trusted);
        self.drop_label.set_label(&format!("Drop files anywhere here to send them to {}", d.device.name));

        let ready = d.trusted && !staged.is_empty();
        if ready {
            let (count, bytes) = fmt::measure(staged);
            let what = match staged {
                [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                _ => fmt::plural(count, "item"),
            };
            self.staged.set_title(&format!("{what} ({}) ready to send to {}", fmt::size(bytes), d.device.name));
        }
        self.staged.set_revealed(ready);
    }

    /// Rebuild the timeline. Returns the widgets of running transfers, for progress.
    pub(super) fn show_history(&self, history: &[HistoryEntry], progress: &HashMap<Uuid, Progress>) -> HashMap<Uuid, LiveRow> {
        while let Some(child) = self.activity.first_child() {
            self.activity.remove(&child);
        }
        self.received.borrow_mut().clear();
        let mut live = HashMap::new();
        let mut day_list: Option<(String, gtk::ListBox)> = None;
        for e in history.iter().take(SHOWN) {
            let day = day_label(e.at);
            if day_list.as_ref().is_none_or(|(d, _)| *d != day) {
                let header = gtk::Label::builder().label(&day).xalign(0.0).css_classes(["caption-heading", "dim-label", "day-header"]).build();
                let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
                let open = self.on_activate.clone();
                list.connect_row_activated(move |_, row| open(row));
                self.activity.append(&header);
                self.activity.append(&list);
                day_list = Some((day, list));
            }
            let (row, live_row) = self.entry_row(e, progress.get(&e.id));
            if let Some((_, list)) = &day_list {
                list.append(&row);
            }
            if let Some(l) = live_row {
                live.insert(e.id, l);
            }
        }
        let empty = history.is_empty();
        self.activity.set_visible(!empty);
        self.empty.set_visible(empty);
        live
    }

    fn entry_row(&self, e: &HistoryEntry, progress: Option<&Progress>) -> (gtk::ListBoxRow, Option<LiveRow>) {
        let incoming = e.direction == Direction::Incoming;
        let (icon, title) = match &e.what {
            Shared::Files { files, .. } => (
                if incoming { "folder-download-symbolic" } else { "document-send-symbolic" },
                match files.as_slice() {
                    [one] => fmt::file_name(one),
                    many => fmt::plural(many.len(), "item"),
                },
            ),
            Shared::Clipboard { text } if text.is_empty() => ("edit-paste-symbolic", "Clipboard text".to_string()),
            Shared::Clipboard { text } => ("edit-paste-symbolic", format!("“{}”", one_line(text, 80))),
            Shared::Ping { title, body } if body.is_empty() => ("preferences-system-notifications-symbolic", title.clone()),
            Shared::Ping { title, body } => ("preferences-system-notifications-symbolic", format!("{title}: {body}")),
            Shared::Camera { .. } => ("camera-web-symbolic", if incoming { "Their camera".to_string() } else { "Your camera".to_string() }),
            Shared::Screen { display: true, .. } => ("video-joined-displays-symbolic", if incoming { "Their second display".to_string() } else { "Second display".to_string() }),
            Shared::Screen { .. } => ("video-display-symbolic", if incoming { "Their screen".to_string() } else { "Your screen".to_string() }),
        };
        let size = match &e.what {
            Shared::Files { total, .. } => Some(fmt::size(*total)),
            Shared::Screen { seconds, .. } | Shared::Camera { seconds } if *seconds > 0 => Some(fmt::duration(*seconds)),
            _ => None,
        };
        let verb = match (&e.what, incoming) {
            (Shared::Files { .. }, true) => "Received",
            (Shared::Files { .. }, false) => "Sent",
            (Shared::Camera { .. }, true) => "Used",
            (Shared::Camera { .. }, false) => "Shared",
            (Shared::Screen { .. }, true) => "Watched",
            (Shared::Screen { .. }, false) => "Shared",
            (_, true) => "From them",
            (_, false) => "From you",
        };
        let state = match &e.status {
            HistoryStatus::InProgress => None,
            HistoryStatus::Done => Some(verb.to_string()),
            HistoryStatus::Declined => Some("Declined".into()),
            HistoryStatus::Cancelled => Some("Cancelled".into()),
            HistoryStatus::Failed { reason } => Some(format!("Failed: {reason}")),
        };
        let detail_text = match &state {
            Some(s) => [Some(s.clone()), size, Some(time_label(e.at))].into_iter().flatten().collect::<Vec<_>>().join(" · "),
            None => match progress {
                _ if matches!(e.what, Shared::Screen { .. } | Shared::Camera { .. }) => "Live now".into(),
                Some(p) if p.total > 0 => progress_text(p),
                _ if !incoming => "Waiting for them to accept…".into(),
                _ => "Starting…".into(),
            },
        };

        let badge = gtk::Image::builder().icon_name(icon).pixel_size(18).valign(gtk::Align::Center).css_classes(["activity-icon", if incoming { "incoming" } else { "outgoing" }]).build();
        let title_label = gtk::Label::builder().label(&title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
        let detail = gtk::Label::builder().label(&detail_text).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label", "numeric"]).build();
        let labels = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
        labels.append(&title_label);
        labels.append(&detail);
        let line = gtk::Box::builder().spacing(12).build();
        line.append(&badge);
        line.append(&labels);
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).margin_top(10).margin_bottom(10).margin_start(12).margin_end(12).build();
        body.append(&line);
        let row = gtk::ListBoxRow::builder().child(&body).activatable(false).build();
        if matches!(e.status, HistoryStatus::Failed { .. } | HistoryStatus::Declined | HistoryStatus::Cancelled) {
            row.add_css_class("activity-failed");
        }

        let suffix = |icon: &str, tip: &str, action: Box<dyn Fn() -> Action>| {
            let b = gtk::Button::builder().icon_name(icon).tooltip_text(tip).valign(gtk::Align::Center).css_classes(["flat", "circular"]).build();
            let h = self.handler.clone();
            b.connect_clicked(move |_| {
                let f = h.borrow().clone();
                if let Some(f) = f {
                    f(action());
                }
            });
            line.append(&b);
        };

        let mut live = None;
        match (&e.status, &e.what) {
            (HistoryStatus::InProgress, Shared::Screen { .. } | Shared::Camera { .. }) => {}
            (HistoryStatus::InProgress, _) => {
                let id = e.id;
                suffix("process-stop-symbolic", "Cancel", Box::new(move || Action::Cancel(id)));
                let bar = gtk::ProgressBar::builder().css_classes(["transfer-progress"]).build();
                if let Some(p) = progress.filter(|p| p.total > 0) {
                    bar.set_fraction(p.bytes as f64 / p.total as f64);
                }
                body.append(&bar);
                live = Some(LiveRow { detail, bar });
            }
            (HistoryStatus::Done, Shared::Files { saved, .. }) if incoming && !saved.is_empty() => {
                let first = saved[0].clone();
                suffix("folder-open-symbolic", "Show in Folder", Box::new(move || Action::ShowInFolder(first.clone())));
                row.set_activatable(true);
                row.set_tooltip_text(Some(if saved.len() == 1 { "Open, or drag it anywhere" } else { "Open the folder, or drag the files anywhere" }));
                row.add_controller(drag_files(saved));
                row.set_widget_name(&e.id.to_string());
                self.received.borrow_mut().insert(e.id.to_string(), saved.clone());
            }
            (_, Shared::Clipboard { text }) if !text.is_empty() => {
                let text = text.clone();
                suffix("edit-copy-symbolic", "Copy Again", Box::new(move || Action::Copy(text.clone())));
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
        "Today".into()
    } else if now.add_days(-1).is_ok_and(|y| key(&y) == key(&then)) {
        "Yesterday".into()
    } else {
        then.format("%A %e %B").map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
    }
}

fn time_label(at_ms: u64) -> String {
    local(at_ms).and_then(|t| t.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}
