//! "Add Device": every way to pair in one place. A QR code for phones (whoever scans it
//! pairs without comparing codes; renewed before it expires, dead once the dialog closes),
//! computers nearby with a Pair button, and pairing by address.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{cairo, glib};

use super::Window;

/// Invites are valid five minutes; ask for a fresh one a little before that.
const RENEW_EVERY: Duration = Duration::from_secs(270);

/// A QR code as its side length in modules and which modules are dark.
type Modules = (usize, Vec<bool>);

impl Window {
    pub(super) fn show_invite(self: &Rc<Self>) {
        let qr = gtk::DrawingArea::builder().content_width(232).content_height(232).halign(gtk::Align::Center).css_classes(["qr-card"]).build();
        qr.set_accessible_role(gtk::AccessibleRole::Img);
        qr.update_property(&[gtk::accessible::Property::Label("Pairing QR code")]);
        let modules: Rc<RefCell<Option<Modules>>> = Rc::default();
        let m = modules.clone();
        qr.set_draw_func(move |_, cr, w, h| draw_qr(cr, w, h, m.borrow().as_ref()));

        let title = gtk::Label::builder().label("Add a Device").css_classes(["title-2"]).build();
        let body = gtk::Label::builder()
            .label("On your phone, open Linksy and scan this code. Whoever scans it can pair, so only show it to your own devices.")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .max_width_chars(36)
            .css_classes(["dim-label"])
            .build();
        let copy = gtk::Button::builder().label("Copy Link").halign(gtk::Align::Center).css_classes(["pill"]).sensitive(false).build();

        // Computers nearby, kept current by the window while the dialog is open.
        let nearby = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let nearby_empty = gtk::Label::builder()
            .label("No other computers found on this network yet.")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label", "caption"])
            .build();
        let by_address = gtk::Button::builder().label("Pair by Address…").action_name("win.pair-address").halign(gtk::Align::Center).css_classes(["flat"]).build();
        let computers = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).margin_top(10).build();
        computers.append(&gtk::Label::builder().label("Or Pair a Computer Nearby").xalign(0.0).css_classes(["heading"]).build());
        computers.append(&nearby);
        computers.append(&nearby_empty);
        computers.append(&by_address);

        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(16).margin_top(4).margin_bottom(24).margin_start(24).margin_end(24).build();
        content.append(&title);
        content.append(&qr);
        content.append(&body);
        content.append(&copy);
        content.append(&computers);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).child(&content).build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&scroller));
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
        toolbar.set_content(Some(&toasts));
        let dialog = adw::Dialog::builder().title("Add Device").content_width(400).content_height(720).child(&toolbar).build();
        self.state.borrow_mut().invite_nearby = Some((nearby.clone(), nearby_empty.clone()));
        self.render_invite_nearby();

        let uri: Rc<RefCell<String>> = Rc::default();
        let u = uri.clone();
        let t = toasts.clone();
        copy.connect_clicked(move |b| {
            b.clipboard().set_text(&u.borrow());
            t.add_toast(adw::Toast::new("Link copied"));
        });

        // Fetch an invite now and renew it while the dialog is open.
        let renew = {
            let (w, qr, modules, uri, copy, toasts) = (Rc::downgrade(self), qr.clone(), modules.clone(), uri.clone(), copy.clone(), toasts.clone());
            move || {
                let Some(w) = w.upgrade() else { return };
                let Some(c) = w.client() else { return };
                let call = w.bridge.call(&c, |c| async move { c.create_invite().await });
                let (qr, modules, uri, copy, toasts) = (qr.clone(), modules.clone(), uri.clone(), copy.clone(), toasts.clone());
                glib::spawn_future_local(async move {
                    match call.await.map_err(|e| e.to_string()).and_then(|link| encode(&link).map(|m| (link, m))) {
                        Ok((link, m)) => {
                            *modules.borrow_mut() = Some(m);
                            *uri.borrow_mut() = link;
                            copy.set_sensitive(true);
                            qr.queue_draw();
                        }
                        Err(e) => {
                            toasts.add_toast(adw::Toast::new(&glib::markup_escape_text(&crate::fmt::capitalize(&e))));
                        }
                    }
                });
            }
        };
        renew();
        let timer = glib::timeout_add_local(RENEW_EVERY, move || {
            renew();
            glib::ControlFlow::Continue
        });
        let timer = RefCell::new(Some(timer));

        let w = Rc::downgrade(self);
        dialog.connect_closed(move |_| {
            if let Some(t) = timer.borrow_mut().take() {
                t.remove();
            }
            let Some(w) = w.upgrade() else { return };
            w.state.borrow_mut().invite_dialog = None;
            w.state.borrow_mut().invite_nearby = None;
            if let Some(c) = w.client() {
                let call = w.bridge.call(&c, |c| async move { c.cancel_invite().await });
                glib::spawn_future_local(async move {
                    let _ = call.await;
                });
            }
        });
        self.state.borrow_mut().invite_dialog = Some(dialog.clone());
        dialog.present(Some(&self.window));
    }

    /// Refresh the dialog's list of computers nearby that aren't paired yet.
    pub(super) fn render_invite_nearby(self: &Rc<Self>) {
        let (list, empty, devices) = {
            let state = self.state.borrow();
            let Some((list, empty)) = state.invite_nearby.clone() else { return };
            (list, empty, state.devices.iter().filter(|d| !d.trusted).cloned().collect::<Vec<_>>())
        };
        list.remove_all();
        for d in &devices {
            list.append(&self.nearby_row(d));
        }
        list.set_visible(!devices.is_empty());
        empty.set_visible(devices.is_empty());
    }

    /// Someone scanned the code and paired: the dialog has done its job.
    pub(super) fn close_invite_dialog(&self) {
        let dialog = self.state.borrow_mut().invite_dialog.take();
        if let Some(d) = dialog {
            d.close();
        }
    }
}

fn encode(link: &str) -> Result<Modules, String> {
    let code = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::M).map_err(|e| e.to_string())?;
    let dark = code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
    Ok((code.width(), dark))
}

/// Black modules on a white card in both light and dark style: scanners expect dark on
/// light, and a quiet zone of four modules around it.
fn draw_qr(cr: &cairo::Context, w: i32, h: i32, modules: Option<&Modules>) {
    let size = w.min(h) as f64;
    let radius = 16.0;
    cr.new_sub_path();
    cr.arc(size - radius, radius, radius, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(size - radius, size - radius, radius, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(radius, size - radius, radius, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(radius, radius, radius, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    let _ = cr.fill();
    let Some((width, dark)) = modules else { return };
    let quiet = 4.0;
    let module = (size / (*width as f64 + 2.0 * quiet)).floor().max(1.0);
    let offset = ((size - module * *width as f64) / 2.0).floor();
    cr.set_source_rgb(0.07, 0.07, 0.09);
    for (i, &is_dark) in dark.iter().enumerate() {
        if is_dark {
            let (x, y) = ((i % width) as f64, (i / width) as f64);
            cr.rectangle(offset + x * module, offset + y * module, module, module);
        }
    }
    let _ = cr.fill();
}
