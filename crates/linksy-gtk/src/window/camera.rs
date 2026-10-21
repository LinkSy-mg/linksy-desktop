//! Controls for a phone's camera used here as a webcam (docs/android-app.md §2.8): under the
//! preview, the phone's own camera app's controls. Zoom chips and lenses, front/back,
//! torch, click-to-focus on the preview, and in a menu exposure, white balance, quality,
//! orientation and stabilization. The phone says what it can do and how it's set in
//! `camera_state`; every change here goes back as a `camera_control`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};

/// Sends one `camera_control` (its keys) to the camera being previewed.
type Send = Rc<dyn Fn(Value)>;

#[derive(Clone)]
pub struct Controls {
    pub root: gtk::Box,
    zooms: gtk::Box,
    flip: gtk::Button,
    torch: gtk::ToggleButton,
    exposure: gtk::Scale,
    white_balance: gtk::DropDown,
    quality: gtk::DropDown,
    portrait: gtk::Switch,
    stabilization: gtk::Switch,
    stabilization_row: adw::ActionRow,
    auto_focus: gtk::Button,
    state: Rc<RefCell<Value>>,
    send: Rc<RefCell<Option<Send>>>,
    /// Set while the controls show a new state, so that showing it sends nothing back.
    updating: Rc<Cell<bool>>,
}

const QUALITIES: [(&str, &str); 3] = [("saver", "Data Saver"), ("standard", "Standard"), ("high", "High")];

impl Controls {
    pub fn new() -> Self {
        let zooms = gtk::Box::builder().css_classes(["linked"]).build();
        let flip = gtk::Button::builder().icon_name("camera-switch-symbolic").tooltip_text("Switch Camera").css_classes(["circular"]).build();
        let torch = gtk::ToggleButton::builder().icon_name("display-brightness-symbolic").tooltip_text("Torch").css_classes(["circular"]).build();
        let auto_focus = gtk::Button::builder().label("Auto Focus").tooltip_text("Click the picture to focus there").css_classes(["pill"]).visible(false).build();

        let exposure = gtk::Scale::with_range(gtk::Orientation::Horizontal, -12.0, 12.0, 1.0);
        exposure.set_hexpand(true);
        exposure.set_draw_value(false);
        exposure.add_mark(0.0, gtk::PositionType::Bottom, None);
        let white_balance = gtk::DropDown::from_strings(&[]);
        white_balance.set_valign(gtk::Align::Center);
        let quality = gtk::DropDown::from_strings(&QUALITIES.map(|(_, label)| label));
        quality.set_valign(gtk::Align::Center);
        let portrait = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let stabilization = gtk::Switch::builder().valign(gtk::Align::Center).build();

        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let row = |title: &str, widget: &gtk::Widget| {
            let r = adw::ActionRow::builder().title(title).build();
            r.add_suffix(widget);
            list.append(&r);
            r
        };
        let exposure_row = adw::ActionRow::builder().title("Exposure").build();
        exposure.set_size_request(160, -1);
        exposure_row.add_suffix(&exposure);
        list.append(&exposure_row);
        row("White Balance", white_balance.upcast_ref());
        row("Quality", quality.upcast_ref());
        row("Portrait", portrait.upcast_ref());
        let stabilization_row = row("Stabilization", stabilization.upcast_ref());
        let popover = gtk::Popover::builder().child(&list).build();
        list.set_width_request(320);
        let more = gtk::MenuButton::builder().icon_name("emblem-system-symbolic").tooltip_text("Camera Settings").popover(&popover).css_classes(["circular"]).build();

        let root = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).css_classes(["toolbar"]).build();
        root.append(&flip);
        root.append(&zooms);
        root.append(&torch);
        root.append(&auto_focus);
        root.append(&more);
        root.set_visible(false);

        let c = Controls {
            root,
            zooms,
            flip,
            torch,
            exposure,
            white_balance,
            quality,
            portrait,
            stabilization,
            stabilization_row,
            auto_focus,
            state: Rc::new(RefCell::new(Value::Null)),
            send: Rc::default(),
            updating: Rc::default(),
        };
        let s = c.clone();
        c.flip.connect_clicked(move |_| {
            let back = s.state.borrow()["facing"] != "front";
            s.emit(json!({ "facing": if back { "front" } else { "back" } }));
        });
        let s = c.clone();
        c.torch.connect_toggled(move |b| s.emit(json!({ "torch": b.is_active() })));
        let s = c.clone();
        c.auto_focus.connect_clicked(move |_| s.emit(json!({ "focus": "auto" })));
        let s = c.clone();
        c.exposure.connect_value_changed(move |r| s.emit(json!({ "exposure": r.value().round() as i64 })));
        let s = c.clone();
        c.white_balance.connect_selected_notify(move |d| {
            let list = s.state.borrow()["white_balances"].as_array().cloned().unwrap_or_default();
            if let Some(wb) = list.get(d.selected() as usize) {
                s.emit(json!({ "white_balance": wb }));
            }
        });
        let s = c.clone();
        c.quality.connect_selected_notify(move |d| {
            if let Some((q, _)) = QUALITIES.get(d.selected() as usize) {
                s.emit(json!({ "quality": q }));
            }
        });
        let s = c.clone();
        c.portrait.connect_active_notify(move |w| s.emit(json!({ "orientation": if w.is_active() { "portrait" } else { "landscape" } })));
        let s = c.clone();
        c.stabilization.connect_active_notify(move |w| s.emit(json!({ "stabilization": w.is_active() })));
        c
    }

    /// Control the camera previewed now through `send`; hidden until it says what it has.
    pub fn attach(&self, send: Send) {
        *self.send.borrow_mut() = Some(send);
        *self.state.borrow_mut() = Value::Null;
        self.root.set_visible(false);
    }

    pub fn detach(&self) {
        *self.send.borrow_mut() = None;
        self.root.set_visible(false);
    }

    /// A click on the preview at `(x, y)`, as fractions of the picture as streamed.
    pub fn focus_at(&self, x: f64, y: f64) {
        if self.state.borrow().get("focus_modes").and_then(Value::as_array).is_some_and(|m| m.iter().any(|v| v == "point")) {
            self.emit(json!({ "focus": "point", "focus_point": [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)] }));
        }
    }

    fn emit(&self, control: Value) {
        if self.updating.get() {
            return;
        }
        if let Some(send) = self.send.borrow().as_ref() {
            send(control);
        }
    }

    /// Show what the camera can do and how it's set.
    pub fn show(&self, state: Value) {
        if self.send.borrow().is_none() || !state.is_object() {
            return;
        }
        self.updating.set(true);
        *self.state.borrow_mut() = state.clone();
        let num = |k: &str| state.get(k).and_then(Value::as_f64);
        let flag = |k: &str| state.get(k).and_then(Value::as_bool).unwrap_or(false);
        let text = |k: &str| state.get(k).and_then(Value::as_str).unwrap_or_default().to_string();

        // Zoom chips: the phone's presets, the current zoom checked.
        while let Some(child) = self.zooms.first_child() {
            self.zooms.remove(&child);
        }
        let zoom = num("zoom").unwrap_or(1.0);
        let presets: Vec<f64> = state.get("zoom_presets").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        let mut group: Option<gtk::ToggleButton> = None;
        for z in presets {
            let label = if z.fract() == 0.0 { format!("{z:.0}×") } else { format!("{z}×") };
            let b = gtk::ToggleButton::builder().label(&label).active((z - zoom).abs() < 0.05).build();
            b.set_group(group.as_ref());
            group.get_or_insert_with(|| b.clone());
            let s = self.clone();
            b.connect_toggled(move |b| {
                if b.is_active() {
                    s.emit(json!({ "zoom": z }));
                }
            });
            self.zooms.append(&b);
        }
        self.zooms.set_visible(self.zooms.first_child().is_some());

        let lenses = state.get("lenses").and_then(Value::as_array).cloned().unwrap_or_default();
        self.flip.set_visible(lenses.iter().any(|l| l["facing"] == "front") && lenses.iter().any(|l| l["facing"] == "back"));
        self.torch.set_visible(flag("has_torch"));
        self.torch.set_active(flag("torch"));
        self.auto_focus.set_visible(text("focus") == "point" || text("focus") == "manual");

        let (min, max) = (num("exposure_min").unwrap_or(0.0), num("exposure_max").unwrap_or(0.0));
        self.exposure.set_range(min, max.max(min + 1.0));
        self.exposure.set_value(num("exposure").unwrap_or(0.0));
        self.exposure.set_sensitive(max > min);

        let wbs: Vec<String> = state.get("white_balances").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(title)).collect()).unwrap_or_default();
        let wb_list = gtk::StringList::new(&wbs.iter().map(String::as_str).collect::<Vec<_>>());
        self.white_balance.set_model(Some(&wb_list));
        let current = state.get("white_balances").and_then(Value::as_array).and_then(|a| a.iter().position(|v| *v == state["white_balance"]));
        self.white_balance.set_selected(current.unwrap_or(0) as u32);
        self.white_balance.set_sensitive(wbs.len() > 1);

        let q = QUALITIES.iter().position(|(k, _)| *k == text("quality")).unwrap_or(1);
        self.quality.set_selected(q as u32);
        self.portrait.set_active(text("orientation") == "portrait");
        self.stabilization_row.set_visible(flag("has_stabilization"));
        self.stabilization.set_active(flag("stabilization"));

        self.root.set_visible(true);
        self.updating.set(false);
    }
}

/// "daylight" → "Daylight".
fn title(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Where a click on a picture shown with `ContentFit::Cover` lands on the picture itself,
/// as fractions: the picture is scaled to cover the widget and centred, edges cropped.
pub fn cover_point(widget: (f64, f64), picture: (f64, f64), at: (f64, f64)) -> (f64, f64) {
    let scale = (widget.0 / picture.0).max(widget.1 / picture.1);
    let (w, h) = (picture.0 * scale, picture.1 * scale);
    (((at.0 - (widget.0 - w) / 2.0) / w), ((at.1 - (widget.1 - h) / 2.0) / h))
}
