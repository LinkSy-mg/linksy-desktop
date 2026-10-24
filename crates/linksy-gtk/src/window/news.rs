//! What's New: the highlights of this version (linksy-news), the first time it opens after
//! an update and from the menu. Each one fades in a moment after the one before, its icon
//! popping in; with GNOME's animations off, they're simply there.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use linksy_i18n::t;

use super::Window;

/// Between one highlight appearing and the next.
const STAGGER: Duration = Duration::from_millis(90);

impl Window {
    /// After an update, once: what's new in it.
    pub(super) fn offer_news(self: &Rc<Self>) {
        let version = env!("CARGO_PKG_VERSION");
        let news = linksy_news::to_show(version);
        if !news.is_empty() {
            linksy_news::mark_seen(version);
            self.show_news_of(news);
        }
    }

    /// What's New in the menu: this version's highlights, any time.
    pub fn show_news(self: &Rc<Self>) {
        self.show_news_of(linksy_news::latest(env!("CARGO_PKG_VERSION")));
    }

    /// `news`: one version's highlights, or (after skipping versions) each version's,
    /// newest first, under its own heading.
    fn show_news_of(self: &Rc<Self>, news: Vec<linksy_news::Release>) {
        let version = env!("CARGO_PKG_VERSION");
        let dialog = adw::Dialog::builder().title(t!("What's New")).content_width(460).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(24).margin_top(6).margin_bottom(24).margin_start(28).margin_end(28).build();

        // Linksy's mark, drawn: no icon theme needed
        let logo = crate::mark::new(72, false);
        logo.set_halign(gtk::Align::Center);
        logo.add_css_class("news-pop");
        let several = news.len() > 1;
        let title = if several { t!("What's New Since Your Last Update").to_string() } else { t!("What's New in Linksy {version}", version) };
        let heading = gtk::Label::builder().label(&title).wrap(true).justify(gtk::Justification::Center).css_classes(["title-1"]).build();
        let top = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        top.append(&logo);
        top.append(&heading);
        body.append(&top);

        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).build();
        let mut rows = Vec::new();
        for release in &news {
            if several {
                let label = gtk::Label::builder().label(t!("Linksy {version}", version = release.version)).xalign(0.0).margin_top(6).css_classes(["caption-heading", "dim-label"]).build();
                list.append(&label);
            }
            for h in &release.highlights {
                let icon = gtk::Image::builder().icon_name(h.gtk).pixel_size(20).valign(gtk::Align::Start).css_classes(["news-icon"]).build();
                let title = gtk::Label::builder().label(h.title).xalign(0.0).wrap(true).css_classes(["heading"]).build();
                let text = gtk::Label::builder().label(h.body).xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
                let words = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).hexpand(true).build();
                words.append(&title);
                words.append(&text);
                let row = gtk::Box::builder().spacing(14).opacity(0.0).build();
                row.append(&icon);
                row.append(&words);
                list.append(&row);
                rows.push((row, icon));
            }
        }
        body.append(&list);

        let more = gtk::LinkButton::builder().uri(linksy_news::notes_url(version)).label(t!("All Changes in {version}", version)).halign(gtk::Align::Center).build();
        body.append(&more);

        // Continue, and a word for the project
        let done = gtk::Button::builder().label(t!("Continue")).halign(gtk::Align::Center).css_classes(["pill", "suggested-action"]).build();
        let star = gtk::Button::builder()
            .child(&adw::ButtonContent::builder().icon_name("starred-symbolic").label(t!("Star Linksy on GitHub")).build())
            .halign(gtk::Align::Center)
            .css_classes(["pill", "news-star"])
            .build();
        let ask = gtk::Label::builder()
            .label(t!("Linksy is free and open source. A star on GitHub helps others find it."))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label", "caption"])
            .build();
        let end = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        end.append(&done);
        end.append(&star);
        end.append(&ask);
        body.append(&end);

        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).child(&body).build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
        view.set_content(Some(&scroller));
        dialog.set_child(Some(&view));

        let d = dialog.clone();
        done.connect_clicked(move |_| {
            d.close();
        });
        let w = self.window.clone();
        star.connect_clicked(move |_| {
            gtk::UriLauncher::new(linksy_news::REPO).launch(Some(&w), gtk::gio::Cancellable::NONE, |_| {});
        });

        dialog.present(Some(&self.window));
        // One after another, each fading in with its icon popping
        for (i, (row, icon)) in rows.into_iter().enumerate() {
            glib::timeout_add_local_once(STAGGER * (i as u32 + 2), move || {
                let target = adw::PropertyAnimationTarget::new(&row, "opacity");
                adw::TimedAnimation::builder().widget(&row).value_from(0.0).value_to(1.0).duration(320).target(&target).build().play();
                icon.add_css_class("news-pop");
            });
        }
    }
}
