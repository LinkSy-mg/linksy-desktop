//! A tile's face as the phone draws it, in each theme. Live data shows as a sample (the
//! music playing, the volume) but the clock, which tells the time.

use gtk::prelude::*;
use gtk::{glib, pango};
use linksy_i18n::t;
use linksy_ipc::linksy_core::DeckTile;

use super::art::{Art, Draw};
use super::model::{self, Metrics, Theme};

/// What a face is drawn with besides its tile.
pub struct Look<'a> {
    pub theme: Theme,
    pub accent: &'a str,
    /// The grid's cells, as the phone shows them.
    pub m: Metrics,
    pub now: &'a glib::DateTime,
    /// Whether this computer offers an action.
    pub offered: &'a dyn Fn(&str) -> bool,
    /// A toggle's state now, where it's known.
    pub on: &'a dyn Fn(&str) -> bool,
    /// Whether the phone asks before it runs an action.
    pub confirm: &'a dyn Fn(&str) -> bool,
}

const PLAYING: bool = true;
const VOLUME: f64 = 0.62;
const PROGRESS: f64 = 0.34;

impl Look<'_> {
    fn scale(&self) -> f64 {
        self.m.cell / 58.0
    }

    fn art(&self, draw: Draw) -> Art {
        Art::new(draw, self.theme, self.accent)
    }

    fn icon(&self, name: &'static str, size: f64) -> gtk::Widget {
        let size = if self.theme == Theme::Pixel { (size / 12.0).round().max(1.0) * 12.0 } else { size };
        Art::icon(name, size.round() as i32, self.theme).upcast()
    }

    /// Text: in glyphs where the theme draws them and has them all, else in its font.
    fn text(&self, s: &str, class: &str, width: f64, lines: usize, pitches: &[f32], center: bool) -> gtk::Widget {
        let s = if self.theme == Theme::Term && class == "t-label" { format!("> {s}") } else { s.to_string() };
        if self.theme.glyphs() && model::has_glyphs(&s) {
            let pitches: Vec<f32> = pitches.iter().map(|p| if self.theme == Theme::Pixel { (p * self.scale() as f32).round().max(1.0) } else { p * self.scale() as f32 }).collect();
            let (lines, pitch) = fit(&s, width, lines, &pitches);
            let art = self.art(Draw::Text(lines, pitch));
            art.set_halign(if center { gtk::Align::Center } else { gtk::Align::Start });
            art.add_css_class(class);
            return art.upcast();
        }
        let label = gtk::Label::builder().label(&s).css_classes([class]).xalign(if center { 0.5 } else { 0.0 }).ellipsize(pango::EllipsizeMode::End).build();
        if lines > 1 {
            label.set_wrap(true);
            label.set_wrap_mode(pango::WrapMode::WordChar);
            label.set_lines(lines as i32);
            label.set_justify(if center { gtk::Justification::Center } else { gtk::Justification::Left });
        }
        label.upcast()
    }

    /// Numbers, `px` high in a font (or the glyphs' height).
    fn number(&self, s: &str, class: &str, px: f64, width: f64) -> gtk::Widget {
        if self.theme.glyphs() {
            let pitch = (px / 7.0) as f32;
            return self.text(s, class, width, s.lines().count().max(1), &[pitch, pitch * 0.85, pitch * 0.7], false);
        }
        let label = gtk::Label::builder().label(s).css_classes([class]).xalign(0.0).build();
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrSize::new_size_absolute((px * self.scale() * pango::SCALE as f64) as i32));
        label.set_attributes(Some(&attrs));
        label.upcast()
    }
}

/// The lines of `s` at the biggest of `pitches` that leaves nothing out, else cut at the last.
fn fit(s: &str, width: f64, lines: usize, pitches: &[f32]) -> (Vec<String>, f32) {
    let per = |p: f32| ((width + p as f64) / (6.0 * p as f64)).floor().max(1.0) as usize;
    for &p in pitches {
        let wrapped = model::wrap(s, per(p), usize::MAX);
        if wrapped.len() <= lines && wrapped.iter().all(|l| l.chars().count() <= per(p)) {
            return (wrapped, p);
        }
    }
    let p = pitches.last().copied().unwrap_or(1.0);
    (model::wrap(s, per(p), lines), p)
}

fn column(spacing: i32) -> gtk::Box {
    gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(spacing).build()
}

fn row(spacing: i32) -> gtk::Box {
    gtk::Box::builder().spacing(spacing).build()
}

fn spacer() -> gtk::Box {
    gtk::Box::builder().vexpand(true).build()
}

/// A tile as the phone shows it.
pub fn tile(t: &DeckTile, look: &Look) -> gtk::Widget {
    let m = look.m;
    let (w, h) = (m.span(t.w), m.span(t.h));
    let one = t.w == 1 && t.h == 1 && t.kind != "clock";
    let action = t.action.as_deref();
    let on = action.is_some_and(|a| (look.on)(a));
    let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).overflow(gtk::Overflow::Hidden).css_classes(["deck-tile", &format!("kind-{}", t.kind)]).build();
    root.set_accessible_role(gtk::AccessibleRole::Button);
    root.update_property(&[gtk::accessible::Property::Label(&t.label)]);
    if one {
        root.add_css_class("one");
    }
    if on {
        root.add_css_class("on");
    }
    let color = if t.color.is_empty() { model::default_color(&t.kind, action) } else { t.color.as_str() };
    let (px, dark) = model::pico(color);
    root.add_css_class(&format!("px{px}"));
    if dark {
        root.add_css_class("ink-dark");
    }
    if action.is_some_and(|a| !(look.offered)(a)) {
        root.add_css_class("unavailable");
        root.set_tooltip_text(Some(t!("This computer doesn't offer this any more")));
    }

    let pad = if one { 5.0 } else { 9.0 };
    let inner = w - 2.0 * pad;
    let face = column(0);
    face.add_css_class("face");
    face.set_vexpand(true);
    let icon_size = if look.theme == Theme::Pixel { 24.0 } else { m.cell * 0.36 };
    let label = if t.label.is_empty() { t.kind.as_str() } else { t.label.as_str() };
    match t.kind.as_str() {
        "clock" => clock(&face, t, look, inner),
        "music" => music(&face, t, look, inner, h - 2.0 * pad),
        "volume" => volume(&face, t, look, inner, icon_size),
        "button" if one => {
            face.set_valign(gtk::Align::Center);
            face.set_spacing(3);
            face.append(&look.icon(model::icon_for(action, PLAYING), icon_size));
            face.append(&look.text(label, "t-label", inner, 2, &[1.4, 1.2, 1.0], true));
        }
        "button" => {
            face.append(&look.icon(model::icon_for(action, PLAYING), icon_size));
            face.first_child().expect("icon").set_halign(gtk::Align::Start);
            face.append(&spacer());
            face.append(&look.text(label, "t-label", inner, 1, &[1.5, 1.3, 1.1], false));
        }
        _ => {
            face.append(&spacer());
            face.append(&look.text(label, "t-label", inner, 2, &[1.3, 1.1], false));
        }
    }
    let content = gtk::Overlay::builder().child(&face).vexpand(true).build();
    if action.is_some_and(|a| (look.confirm)(a)) {
        let mark = gtk::Label::builder().label("!").css_classes(["deck-confirm"]).halign(gtk::Align::End).valign(gtk::Align::Start).can_target(false).build();
        content.add_overlay(&mark);
    }
    root.append(&content);
    root.upcast()
}

fn clock(face: &gtk::Box, t: &DeckTile, look: &Look, inner: f64) {
    let (hh, mm) = (look.now.format("%H").map(String::from).unwrap_or_default(), look.now.format("%M").map(String::from).unwrap_or_default());
    let date = look.now.format("%a %-d %b").map(String::from).unwrap_or_default();
    match (t.w, t.h) {
        (1, _) => {
            face.set_valign(gtk::Align::Center);
            face.append(&look.number(&format!("{hh}\n{mm}"), "mid", 17.0, inner));
        }
        (_, 1) => {
            face.set_valign(gtk::Align::Center);
            face.set_spacing(2);
            face.append(&look.number(&format!("{hh}:{mm}"), "mid", 18.5, inner));
            face.append(&look.text(&date, "small", inner, 1, &[1.1, 1.0], false));
        }
        _ => {
            face.append(&look.number(&format!("{hh}:{mm}"), "big", 29.0, inner));
            face.append(&spacer());
            face.append(&look.text(&date, "small", inner, 1, &[1.3, 1.1], false));
        }
    }
}

fn music(face: &gtk::Box, t: &DeckTile, look: &Look, inner: f64, inner_h: f64) {
    let across = row(9);
    across.set_vexpand(true);
    let cover = t.w >= 4;
    let meta_w = if cover { inner - inner_h - 9.0 } else { inner };
    if cover {
        let art = look.art(Draw::Cover);
        art.set_size_request(inner_h as i32, inner_h as i32);
        art.set_valign(gtk::Align::Center);
        across.append(&art);
    }
    let meta = column(2);
    meta.set_hexpand(true);
    meta.append(&look.text("Midnight City", "title", meta_w, 1, &[1.5, 1.3], false));
    meta.append(&look.text("M83 · Hurry Up, We're Dreaming", "small", meta_w, 1, &[1.0], false));
    let progress = look.art(Draw::Bar { value: PROGRESS, vertical: false, thick: false });
    progress.set_hexpand(true);
    progress.set_margin_top(6);
    meta.append(&progress);
    meta.append(&spacer());
    let controls = row(12);
    controls.set_valign(gtk::Align::End);
    controls.append(&look.icon("prev", 18.0));
    let play = gtk::Box::builder().css_classes(["deck-play"]).width_request(30).height_request(30).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
    let glyph = look.icon(if PLAYING { "pause" } else { "play" }, if look.theme == Theme::Pixel { 12.0 } else { 15.0 });
    glyph.set_hexpand(true);
    play.append(&glyph);
    controls.append(&play);
    controls.append(&look.icon("next", 18.0));
    meta.append(&controls);
    across.append(&meta);
    face.append(&across);
}

fn volume(face: &gtk::Box, t: &DeckTile, look: &Look, inner: f64, icon_size: f64) {
    let vertical = t.h > t.w;
    let bar = look.art(Draw::Bar { value: VOLUME, vertical, thick: true });
    let icon = look.icon(model::icon_for(t.action.as_deref(), PLAYING), icon_size);
    let percent = format!("{}%", (VOLUME * 100.0).round());
    if vertical {
        face.set_spacing(9);
        bar.set_vexpand(true);
        face.append(&bar);
        face.append(&icon);
    } else {
        let across = row(9);
        across.set_vexpand(true);
        across.set_valign(gtk::Align::Center);
        bar.set_hexpand(true);
        bar.set_valign(gtk::Align::Center);
        across.append(&icon);
        across.append(&bar);
        if t.w > 1 {
            across.append(&look.text(&percent, "pct", inner, 1, &[1.4, 1.2], false));
        }
        face.set_valign(gtk::Align::Center);
        face.append(&across);
    }
}
