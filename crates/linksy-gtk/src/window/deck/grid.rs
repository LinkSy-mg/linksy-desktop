//! The phone's grid in the editor: tiles placed by cell, the picked tile's delete button and
//! resize handles, and what a drag shows (where a tile lands, the sizes a resize snaps to).
//! It says what the user did; the editor changes the layout and hands the tiles back.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use linksy_i18n::t;
use linksy_ipc::linksy_core::{DeckFace, DeckTile};

use super::art::Art;
use super::model::{self, Edge, Metrics, Theme};

pub enum Event {
    Pick(Option<String>),
    Move(String, u8, u8),
    Resize(String, u8, u8),
    /// Arrow keys: by a cell.
    Nudge(String, i8, i8),
    Delete(String),
    /// A widget from the library, dropped with its corner on this cell.
    Drop(String, u8, u8),
}

/// A place in the grid: x, y, width, height, in cells.
type Cells = (u8, u8, u8, u8);
type Handler = Rc<dyn Fn(Event)>;

struct Slot {
    tile: DeckTile,
    widget: gtk::Widget,
    /// Where it was last put, and where it moves from while it animates.
    rect: graphene::Rect,
    from: Option<graphene::Rect>,
}

#[derive(Default)]
enum Gesture {
    #[default]
    Idle,
    /// Pressed, not dragged far enough yet.
    Pressed(String, Option<Edge>),
    Move { id: String, dx: f64, dy: f64, to: (u8, u8), ok: bool },
    Resize { id: String, edge: Edge, dx: f64, dy: f64, options: Vec<(u8, u8)>, best: (u8, u8) },
}

mod imp {
    use super::*;

    pub struct Grid {
        pub(super) slots: RefCell<Vec<Slot>>,
        /// Deleted tiles while they shrink away, where they were.
        pub(super) dying: RefCell<Vec<(gtk::Widget, graphene::Rect)>>,
        pub(super) face: RefCell<DeckFace>,
        pub(super) theme: Cell<Theme>,
        pub(super) metrics: Cell<Metrics>,
        pub(super) origin: Cell<(f64, f64)>,
        pub(super) picked: RefCell<Option<String>>,
        pub(super) gesture: RefCell<Gesture>,
        /// A library drag over the grid: the size it brings, and where it would land.
        pub(super) incoming: Cell<Option<(u8, u8)>>,
        pub(super) landing: Cell<Option<(Cells, bool)>>,
        /// The picked tile's delete button (no edge) and handles.
        pub(super) overlays: RefCell<Vec<(Option<Edge>, gtk::Widget)>>,
        pub(super) badge: RefCell<Option<gtk::Label>>,
        pub(super) progress: Cell<f64>,
        pub(super) animation: RefCell<Option<adw::TimedAnimation>>,
        pub(super) handler: RefCell<Option<Handler>>,
    }

    impl Default for Grid {
        fn default() -> Self {
            Self {
                slots: RefCell::default(),
                dying: RefCell::default(),
                face: RefCell::new(DeckFace::portrait()),
                theme: Cell::default(),
                metrics: Cell::default(),
                origin: Cell::default(),
                picked: RefCell::default(),
                gesture: RefCell::default(),
                incoming: Cell::default(),
                landing: Cell::default(),
                overlays: RefCell::default(),
                badge: RefCell::default(),
                progress: Cell::new(1.0),
                animation: RefCell::default(),
                handler: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Grid {
        const NAME: &'static str = "LinksyDeckGrid";
        type Type = super::Grid;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Grid {
        fn constructed(&self) {
            self.parent_constructed();
            let grid = self.obj();
            grid.set_overflow(gtk::Overflow::Visible);
            grid.add_css_class("deck-grid");
            grid.setup_drag();
            grid.setup_keys();
            grid.setup_drop();
        }

        fn dispose(&self) {
            if let Some(a) = self.animation.take() {
                a.pause();
            }
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Grid {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let [cols, rows] = self.face.borrow().grid;
            let n = if orientation == gtk::Orientation::Horizontal { cols } else { rows } as i32;
            (n * 12, n * 58 + (n - 1) * 8, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let grid = self.face.borrow().grid;
            let m = Metrics::fit(width as f64, height as f64, grid);
            let (w, h) = (m.span(grid[0]), m.span(grid[1]));
            self.metrics.set(m);
            self.origin.set((((width as f64 - w) / 2.0).floor(), ((height as f64 - h) / 2.0).floor()));
            let obj = self.obj();
            for (widget, r) in self.dying.borrow().iter() {
                put(widget, *r);
            }
            let mut picked_rect = None;
            for slot in self.slots.borrow_mut().iter_mut() {
                let r = obj.shown(slot);
                put(&slot.widget, r);
                slot.rect = r;
                if self.picked.borrow().as_deref() == Some(slot.tile.id.as_str()) {
                    picked_rect = Some(r);
                }
            }
            if let Some(r) = picked_rect {
                for (edge, widget) in self.overlays.borrow().iter() {
                    let (x, y, w, h) = (r.x(), r.y(), r.width(), r.height());
                    let at = match edge {
                        None => rect(x + w - 12.0, y - 10.0, 22.0, 22.0),
                        Some(Edge::Right) => rect(x + w - 3.0, y + h / 2.0 - 12.0, 8.0, 24.0),
                        Some(Edge::Bottom) => rect(x + w / 2.0 - 12.0, y + h - 3.0, 24.0, 8.0),
                        Some(Edge::Corner) => rect(x + w - 7.0, y + h - 7.0, 14.0, 14.0),
                    };
                    put(widget, at);
                }
                if let Some(badge) = self.badge.borrow().as_ref() {
                    let (bw, bh) = (badge.measure(gtk::Orientation::Horizontal, -1).1, badge.measure(gtk::Orientation::Vertical, -1).1);
                    let cell = obj.cell_rect(&self.cell_of(&r));
                    put(badge.upcast_ref(), rect(cell.x(), cell.y() - bh as f32 - 6.0, bw as f32, bh as f32));
                }
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let theme = self.theme.get();
            let [cols, rows] = self.face.borrow().grid;
            let (light, radius) = (theme != Theme::Eink, tile_radius(theme));

            let cells = gsk::PathBuilder::new();
            for y in 0..rows {
                for x in 0..cols {
                    let r = obj.cell_rect(&(x, y, 1, 1));
                    cells.add_rounded_rect(&gsk::RoundedRect::from_rect(rect(r.x() + 0.5, r.y() + 0.5, r.width() - 1.0, r.height() - 1.0), cell_radius(theme)));
                }
            }
            let dashed = gsk::Stroke::new(1.0);
            dashed.set_dash(&[3.0, 3.0]);
            snapshot.append_stroke(&cells.to_path(), &dashed, &cell_line(theme));

            let ink = |alpha: f32| if light { gdk::RGBA::new(1.0, 1.0, 1.0, alpha) } else { gdk::RGBA::new(0.07, 0.07, 0.07, alpha) };
            let red = |alpha: f32| gdk::RGBA::new(1.0, 0.42, 0.42, alpha);
            let mut lifted = None;
            match &*self.gesture.borrow() {
                Gesture::Move { id, to, ok, .. } => {
                    let tile = self.slots.borrow().iter().find(|s| s.tile.id == *id).map(|s| s.tile.clone());
                    if let Some(t) = tile {
                        outline(snapshot, obj.cell_rect(&(to.0, to.1, t.w, t.h)), radius, 2.0, Some(&[6.0, 4.0]), if *ok { ink(0.6) } else { red(1.0) }, Some(if *ok { ink(0.06) } else { red(0.12) }));
                    }
                    lifted = Some(id.clone());
                }
                Gesture::Resize { id, options, best, .. } => {
                    if let Some(t) = self.slots.borrow().iter().find(|s| s.tile.id == *id).map(|s| s.tile.clone()) {
                        for &(w, h) in options {
                            let r = obj.cell_rect(&(t.x, t.y, w, h));
                            if (w, h) == *best {
                                outline(snapshot, r, radius, 2.0, None, ink(1.0), Some(ink(0.08)));
                            } else {
                                outline(snapshot, r, radius, 1.5, Some(&[5.0, 4.0]), ink(if light { 0.25 } else { 0.4 }), None);
                            }
                        }
                    }
                    lifted = Some(id.clone());
                }
                _ => {}
            }
            if let Some((cell, ok)) = self.landing.get() {
                outline(snapshot, obj.cell_rect(&cell), radius, 2.0, Some(&[6.0, 4.0]), if ok { ink(0.6) } else { red(1.0) }, Some(if ok { ink(0.06) } else { red(0.12) }));
            }

            for (widget, _) in self.dying.borrow().iter() {
                obj.snapshot_child(widget, snapshot);
            }
            let slots = self.slots.borrow();
            for slot in slots.iter().filter(|s| Some(&s.tile.id) != lifted.as_ref()) {
                obj.snapshot_child(&slot.widget, snapshot);
            }
            for slot in slots.iter().filter(|s| Some(&s.tile.id) == lifted.as_ref()) {
                obj.snapshot_child(&slot.widget, snapshot);
            }
            for (_, widget) in self.overlays.borrow().iter() {
                obj.snapshot_child(widget, snapshot);
            }
            if let Some(badge) = self.badge.borrow().as_ref() {
                obj.snapshot_child(badge, snapshot);
            }
        }
    }

    impl Grid {
        /// The cells a pixel rectangle stands on, rounded.
        pub(super) fn cell_of(&self, r: &graphene::Rect) -> (u8, u8, u8, u8) {
            let (m, (ox, oy)) = (self.metrics.get(), self.origin.get());
            let n = |p: f32, o: f64| ((p as f64 - o) / m.step()).round().max(0.0) as u8;
            (n(r.x(), ox), n(r.y(), oy), 1, 1)
        }
    }
}

glib::wrapper! {
    pub struct Grid(ObjectSubclass<imp::Grid>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> graphene::Rect {
    graphene::Rect::new(x, y, w, h)
}

fn put(widget: &gtk::Widget, r: graphene::Rect) {
    let _ = widget.measure(gtk::Orientation::Horizontal, -1);
    let transform = gsk::Transform::new().translate(&graphene::Point::new(r.x(), r.y()));
    widget.allocate(r.width().round() as i32, r.height().round() as i32, -1, Some(transform));
}

pub fn tile_radius(theme: Theme) -> f32 {
    match theme {
        Theme::Stock => 14.0,
        Theme::Dots => 18.0,
        Theme::Eink => 10.0,
        Theme::Term => 4.0,
        Theme::Pixel => 0.0,
    }
}

fn cell_radius(theme: Theme) -> f32 {
    match theme {
        Theme::Pixel => 0.0,
        Theme::Term => 4.0,
        _ => 12.0,
    }
}

fn cell_line(theme: Theme) -> gdk::RGBA {
    match theme {
        Theme::Eink => gdk::RGBA::new(0.0, 0.0, 0.0, 0.14),
        Theme::Term => gdk::RGBA::new(0.22, 1.0, 0.48, 0.12),
        Theme::Dots => gdk::RGBA::new(1.0, 1.0, 1.0, 0.07),
        _ => gdk::RGBA::new(1.0, 1.0, 1.0, 0.07),
    }
}

fn outline(snapshot: &gtk::Snapshot, r: graphene::Rect, radius: f32, width: f32, dash: Option<&[f32]>, line: gdk::RGBA, fill: Option<gdk::RGBA>) {
    let inset = width / 2.0;
    let shape = gsk::RoundedRect::from_rect(rect(r.x() + inset, r.y() + inset, r.width() - width, r.height() - width), (radius - inset).max(0.0));
    let path = gsk::PathBuilder::new();
    path.add_rounded_rect(&shape);
    let path = path.to_path();
    if let Some(fill) = fill {
        snapshot.append_fill(&path, gsk::FillRule::Winding, &fill);
    }
    let stroke = gsk::Stroke::new(width);
    if let Some(dash) = dash {
        stroke.set_dash(dash);
    }
    snapshot.append_stroke(&path, &stroke, &line);
}

fn lerp(a: &graphene::Rect, b: &graphene::Rect, t: f32) -> graphene::Rect {
    let l = |a: f32, b: f32| a + (b - a) * t;
    rect(l(a.x(), b.x()), l(a.y(), b.y()), l(a.width(), b.width()), l(a.height(), b.height()))
}

fn animations(widget: &impl IsA<gtk::Widget>) -> bool {
    widget.settings().is_gtk_enable_animations()
}

impl Default for Grid {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Grid {
    pub fn connect(&self, handler: impl Fn(Event) + 'static) {
        *self.imp().handler.borrow_mut() = Some(Rc::new(handler));
    }

    fn emit(&self, event: Event) {
        let handler = self.imp().handler.borrow().clone();
        if let Some(h) = handler {
            h(event);
        }
    }

    pub fn metrics(&self) -> Metrics {
        self.imp().metrics.get()
    }

    /// Cells as pixels, from the grid's corner.
    fn cell_rect(&self, &(x, y, w, h): &(u8, u8, u8, u8)) -> graphene::Rect {
        let (m, (ox, oy)) = (self.metrics(), self.imp().origin.get());
        rect((ox + m.at(x)) as f32, (oy + m.at(y)) as f32, m.span(w) as f32, m.span(h) as f32)
    }

    /// Where a slot shows now: its cells, on its way there, and following a drag.
    fn shown(&self, slot: &Slot) -> graphene::Rect {
        let imp = self.imp();
        let t = &slot.tile;
        let target = self.cell_rect(&(t.x, t.y, t.w, t.h));
        let mut r = match &slot.from {
            Some(from) => lerp(from, &target, imp.progress.get() as f32),
            None => target,
        };
        match &*imp.gesture.borrow() {
            Gesture::Move { id, dx, dy, .. } if *id == t.id => r = rect(r.x() + *dx as f32, r.y() + *dy as f32, r.width(), r.height()),
            Gesture::Resize { id, edge, dx, dy, .. } if *id == t.id => {
                let cell = imp.metrics.get().cell as f32;
                let w = if *edge == Edge::Bottom { target.width() } else { (target.width() + *dx as f32).max(cell) };
                let h = if *edge == Edge::Right { target.height() } else { (target.height() + *dy as f32).max(cell) };
                r = rect(target.x(), target.y(), w, h);
            }
            _ => {}
        }
        r
    }

    /// The tiles of the page shown, as `build` draws them. With `animate`, new ones pop in,
    /// gone ones shrink away and moved ones slide; otherwise (another page, layout or theme)
    /// everything is simply there.
    pub fn sync(&self, grid: [u8; 2], tiles: &[DeckTile], theme: Theme, animate: bool, build: &dyn Fn(&DeckTile) -> gtk::Widget) {
        let imp = self.imp();
        let animate = animate && animations(self) && imp.face.borrow().grid == grid && imp.theme.get() == theme;
        let fresh = imp.face.borrow().grid != grid || imp.theme.get() != theme;
        imp.theme.set(theme);
        *imp.face.borrow_mut() = DeckFace { grid, pages: vec![tiles.to_vec()] };

        let mut moved = Vec::new();
        let mut gone = Vec::new();
        {
            let mut slots = imp.slots.borrow_mut();
            slots.retain(|s| {
                let keep = !fresh && tiles.iter().any(|t| t.id == s.tile.id);
                if !keep {
                    gone.push((s.widget.clone(), s.rect));
                }
                keep
            });
            for slot in slots.iter_mut() {
                let t = tiles.iter().find(|t| t.id == slot.tile.id).expect("kept");
                let same_face = DeckTile { x: t.x, y: t.y, ..slot.tile.clone() } == *t;
                if (t.x, t.y, t.w, t.h) != (slot.tile.x, slot.tile.y, slot.tile.w, slot.tile.h) {
                    moved.push(t.id.clone());
                }
                if !same_face {
                    let widget = build(t);
                    self.adopt(&widget, &t.id);
                    widget.insert_after(self, Some(&slot.widget));
                    slot.widget.unparent();
                    slot.widget = widget;
                }
                slot.tile = t.clone();
            }
        }
        for t in tiles {
            if imp.slots.borrow().iter().any(|s| s.tile.id == t.id) {
                continue;
            }
            let widget = build(t);
            self.adopt(&widget, &t.id);
            let before = imp.overlays.borrow().first().map(|(_, w)| w.clone());
            widget.insert_before(self, before.as_ref());
            if animate {
                widget.add_css_class("pop");
                let w = widget.downgrade();
                glib::timeout_add_local_once(Duration::from_millis(320), move || {
                    if let Some(w) = w.upgrade() {
                        w.remove_css_class("pop");
                    }
                });
            }
            imp.slots.borrow_mut().push(Slot { tile: t.clone(), widget, rect: graphene::Rect::zero(), from: None });
        }
        for (widget, r) in gone {
            if animate {
                // Shrinks away, then it's gone
                widget.add_css_class("gone");
                widget.set_can_target(false);
                widget.set_focusable(false);
                imp.dying.borrow_mut().push((widget.clone(), r));
                let grid = self.downgrade();
                glib::timeout_add_local_once(Duration::from_millis(170), move || {
                    if let Some(grid) = grid.upgrade() {
                        grid.imp().dying.borrow_mut().retain(|(w, _)| *w != widget);
                        widget.unparent();
                        grid.queue_draw();
                    }
                });
            } else {
                widget.unparent();
            }
        }
        if fresh {
            for (w, _) in imp.dying.take() {
                w.unparent();
            }
        }
        if fresh {
            for s in imp.slots.borrow_mut().iter_mut() {
                s.from = None;
            }
        }
        if animate && !moved.is_empty() {
            self.slide(&moved);
        }
        self.sync_picked();
        self.queue_allocate();
        self.queue_draw();
    }

    /// Faces built again for the tiles `which` says (the clock's, each minute).
    pub fn rebuild(&self, which: impl Fn(&DeckTile) -> bool, build: &dyn Fn(&DeckTile) -> gtk::Widget) {
        let mut slots = self.imp().slots.borrow_mut();
        for slot in slots.iter_mut().filter(|s| which(&s.tile)) {
            let widget = build(&slot.tile);
            self.adopt(&widget, &slot.tile.id);
            widget.insert_after(self, Some(&slot.widget));
            slot.widget.unparent();
            slot.widget = widget;
        }
        drop(slots);
        self.sync_picked();
        self.queue_allocate();
    }

    /// A tile's widget, taken in: focus picks it, and it says what it is.
    fn adopt(&self, widget: &gtk::Widget, id: &str) {
        widget.set_focusable(true);
        widget.set_cursor_from_name(Some("grab"));
        let focus = gtk::EventControllerFocus::new();
        let (grid, id) = (self.downgrade(), id.to_string());
        focus.connect_enter(move |_| {
            if let Some(grid) = grid.upgrade()
                && grid.imp().picked.borrow().as_deref() != Some(id.as_str())
            {
                grid.emit(Event::Pick(Some(id.clone())));
            }
        });
        widget.add_controller(focus);
    }

    pub fn tile_widget(&self, id: &str) -> Option<gtk::Widget> {
        self.imp().slots.borrow().iter().find(|s| s.tile.id == id).map(|s| s.widget.clone())
    }

    /// Slides `ids` from where they show to their cells; those already moving go on from
    /// where they are.
    fn slide(&self, ids: &[String]) {
        let imp = self.imp();
        if let Some(a) = imp.animation.take() {
            a.pause();
        }
        for slot in imp.slots.borrow_mut().iter_mut() {
            if (ids.contains(&slot.tile.id) || slot.from.is_some()) && slot.rect.width() > 0.0 {
                slot.from = Some(slot.rect);
            }
        }
        imp.progress.set(0.0);
        let grid = self.downgrade();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            if let Some(grid) = grid.upgrade() {
                grid.imp().progress.set(v);
                grid.queue_allocate();
            }
        });
        let animation = adw::TimedAnimation::builder().widget(self).value_from(0.0).value_to(1.0).duration(140).easing(adw::Easing::EaseOutCubic).target(&target).build();
        let grid = self.downgrade();
        animation.connect_done(move |a| {
            let Some(grid) = grid.upgrade() else { return };
            if grid.imp().animation.borrow().as_ref() == Some(a) {
                for slot in grid.imp().slots.borrow_mut().iter_mut() {
                    slot.from = None;
                }
                grid.queue_allocate();
            }
        });
        *imp.animation.borrow_mut() = Some(animation.clone());
        animation.play();
    }

    /// Picks a tile (or none): its outline, its delete button and handles.
    pub fn set_picked(&self, id: Option<&str>) {
        let imp = self.imp();
        if imp.picked.borrow().as_deref() == id && !imp.overlays.borrow().is_empty() == id.is_some() {
            return;
        }
        *imp.picked.borrow_mut() = id.map(String::from);
        for (_, w) in imp.overlays.take() {
            w.unparent();
        }
        let tile = id.and_then(|id| imp.slots.borrow().iter().find(|s| s.tile.id == id).map(|s| s.tile.clone()));
        if let Some(t) = tile {
            let delete = gtk::Button::builder().css_classes(["deck-del"]).focusable(false).tooltip_text(t!("Delete")).build();
            let x = Art::icon("x", 12, Theme::Stock);
            x.set_stroke(3.2);
            delete.set_child(Some(&x));
            delete.set_cursor_from_name(Some("pointer"));
            let (grid, tid) = (self.downgrade(), t.id.clone());
            delete.connect_clicked(move |_| {
                if let Some(grid) = grid.upgrade() {
                    grid.emit(Event::Delete(tid.clone()));
                }
            });
            let mut overlays: Vec<(Option<Edge>, gtk::Widget)> = vec![(None, delete.upcast())];
            if model::sizes(&t.kind, imp.face.borrow().grid).len() > 1 {
                for (edge, class, cursor) in [(Edge::Right, "right", "ew-resize"), (Edge::Bottom, "bottom", "ns-resize"), (Edge::Corner, "corner", "nwse-resize")] {
                    let handle = gtk::Box::builder().css_classes(["deck-handle", class]).build();
                    handle.set_cursor_from_name(Some(cursor));
                    handle.set_tooltip_text(Some(t!("Resize")));
                    overlays.push((Some(edge), handle.upcast()));
                }
            }
            for (_, w) in &overlays {
                w.set_parent(self);
            }
            *imp.overlays.borrow_mut() = overlays;
        }
        self.sync_picked();
        self.queue_allocate();
    }

    fn sync_picked(&self) {
        let imp = self.imp();
        let picked = imp.picked.borrow().clone();
        if !imp.slots.borrow().iter().any(|s| picked.as_deref() == Some(s.tile.id.as_str())) {
            for (_, w) in imp.overlays.take() {
                w.unparent();
            }
        }
        for slot in imp.slots.borrow().iter() {
            if picked.as_deref() == Some(slot.tile.id.as_str()) {
                slot.widget.add_css_class("picked");
            } else {
                slot.widget.remove_css_class("picked");
            }
        }
        if let Some(badge) = imp.badge.borrow().as_ref() {
            badge.insert_before(self, gtk::Widget::NONE);
        }
    }

    /// The widget `x`,`y` is on, among the grid's own: a tile, a handle or the delete button.
    fn target_at(&self, x: f64, y: f64) -> Option<gtk::Widget> {
        let mut w = self.pick(x, y, gtk::PickFlags::DEFAULT)?;
        while w.parent().as_ref() != Some(self.upcast_ref()) {
            w = w.parent()?;
        }
        Some(w)
    }

    fn setup_drag(&self) {
        let drag = gtk::GestureDrag::new();
        let grid = self.downgrade();
        drag.connect_drag_begin(move |g, x, y| {
            let Some(grid) = grid.upgrade() else { return };
            let imp = grid.imp();
            let target = grid.target_at(x, y);
            let overlay = target.as_ref().and_then(|t| imp.overlays.borrow().iter().find(|(_, w)| w == t).map(|(e, _)| *e));
            let picked = imp.picked.borrow().clone();
            match (overlay, picked) {
                (Some(None), _) | (Some(Some(_)), None) => {
                    g.set_state(gtk::EventSequenceState::Denied);
                }
                (Some(Some(edge)), Some(id)) => {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    *imp.gesture.borrow_mut() = Gesture::Pressed(id, Some(edge));
                }
                (None, _) => {
                    let id = target.as_ref().and_then(|t| imp.slots.borrow().iter().find(|s| s.widget == *t).map(|s| s.tile.id.clone()));
                    match id {
                        Some(id) => {
                            if let Some(t) = &target {
                                t.grab_focus();
                            }
                            grid.emit(Event::Pick(Some(id.clone())));
                            *imp.gesture.borrow_mut() = Gesture::Pressed(id, None);
                        }
                        None => grid.emit(Event::Pick(None)),
                    }
                }
            }
        });
        let grid = self.downgrade();
        drag.connect_drag_update(move |g, dx, dy| {
            if let Some(grid) = grid.upgrade() {
                grid.dragged(g, dx, dy);
            }
        });
        let grid = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(grid) = grid.upgrade() {
                grid.dropped();
            }
        });
        let grid = self.downgrade();
        drag.connect_cancel(move |_, _| {
            if let Some(grid) = grid.upgrade() {
                grid.cancel_gesture();
            }
        });
        self.add_controller(drag);
    }

    fn dragged(&self, g: &gtk::GestureDrag, dx: f64, dy: f64) {
        let imp = self.imp();
        let m = imp.metrics.get();
        let grid = imp.face.borrow().grid;
        let tile_of = |id: &str| imp.slots.borrow().iter().find(|s| s.tile.id == id).map(|s| (s.tile.clone(), s.widget.clone()));
        let started = match &*imp.gesture.borrow() {
            Gesture::Pressed(id, edge) if dx.abs() + dy.abs() >= 4.0 => Some((id.clone(), *edge)),
            _ => None,
        };
        if let Some((id, edge)) = started {
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some((t, widget)) = tile_of(&id) else { return };
            *imp.gesture.borrow_mut() = match edge {
                None => {
                    widget.add_css_class("lifted");
                    widget.set_cursor_from_name(Some("grabbing"));
                    Gesture::Move { id, dx, dy, to: (t.x, t.y), ok: true }
                }
                Some(edge) => {
                    // The sizes it takes that fit where it is, as outlines from its corner
                    let face = imp.face.borrow();
                    let mut options: Vec<(u8, u8)> = model::sizes(&t.kind, grid).into_iter().filter(|&(w, h)| face.fits(0, t.x, t.y, w, h, Some(&t.id))).collect();
                    if options.is_empty() {
                        options.push((t.w, t.h));
                    }
                    let badge = gtk::Label::builder().css_classes(["deck-size-badge"]).can_target(false).build();
                    badge.set_parent(self);
                    *imp.badge.borrow_mut() = Some(badge);
                    Gesture::Resize { id, edge, dx, dy, options, best: (t.w, t.h) }
                }
            };
        }
        let mut blocked = None;
        let mut size = None;
        match &mut *imp.gesture.borrow_mut() {
            Gesture::Move { id, dx: mx, dy: my, to, ok } => {
                let Some((t, widget)) = tile_of(id) else { return };
                (*mx, *my) = (dx, dy);
                *to = model::moved_cell(m, grid, (t.x, t.y), (t.w, t.h), dx, dy);
                *ok = imp.face.borrow().fits(0, to.0, to.1, t.w, t.h, Some(&t.id));
                blocked = Some((widget, !*ok));
            }
            Gesture::Resize { id, edge, dx: rx, dy: ry, options, best } => {
                let Some((t, _)) = tile_of(id) else { return };
                (*rx, *ry) = (dx, dy);
                if let Some(b) = model::nearest(options, model::wanted((t.w, t.h), *edge, dx, dy, m)) {
                    *best = b;
                }
                size = Some(*best);
            }
            _ => return,
        }
        if let Some((widget, blocked)) = blocked {
            if blocked {
                widget.add_css_class("blocked");
            } else {
                widget.remove_css_class("blocked");
            }
        }
        if let (Some((w, h)), Some(badge)) = (size, imp.badge.borrow().as_ref()) {
            badge.set_label(&format!("{w} × {h}"));
        }
        self.queue_allocate();
        self.queue_draw();
    }

    /// Let go: a move lands where it showed or goes back; a resize takes the lit size.
    fn dropped(&self) {
        let imp = self.imp();
        let gesture = imp.gesture.take();
        if let Some(badge) = imp.badge.take() {
            badge.unparent();
        }
        let (id, event) = match gesture {
            Gesture::Move { id, to, ok, .. } => {
                let tile = imp.slots.borrow().iter().find(|s| s.tile.id == id).map(|s| (s.tile.clone(), s.widget.clone()));
                let Some((t, widget)) = tile else { return };
                widget.remove_css_class("lifted");
                widget.remove_css_class("blocked");
                widget.set_cursor_from_name(Some("grab"));
                let event = (ok && to != (t.x, t.y)).then(|| Event::Move(id.clone(), to.0, to.1));
                (id, event)
            }
            Gesture::Resize { id, best, .. } => {
                let same = imp.slots.borrow().iter().any(|s| s.tile.id == id && (s.tile.w, s.tile.h) == best);
                let event = (!same).then(|| Event::Resize(id.clone(), best.0, best.1));
                (id, event)
            }
            _ => return,
        };
        self.slide(&[id]);
        if let Some(event) = event {
            self.emit(event);
        }
        self.queue_draw();
    }

    fn cancel_gesture(&self) {
        let imp = self.imp();
        if matches!(&*imp.gesture.borrow(), Gesture::Move { .. } | Gesture::Resize { .. }) {
            if let Gesture::Move { id, .. } = &*imp.gesture.borrow()
                && let Some(w) = self.tile_widget(id)
            {
                w.remove_css_class("lifted");
                w.remove_css_class("blocked");
            }
            let id = match &*imp.gesture.borrow() {
                Gesture::Move { id, .. } | Gesture::Resize { id, .. } => id.clone(),
                _ => String::new(),
            };
            imp.gesture.take();
            if let Some(badge) = imp.badge.take() {
                badge.unparent();
            }
            self.slide(&[id]);
        } else {
            imp.gesture.take();
        }
        self.queue_draw();
    }

    fn setup_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        let grid = self.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(grid) = grid.upgrade() else { return glib::Propagation::Proceed };
            let picked = grid.imp().picked.borrow().clone();
            let focused = grid.focus_child().and_then(|f| grid.imp().slots.borrow().iter().find(|s| s.widget == f).map(|s| s.tile.id.clone()));
            let step = match key {
                gdk::Key::Left | gdk::Key::KP_Left => Some((-1, 0)),
                gdk::Key::Right | gdk::Key::KP_Right => Some((1, 0)),
                gdk::Key::Up | gdk::Key::KP_Up => Some((0, -1)),
                gdk::Key::Down | gdk::Key::KP_Down => Some((0, 1)),
                _ => None,
            };
            match (key, step, picked) {
                (_, Some((dx, dy)), Some(id)) => grid.emit(Event::Nudge(id, dx, dy)),
                (gdk::Key::Delete | gdk::Key::KP_Delete | gdk::Key::BackSpace, _, Some(id)) => grid.emit(Event::Delete(id)),
                (gdk::Key::Escape, _, Some(_)) => grid.emit(Event::Pick(None)),
                (gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space, _, None) if focused.is_some() => grid.emit(Event::Pick(focused)),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.add_controller(keys);
    }

    /// What a library drag brings: its size, for the outline of where it would land.
    pub fn set_incoming(&self, size: Option<(u8, u8)>) {
        self.imp().incoming.set(size);
        if size.is_none() {
            self.imp().landing.set(None);
            self.queue_draw();
        }
    }

    fn setup_drop(&self) {
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
        let grid = self.downgrade();
        drop.connect_motion(move |_, x, y| {
            let Some(grid) = grid.upgrade() else { return gdk::DragAction::empty() };
            let imp = grid.imp();
            let Some(size) = imp.incoming.get() else { return gdk::DragAction::COPY };
            let face = imp.face.borrow();
            let (ox, oy) = imp.origin.get();
            let (cx, cy) = model::drop_cell(imp.metrics.get(), face.grid, size, x - ox, y - oy);
            let ok = face.fits(0, cx, cy, size.0, size.1, None);
            imp.landing.set(Some(((cx, cy, size.0, size.1), ok)));
            grid.queue_draw();
            gdk::DragAction::COPY
        });
        let grid = self.downgrade();
        drop.connect_leave(move |_| {
            if let Some(grid) = grid.upgrade() {
                grid.imp().landing.set(None);
                grid.queue_draw();
            }
        });
        let grid = self.downgrade();
        drop.connect_drop(move |_, value, x, y| {
            let Some(grid) = grid.upgrade() else { return false };
            let Ok(key) = value.get::<String>() else { return false };
            let imp = grid.imp();
            imp.landing.set(None);
            let size = imp.incoming.get().unwrap_or((1, 1));
            let (ox, oy) = imp.origin.get();
            let (cx, cy) = model::drop_cell(imp.metrics.get(), imp.face.borrow().grid, size, x - ox, y - oy);
            grid.queue_draw();
            grid.emit(Event::Drop(key, cx, cy));
            true
        });
        self.add_controller(drop);
    }
}
