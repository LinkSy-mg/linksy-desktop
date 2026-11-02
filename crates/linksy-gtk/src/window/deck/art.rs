//! What the deck's tiles draw themselves, per theme: icons as solid, dotted or pixel
//! strokes, text in 5×7 glyphs (dots or squares), bars and the music's cover.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, glib, graphene, gsk};

use super::model::{self, Theme};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Draw {
    #[default]
    Nothing,
    /// An icon of [`model::ICONS`], `size` pixels square.
    Icon(&'static str, i32),
    /// Lines of glyphs, `pitch` pixels a dot.
    Text(Vec<String>, f32),
    /// A bar, `value` of it full; `thick` is the volume's.
    Bar { value: f64, vertical: bool, thick: bool },
    Cover,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Art {
        pub draw: RefCell<Draw>,
        pub theme: Cell<Theme>,
        pub accent: Cell<[f32; 3]>,
        /// An icon's stroke, in its 24-unit box.
        pub stroke: Cell<f32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Art {
        const NAME: &'static str = "LinksyDeckArt";
        type Type = super::Art;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Art {}

    impl WidgetImpl for Art {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let horizontal = orientation == gtk::Orientation::Horizontal;
            let n = match &*self.draw.borrow() {
                Draw::Nothing => 0,
                Draw::Icon(_, size) => *size,
                Draw::Text(lines, pitch) => {
                    let n = if horizontal { lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) * 6 } else { lines.len() * 9 };
                    (n.saturating_sub(if horizontal { 1 } else { 2 }) as f32 * pitch).ceil() as i32
                }
                Draw::Bar { vertical, thick, .. } => match (horizontal == *vertical, thick, self.theme.get()) {
                    (false, _, _) => 8,
                    (true, true, Theme::Pixel) => 22,
                    (true, true, _) => 26,
                    (true, false, Theme::Pixel) => 8,
                    (true, false, Theme::Dots) => 6,
                    (true, false, _) => 4,
                },
                Draw::Cover => 16,
            };
            (n, n, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            let ink = widget.color();
            let theme = self.theme.get();
            match &*self.draw.borrow() {
                Draw::Nothing => {}
                Draw::Icon(name, size) => icon(snapshot, name, *size as f32, w, h, theme, &ink, self.stroke.get()),
                Draw::Text(lines, pitch) => text(snapshot, lines, *pitch, w, theme, &ink),
                Draw::Bar { value, vertical, thick } => bar(snapshot, *value as f32, *vertical, *thick, w, h, theme, self.accent.get()),
                Draw::Cover => cover(snapshot, w.min(h), theme),
            }
        }
    }
}

glib::wrapper! {
    pub struct Art(ObjectSubclass<imp::Art>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Art {
    pub fn new(draw: Draw, theme: Theme, accent: &str) -> Self {
        let art: Self = glib::Object::new();
        let imp = art.imp();
        imp.theme.set(theme);
        imp.accent.set(rgb(accent));
        imp.stroke.set(2.0);
        *imp.draw.borrow_mut() = draw;
        art.set_can_target(false);
        art
    }

    /// An icon, `size` pixels square.
    pub fn icon(name: &'static str, size: i32, theme: Theme) -> Self {
        let art = Self::new(Draw::Icon(name, size), theme, "#ffffff");
        art.set_halign(gtk::Align::Center);
        art.set_valign(gtk::Align::Center);
        art
    }

    pub fn set_stroke(&self, width: f32) {
        self.imp().stroke.set(width);
        self.queue_draw();
    }
}

fn rgb(hex: &str) -> [f32; 3] {
    gdk::RGBA::parse(hex).map(|c| [c.red(), c.green(), c.blue()]).unwrap_or([0.21, 0.52, 0.89])
}

fn color(hex: &str, alpha: f32) -> gdk::RGBA {
    let [r, g, b] = rgb(hex);
    gdk::RGBA::new(r, g, b, alpha)
}

fn white(alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(1.0, 1.0, 1.0, alpha)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> graphene::Rect {
    graphene::Rect::new(x, y, w, h)
}

fn rounded(r: graphene::Rect, radius: f32) -> gsk::RoundedRect {
    gsk::RoundedRect::from_rect(r, radius.min(r.width() / 2.0).min(r.height() / 2.0))
}

thread_local! {
    static PATHS: RefCell<HashMap<&'static str, gsk::Path>> = RefCell::default();
    static MASKS: RefCell<HashMap<&'static str, [[bool; 12]; 12]>> = RefCell::default();
}

fn path(name: &'static str) -> Option<gsk::Path> {
    PATHS.with_borrow_mut(|paths| {
        if let Some(p) = paths.get(name) {
            return Some(p.clone());
        }
        let p = gsk::Path::parse(model::icon_path(name)).ok()?;
        paths.insert(name, p.clone());
        Some(p)
    })
}

/// The icon on a 12-pixel grid, every pixel on or off: stroked with smoothing on, then each
/// pixel kept where it's more than faintly covered.
fn pixel_mask(name: &'static str) -> [[bool; 12]; 12] {
    if let Some(m) = MASKS.with_borrow(|m| m.get(name).copied()) {
        return m;
    }
    let mut mask = [[false; 12]; 12];
    if let (Some(p), Ok(mut surface)) = (path(name), cairo::ImageSurface::create(cairo::Format::ARgb32, 12, 12)) {
        if let Ok(cr) = cairo::Context::new(&surface) {
            cr.scale(0.5, 0.5);
            cr.set_line_width(2.6);
            cr.set_line_cap(cairo::LineCap::Square);
            cr.set_line_join(cairo::LineJoin::Miter);
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            p.to_cairo(&cr);
            let _ = cr.stroke();
        }
        surface.flush();
        let stride = surface.stride() as usize;
        if let Ok(data) = surface.data() {
            for (y, row) in mask.iter_mut().enumerate() {
                for (x, px) in row.iter_mut().enumerate() {
                    // ARGB32 in native order: alpha is the last byte on little endian
                    let i = y * stride + x * 4 + if cfg!(target_endian = "little") { 3 } else { 0 };
                    *px = data[i] > 100;
                }
            }
        }
    }
    MASKS.with_borrow_mut(|m| m.insert(name, mask));
    mask
}

#[allow(clippy::too_many_arguments)]
fn icon(snapshot: &gtk::Snapshot, name: &'static str, size: f32, w: f32, h: f32, theme: Theme, ink: &gdk::RGBA, width: f32) {
    if theme == Theme::Pixel {
        let px = (size / 12.0).floor().max(1.0);
        let (ox, oy) = (((w - 12.0 * px) / 2.0).round(), ((h - 12.0 * px) / 2.0).round());
        let squares = gsk::PathBuilder::new();
        for (y, row) in pixel_mask(name).iter().enumerate() {
            for (x, on) in row.iter().enumerate() {
                if *on {
                    squares.add_rect(&rect(ox + x as f32 * px, oy + y as f32 * px, px, px));
                }
            }
        }
        snapshot.append_fill(&squares.to_path(), gsk::FillRule::Winding, ink);
        return;
    }
    let Some(p) = path(name) else { return };
    let stroke = gsk::Stroke::new(if theme == Theme::Dots { 2.3 } else { width });
    stroke.set_line_cap(gsk::LineCap::Round);
    stroke.set_line_join(gsk::LineJoin::Round);
    if theme == Theme::Dots {
        stroke.set_dash(&[0.0, 2.7]);
    }
    let scale = size / 24.0;
    let draw = |color: &gdk::RGBA| {
        snapshot.save();
        snapshot.translate(&graphene::Point::new((w - size) / 2.0, (h - size) / 2.0));
        snapshot.scale(scale, scale);
        snapshot.append_stroke(&p, &stroke, color);
        snapshot.restore();
    };
    if theme == Theme::Term {
        // Phosphor: a soft glow under the line
        snapshot.push_blur(4.0);
        draw(&gdk::RGBA::new(ink.red(), ink.green(), ink.blue(), ink.alpha() * 0.6));
        snapshot.pop();
    }
    draw(ink);
}

/// Lines of glyphs: round dots in Dot Matrix, squares in Pixel. Starts at the left; a line
/// longer than the width is cut.
fn text(snapshot: &gtk::Snapshot, lines: &[String], pitch: f32, w: f32, theme: Theme, ink: &gdk::RGBA) {
    let dots = gsk::PathBuilder::new();
    let square = theme == Theme::Pixel;
    for (row, line) in lines.iter().enumerate() {
        let y0 = row as f32 * 9.0 * pitch;
        for (i, c) in line.chars().enumerate() {
            let x0 = i as f32 * 6.0 * pitch;
            if x0 + 5.0 * pitch > w + 0.5 {
                break;
            }
            let Some(bits) = model::glyph(c) else { continue };
            for (y, bits) in bits.iter().enumerate() {
                for x in 0..5 {
                    if bits & (0x10 >> x) == 0 {
                        continue;
                    }
                    let (px, py) = (x0 + x as f32 * pitch, y0 + y as f32 * pitch);
                    if square {
                        dots.add_rect(&rect(px, py, pitch, pitch));
                    } else {
                        dots.add_circle(&graphene::Point::new(px + pitch / 2.0, py + pitch / 2.0), pitch * 0.42);
                    }
                }
            }
        }
    }
    snapshot.append_fill(&dots.to_path(), gsk::FillRule::Winding, ink);
}

#[allow(clippy::too_many_arguments)]
fn bar(snapshot: &gtk::Snapshot, value: f32, vertical: bool, thick: bool, w: f32, h: f32, theme: Theme, accent: [f32; 3]) {
    let value = value.clamp(0.0, 1.0);
    let whole = rect(0.0, 0.0, w, h);
    // The full part: from the left, or up from the bottom
    let full = if vertical { rect(0.0, h * (1.0 - value), w, h * value) } else { rect(0.0, 0.0, w * value, h) };
    let accent = gdk::RGBA::new(accent[0], accent[1], accent[2], 1.0);
    match theme {
        Theme::Dots => {
            let pitch = if thick { 6.0 } else { 5.0 };
            let (lit, unlit) = (gsk::PathBuilder::new(), gsk::PathBuilder::new());
            let (cols, rows) = ((w / pitch).floor().max(1.0) as i32, if thick || vertical { (h / pitch).floor().max(1.0) as i32 } else { 1 });
            let (ox, oy) = ((w - cols as f32 * pitch) / 2.0, (h - rows as f32 * pitch) / 2.0);
            for y in 0..rows {
                for x in 0..cols {
                    let at = graphene::Point::new(ox + (x as f32 + 0.5) * pitch, oy + (y as f32 + 0.5) * pitch);
                    let on = if vertical { (rows - y) as f32 <= value * rows as f32 + 0.01 } else { (x as f32) < value * cols as f32 - 0.01 };
                    if on {
                        lit.add_circle(&at, if thick { 1.5 } else { 1.3 });
                    } else {
                        unlit.add_circle(&at, if thick { 1.4 } else { 1.2 });
                    }
                }
            }
            snapshot.append_fill(&unlit.to_path(), gsk::FillRule::Winding, &white(if thick { 0.18 } else { 0.22 }));
            snapshot.append_fill(&lit.to_path(), gsk::FillRule::Winding, &white(1.0));
        }
        Theme::Pixel => {
            snapshot.append_color(&gdk::RGBA::BLACK, &whole);
            let (on, off, fill): (f32, f32, _) = if thick { (6.0, 2.0, color("#ffec27", 1.0)) } else { (5.0, 2.0, color("#00e436", 1.0)) };
            let inner = rect(full.x() + 2.0, full.y() + 2.0, (full.width() - 4.0).max(0.0), (full.height() - 4.0).max(0.0));
            let segments = gsk::PathBuilder::new();
            let length = if vertical { inner.height() } else { inner.width() };
            let mut at = 0.0;
            while at < length {
                let n = on.min(length - at);
                segments.add_rect(&if vertical { rect(inner.x(), inner.y() + inner.height() - at - n, inner.width(), n) } else { rect(inner.x() + at, inner.y(), n, inner.height()) });
                at += on + off;
            }
            snapshot.append_fill(&segments.to_path(), gsk::FillRule::Winding, &fill);
        }
        _ => {
            let (track, fill, radius) = match theme {
                Theme::Eink => (color("#cfcfc6", 1.0), color("#111111", 1.0), if thick { 10.0 } else { 3.0 }),
                Theme::Term => (color("#39ff7a", 0.15), color("#39ff7a", 1.0), 0.0),
                _ => (white(0.14), accent, if thick { 10.0 } else { 3.0 }),
            };
            snapshot.push_rounded_clip(&rounded(whole, radius));
            snapshot.append_color(&track, &whole);
            snapshot.append_color(&fill, &full);
            snapshot.pop();
        }
    }
}

fn cover(snapshot: &gtk::Snapshot, size: f32, theme: Theme) {
    let whole = rect(0.0, 0.0, size, size);
    match theme {
        Theme::Pixel => {
            let px = (size / 10.0).floor().max(1.0);
            for (y, row) in model::pixel_art().iter().enumerate() {
                for (x, hex) in row.iter().enumerate() {
                    snapshot.append_color(&color(hex, 1.0), &rect(x as f32 * px, y as f32 * px, px, px));
                }
            }
        }
        Theme::Dots => {
            // A disc of lit dots that fades out from its upper left
            let (pitch, center) = (6.0f32, size * 0.38);
            let reach = (2.0 * (size - center).powi(2)).sqrt();
            let n = (size / pitch).floor() as i32;
            for y in 0..n {
                for x in 0..n {
                    let at = graphene::Point::new((x as f32 + 0.5) * pitch, (y as f32 + 0.5) * pitch);
                    let d = ((at.x() - center).powi(2) + (at.y() - center).powi(2)).sqrt() / reach;
                    let alpha = ((0.72 - d) / (0.72 - 0.25)).clamp(0.0, 1.0);
                    if alpha > 0.02 {
                        let dot = gsk::PathBuilder::new();
                        dot.add_circle(&at, 1.4);
                        snapshot.append_fill(&dot.to_path(), gsk::FillRule::Winding, &white(alpha));
                    }
                }
            }
        }
        _ => {
            let (stops, radius): (&[&str], f32) = match theme {
                Theme::Eink => (&["#aaaaaa", "#f7f7f7", "#e2e2e2", "#757575", "#aaaaaa"], 6.0),
                Theme::Term => (&["#0f6a2a", "#3cff7f", "#2fd468", "#0a4a1d", "#0f6a2a"], 2.0),
                _ => (&["#ff7a59", "#ffd36e", "#6ee7ff", "#7c5cff", "#ff7a59"], 8.0),
            };
            let stops: Vec<gsk::ColorStop> = stops.iter().enumerate().map(|(i, c)| gsk::ColorStop::new(i as f32 / 4.0, color(c, 1.0))).collect();
            snapshot.push_rounded_clip(&rounded(whole, radius));
            snapshot.append_conic_gradient(&whole, &graphene::Point::new(size / 2.0, size / 2.0), 200.0, &stops);
            snapshot.pop();
        }
    }
}
