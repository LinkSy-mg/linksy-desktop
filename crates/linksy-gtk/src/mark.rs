//! The Linksy "Sender" mark, drawn with Cairo in the brand's Signal colors: electric blue
//! for the sender, cyan for the receiver. Optionally pulses (the "Live" state) while
//! searching for devices.

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{cairo, glib};

const WAVE: f64 = 1.8;
const WAVES: f64 = 3.0;

/// A live mark pulses a few times each time it's shown, then holds still: a pulse that never
/// stops redraws the window 60 times a second for as long as it's open.
pub fn new(size: i32, live: bool) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(size).content_height(size).build();
    area.set_accessible_role(gtk::AccessibleRole::Img);
    area.update_property(&[gtk::accessible::Property::Label("Linksy")]);

    let pulse = Rc::new(Cell::new(None::<(f64, f64)>));
    let p = pulse.clone();
    area.set_draw_func(move |area, cr, w, h| draw(area, cr, w, h, p.get()));

    if live {
        let tick = Rc::new(RefCell::new(None::<gtk::TickCallbackId>));
        let (p, t) = (pulse.clone(), tick.clone());
        area.connect_map(move |area| {
            if reduced_motion(area) || t.borrow().is_some() {
                return;
            }
            let (p, t2, start) = (p.clone(), t.clone(), Cell::new(None::<i64>));
            let id = area.add_tick_callback(move |area, clock| {
                let now = clock.frame_time();
                let t0 = start.get().unwrap_or(now);
                start.set(Some(t0));
                let secs = (now - t0) as f64 / 1e6;
                area.queue_draw();
                if secs >= WAVE * WAVES {
                    p.set(None);
                    t2.borrow_mut().take();
                    return glib::ControlFlow::Break;
                }
                let fade_in = (secs / 0.6).min(1.0);
                let fade_out = 1.0 - ((secs - WAVE * (WAVES - 1.0)) / WAVE).clamp(0.0, 1.0);
                p.set(Some((secs / WAVE % 1.0, ease(fade_in.min(fade_out)))));
                glib::ControlFlow::Continue
            });
            *t.borrow_mut() = Some(id);
        });
        area.connect_unmap(move |area| {
            if let Some(id) = tick.borrow_mut().take() {
                id.remove();
            }
            pulse.set(None);
            area.queue_draw();
        });
    }
    area
}

fn ease(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Signal blue `#2563FF` and cyan `#22C3E6`.
pub const SENDER: (f64, f64, f64) = (0x25 as f64 / 255.0, 0x63 as f64 / 255.0, 1.0);
pub const RECEIVER: (f64, f64, f64) = (0x22 as f64 / 255.0, 0xC3 as f64 / 255.0, 0xE6 as f64 / 255.0);

/// `live`: where in the wave it is (0 to 1) and how strongly it pulses (0 to 1); none is still.
fn draw(_area: &gtk::DrawingArea, cr: &cairo::Context, w: i32, h: i32, live: Option<(f64, f64)>) {
    let (r, g, b) = SENDER;
    let s = w.min(h) as f64 / 100.0;
    cr.translate((w as f64 - 100.0 * s) / 2.0, (h as f64 - 100.0 * s) / 2.0);
    cr.scale(s, s);
    cr.set_line_cap(cairo::LineCap::Round);

    // The two arcs pulse in turn, half a period apart.
    let pulse = |offset: f64| live.map(|(p, a)| 1.0 - a * 0.75 * (0.5 - 0.5 * ((p + offset) * 2.0 * PI).cos())).unwrap_or(1.0);

    // Sender: large device and its reach.
    cr.set_source_rgba(r, g, b, 1.0);
    cr.arc(25.0, 75.0, 12.0, 0.0, 2.0 * PI);
    let _ = cr.fill();
    cr.set_source_rgba(r, g, b, pulse(0.0));
    cr.set_line_width(9.5);
    cr.arc(25.0, 75.0, 28.0, -PI / 2.0, 0.0);
    let _ = cr.stroke();

    // Receiver: smaller device answering, in cyan.
    let (r, g, b) = RECEIVER;
    cr.set_source_rgba(r, g, b, pulse(0.5));
    cr.set_line_width(8.0);
    cr.arc(78.0, 22.0, 20.0, PI / 2.0, PI);
    let _ = cr.stroke();
    cr.set_source_rgba(r, g, b, 1.0);
    cr.arc(78.0, 22.0, 8.0, 0.0, 2.0 * PI);
    let _ = cr.fill();
}

pub fn reduced_motion(widget: &impl IsA<gtk::Widget>) -> bool {
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
