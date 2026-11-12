//! The first run: a welcome in five slides at most, shown once on a computer where Linksy
//! was never used, and skippable. What Linksy does, the phone's app (a QR code to its
//! download), pairing (the same code as Add Device), Finish Setup (the same checks and
//! buttons as its dialog), and where things are. It covers the window, which works as
//! usual under it: a phone that pairs meanwhile moves the welcome on.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use linksy_i18n::t;

use super::{Window, invite, setup};

/// The slide where the phone pairs, and the one after it.
const PAIR: u32 = 2;
const SETUP: u32 = 3;

pub(super) struct Onboarding {
    root: gtk::Widget,
    carousel: adw::Carousel,
    pair_qr: gtk::DrawingArea,
    pair_code: Rc<RefCell<Option<invite::Modules>>>,
    pair_status: gtk::Label,
    /// An invite was asked for (it is cancelled when the welcome closes).
    invited: Cell<bool>,
    renew: RefCell<Option<glib::SourceId>>,
    setup: setup::SetupView,
}

fn title(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).wrap(true).justify(gtk::Justification::Center).css_classes(["title-1"]).build()
}

fn body(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).wrap(true).justify(gtk::Justification::Center).max_width_chars(44).css_classes(["dim-label"]).build()
}

/// One thing Linksy does, or where something is: an icon and a line.
fn point(icon: &str, text: &str) -> gtk::Box {
    let row = gtk::Box::builder().spacing(14).build();
    row.append(&gtk::Image::builder().icon_name(icon).pixel_size(22).valign(gtk::Align::Center).css_classes(["onboarding-icon"]).build());
    row.append(&gtk::Label::builder().label(text).wrap(true).xalign(0.0).hexpand(true).width_chars(38).max_width_chars(44).build());
    row
}

fn points(rows: &[gtk::Box]) -> gtk::Box {
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).margin_top(10).halign(gtk::Align::Center).build();
    for row in rows {
        list.append(row);
    }
    list
}

fn qr(code: &Rc<RefCell<Option<invite::Modules>>>, label: &str) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(208).content_height(208).halign(gtk::Align::Center).css_classes(["qr-card"]).build();
    area.set_accessible_role(gtk::AccessibleRole::Img);
    area.update_property(&[gtk::accessible::Property::Label(label)]);
    let code = code.clone();
    area.set_draw_func(move |_, cr, w, h| invite::draw_qr(cr, w, h, code.borrow().as_ref()));
    area
}

/// A slide: its content in the middle, scrolling when the window is short.
fn slide(children: &[&gtk::Widget]) -> gtk::Widget {
    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).valign(gtk::Align::Center).margin_top(12).margin_bottom(12).build();
    for child in children {
        column.append(*child);
    }
    let clamp = adw::Clamp::builder().maximum_size(480).child(&column).margin_start(24).margin_end(24).build();
    gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).hexpand(true).vexpand(true).build().upcast()
}

impl Window {
    /// Shows the welcome if this is the first run here. Whether it did.
    pub(super) fn offer_onboarding(self: &Rc<Self>) -> bool {
        let due = linksy_setup::onboarding_due(linksy_news::used_before());
        if due {
            self.show_onboarding();
        }
        due
    }

    pub(super) fn show_onboarding(self: &Rc<Self>) {
        if self.state.borrow().onboarding.is_some() {
            return;
        }
        let carousel = adw::Carousel::builder().allow_scroll_wheel(false).hexpand(true).vexpand(true).build();

        // 1. What Linksy does
        let welcome = slide(&[
            crate::mark::new(88, false).upcast_ref(),
            title(t!("Welcome to Linksy")).upcast_ref(),
            body(t!("Your phone and this computer, working as one.")).upcast_ref(),
            points(&[
                point("document-send-symbolic", t!("Files and the clipboard, both ways")),
                point("camera-web-symbolic", t!("The phone as a webcam, a microphone or a second screen")),
                point("mail-unread-symbolic", t!("Its texts, notifications and calls on this computer")),
            ])
            .upcast_ref(),
        ]);

        // 2. The phone's app
        let app_code = Rc::new(RefCell::new(invite::encode(linksy_setup::PHONE_APP).ok()));
        let phone = slide(&[
            title(t!("Get Linksy on Your Phone")).upcast_ref(),
            qr(&app_code, t!("QR code to download Linksy for Android")).upcast_ref(),
            body(t!("Scan this with your phone's camera to download the Android app, then open it.")).upcast_ref(),
            gtk::LinkButton::builder().uri(linksy_setup::PHONE_APP).label(linksy_setup::PHONE_APP.trim_start_matches("https://")).halign(gtk::Align::Center).build().upcast_ref(),
        ]);

        // 3. Pairing
        let pair_code: Rc<RefCell<Option<invite::Modules>>> = Rc::default();
        let pair_qr = qr(&pair_code, t!("Pairing QR code"));
        let pair_status = gtk::Label::builder().label(t!("Waiting for your phone…")).wrap(true).justify(gtk::Justification::Center).css_classes(["heading"]).build();
        let pair = slide(&[
            title(t!("Pair Your Phone")).upcast_ref(),
            pair_qr.upcast_ref(),
            body(t!("In Linksy on your phone, choose Add Device and scan this code. Whoever scans it can pair, so only show it to your own devices.")).upcast_ref(),
            pair_status.upcast_ref(),
        ]);

        // 4. Finish Setup: the dialog's own list and buttons
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let everything = gtk::Button::builder().label(t!("Set Up Everything")).halign(gtk::Align::Center).css_classes(["pill", "suggested-action"]).build();
        let progress = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label"]).visible(false).build();
        let finish = slide(&[
            title(t!("Finish Setup")).upcast_ref(),
            body(t!("A few parts of your system make every feature work. Linksy adds what's missing in one go; your computer asks for your password once.")).upcast_ref(),
            everything.upcast_ref(),
            progress.upcast_ref(),
            list.upcast_ref(),
        ]);

        // 5. Where things are
        let done = slide(&[
            gtk::Image::builder().icon_name("object-select-symbolic").pixel_size(64).css_classes(["success"]).build().upcast_ref(),
            title(t!("You're All Set")).upcast_ref(),
            points(&[
                point("computer-symbolic", t!("Your devices are in the list on the left: pick one to share with it")),
                point("document-send-symbolic", t!("Drop files on a device's page, or use Send with Linksy in your file manager")),
                point("emblem-system-symbolic", t!("Finish Setup shows under the list while something is missing; Preferences are in the menu")),
            ])
            .upcast_ref(),
        ]);
        for page in [&welcome, &phone, &pair, &finish, &done] {
            carousel.append(page);
        }

        let back = gtk::Button::builder().label(t!("Back")).css_classes(["flat"]).visible(false).build();
        let next = gtk::Button::builder().label(t!("Next")).css_classes(["pill", "suggested-action"]).build();
        let dots = adw::CarouselIndicatorDots::builder().carousel(&carousel).hexpand(true).build();
        let bar = gtk::CenterBox::builder().margin_top(6).margin_bottom(18).margin_start(24).margin_end(24).build();
        bar.set_start_widget(Some(&back));
        bar.set_center_widget(Some(&dots));
        bar.set_end_widget(Some(&next));
        // As wide as each other, so the dots stay in the middle
        let sides = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        sides.add_widget(&back);
        sides.add_widget(&next);

        let skip = gtk::Button::builder().label(t!("Skip")).css_classes(["flat"]).build();
        let header = adw::HeaderBar::builder().show_title(false).build();
        header.pack_end(&skip);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&carousel));
        view.add_bottom_bar(&bar);
        view.add_css_class("background");
        let root: gtk::Widget = view.upcast();
        root.update_property(&[gtk::accessible::Property::Label(t!("Welcome to Linksy"))]);

        let setup = setup::SetupView {
            parent: root.clone(),
            list,
            toasts: self.toasts.clone(),
            everything: everything.clone(),
            progress,
            // The command for a terminal stays in the dialog
            terminal: gtk::ListBox::new(),
            command: gtk::Label::new(None),
        };
        let o = Rc::new(Onboarding { root: root.clone(), carousel: carousel.clone(), pair_qr, pair_code, pair_status, invited: Cell::new(false), renew: RefCell::default(), setup });
        self.state.borrow_mut().onboarding = Some(o.clone());
        self.fill_setup(&o.setup);
        let (w, view) = (self.clone(), o.setup.clone());
        everything.connect_clicked(move |_| w.clone().fix(view.clone(), linksy_setup::actionable(&linksy_setup::check(linksy_setup::Ui::Gtk))));

        let last = carousel.n_pages() - 1;
        let (w, b, n) = (Rc::downgrade(self), back.clone(), next.clone());
        carousel.connect_page_changed(move |_, index| {
            b.set_visible(index > 0);
            n.set_label(if index == last { t!("Start Using Linksy") } else { t!("Next") });
            let Some(w) = w.upgrade() else { return };
            match index {
                PAIR => w.onboarding_pair_slide(),
                // What was set up outside meanwhile shows as ready
                SETUP => {
                    let view = w.state.borrow().onboarding.as_ref().map(|o| o.setup.clone());
                    if let Some(view) = view {
                        w.fill_setup(&view);
                    }
                }
                _ => {}
            }
        });
        let c = carousel.clone();
        back.connect_clicked(move |_| {
            let at = c.position().round() as u32;
            if at > 0 {
                c.scroll_to(&c.nth_page(at - 1), true);
            }
        });
        let (w, c) = (Rc::downgrade(self), carousel.clone());
        next.connect_clicked(move |_| {
            let at = c.position().round() as u32;
            if at < last {
                c.scroll_to(&c.nth_page(at + 1), true);
            } else if let Some(w) = w.upgrade() {
                w.close_onboarding();
            }
        });
        let w = Rc::downgrade(self);
        skip.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.close_onboarding();
            }
        });
        self.overlay.add_overlay(&root);
        next.grab_focus();
        // Development aid (screenshots): LINKSY_ONBOARDING_SLIDE=3 opens on the third slide.
        if let Some(n) = std::env::var("LINKSY_ONBOARDING_SLIDE").ok().and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=last + 1).contains(n)) {
            let c = carousel.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || c.scroll_to(&c.nth_page(n - 1), false));
        }
    }

    /// Done or skipped: not shown again, and the pairing code it showed stops working.
    fn close_onboarding(self: &Rc<Self>) {
        let Some(o) = self.state.borrow_mut().onboarding.take() else { return };
        linksy_setup::set_onboarded();
        if let Some(timer) = o.renew.borrow_mut().take() {
            timer.remove();
        }
        if o.invited.get()
            && self.state.borrow().invite_dialog.is_none()
            && let Some(c) = self.client()
        {
            let call = self.bridge.call(&c, |c| async move { c.cancel_invite().await });
            glib::spawn_future_local(async move {
                let _ = call.await;
            });
        }
        self.overlay.remove_overlay(&o.root);
        self.refresh_setup();
    }

    /// The pairing slide's code: asked for when the slide first shows (again in a moment
    /// while the service is still starting), and renewed before it expires.
    fn onboarding_pair_slide(self: &Rc<Self>) {
        let Some(o) = self.state.borrow().onboarding.clone() else { return };
        if o.invited.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        let timer = glib::timeout_add_local(invite::RENEW_EVERY, move || match w.upgrade() {
            Some(w) => {
                w.onboarding_invite();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        *o.renew.borrow_mut() = Some(timer);
        self.onboarding_invite();
    }

    fn onboarding_invite(self: &Rc<Self>) {
        let Some(o) = self.state.borrow().onboarding.clone() else { return };
        let Some(c) = self.client() else {
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_secs(1), move || {
                if let Some(w) = w.upgrade() {
                    w.onboarding_invite();
                }
            });
            return;
        };
        let call = self.bridge.call(&c, |c| async move { c.create_invite().await });
        glib::spawn_future_local(async move {
            if let Ok(link) = call.await
                && let Ok(code) = invite::encode(&link)
            {
                *o.pair_code.borrow_mut() = Some(code);
                o.pair_qr.queue_draw();
            }
        });
    }

    /// A device paired while the welcome shows: say so, and on to Finish Setup.
    pub(super) fn onboarding_paired(&self, name: &str) {
        let Some(o) = self.state.borrow().onboarding.clone() else { return };
        o.pair_status.set_label(&t!("Paired with {name}", name = name));
        o.pair_status.add_css_class("success");
        if o.carousel.position().round() as u32 == PAIR {
            o.carousel.scroll_to(&o.carousel.nth_page(SETUP), true);
        }
    }
}
