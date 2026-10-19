//! The Linksy "Sender" mark, drawn with Cairo in the widget's current color so it follows
//! the system accent and light/dark style. Optionally pulses (the "Live" state) while
//! searching for devices.

use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{cairo, glib};

pub fn new(size: i32, live: bool) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(size).content_height(size).css_classes(["accent"]).build();
    area.set_accessible_role(gtk::AccessibleRole::Img);
    area.update_property(&[gtk::accessible::Property::Label("Linksy")]);

    let phase = Rc::new(Cell::new(0.0f64));
    let p = phase.clone();
    area.set_draw_func(move |area, cr, w, h| draw(area, cr, w, h, if live { Some(p.get()) } else { None }));

    if live && !reduced_motion(&area) {
        let start = Rc::new(Cell::new(None::<i64>));
        area.add_tick_callback(move |area, clock| {
            let t = clock.frame_time();
            let t0 = start.get().unwrap_or_else(|| {
                start.set(Some(t));
                t
            });
            phase.set(((t - t0) as f64 / 1_000_000.0) / 1.8 % 1.0);
            area.queue_draw();
            glib::ControlFlow::Continue
        });
    }
    area
}

fn draw(area: &gtk::DrawingArea, cr: &cairo::Context, w: i32, h: i32, phase: Option<f64>) {
    let c = area.color();
    let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
    let s = w.min(h) as f64 / 100.0;
    cr.translate((w as f64 - 100.0 * s) / 2.0, (h as f64 - 100.0 * s) / 2.0);
    cr.scale(s, s);
    cr.set_line_cap(cairo::LineCap::Round);

    // The two arcs pulse in turn, half a period apart.
    let pulse = |offset: f64| phase.map(|p| 0.25 + 0.75 * (0.5 + 0.5 * ((p + offset) * 2.0 * PI).cos())).unwrap_or(1.0);

    // Sender: large device and its reach.
    cr.set_source_rgba(r, g, b, 1.0);
    cr.arc(25.0, 75.0, 12.0, 0.0, 2.0 * PI);
    let _ = cr.fill();
    cr.set_source_rgba(r, g, b, pulse(0.0));
    cr.set_line_width(9.5);
    cr.arc(25.0, 75.0, 28.0, -PI / 2.0, 0.0);
    let _ = cr.stroke();

    // Receiver: smaller device answering, in a lighter tone of the same color.
    cr.set_source_rgba(r, g, b, 0.6 * pulse(0.5));
    cr.set_line_width(8.0);
    cr.arc(78.0, 22.0, 20.0, PI / 2.0, PI);
    let _ = cr.stroke();
    cr.set_source_rgba(r, g, b, 0.6);
    cr.arc(78.0, 22.0, 8.0, 0.0, 2.0 * PI);
    let _ = cr.fill();
}

fn reduced_motion(widget: &impl IsA<gtk::Widget>) -> bool {
    !widget.settings().is_gtk_enable_animations()
}

/// Smoothly animate a progress bar towards `target` instead of jumping.
pub fn ease_progress(bar: &gtk::ProgressBar, target: f64) {
    let from = bar.fraction();
    if (target - from).abs() < 0.002 || reduced_motion(bar) {
        bar.set_fraction(target);
        return;
    }
    let bar = bar.clone();
    let steps = 8;
    let i = Rc::new(Cell::new(0));
    glib::timeout_add_local(Duration::from_millis(12), move || {
        i.set(i.get() + 1);
        let t = i.get() as f64 / steps as f64;
        let eased = 1.0 - (1.0 - t).powi(3);
        bar.set_fraction(from + (target - from) * eased);
        if i.get() >= steps { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}
