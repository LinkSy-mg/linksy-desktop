//! "Why Can't I See My Phone?": what the service finds wrong from this computer, and what Finish
//! Setup says of the firewall, one row each, with a button where Linksy can act on it.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use linksy_i18n::t;
use linksy_ipc::{Finding, FindingAction};

use super::Window;

impl Window {
    pub fn show_doctor(self: &Rc<Self>) {
        let dialog = adw::Dialog::builder().title(t!("Why Can't I See My Phone?")).content_width(560).follows_content_size(true).build();
        let intro = gtk::Label::builder()
            .label(t!("Linksy looks for what could keep a phone from finding this computer, and says what to do about it."))
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).visible(false).build();
        let waiting = gtk::Box::builder().spacing(10).halign(gtk::Align::Center).margin_top(12).build();
        waiting.append(&super::spinner());
        waiting.append(&gtk::Label::builder().label(t!("Looking…")).css_classes(["dim-label"]).build());
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(6).margin_bottom(24).margin_start(24).margin_end(24).build();
        body.append(&intro);
        body.append(&waiting);
        body.append(&list);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).child(&body).build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&scroller));
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.window));

        let Some(c) = self.client() else { return };
        let asked = self.bridge.call(&c, |c| async move { c.doctor().await });
        let firewall = gio::spawn_blocking(linksy_setup::firewall_finding);
        let w = self.clone();
        glib::spawn_future_local(async move {
            let found = asked.await;
            let firewall = firewall.await.ok().flatten();
            waiting.set_visible(false);
            let mut findings: Vec<Finding> = Vec::new();
            match found {
                Ok(f) => findings = f,
                Err(e) => {
                    // The service is the one that looks: without it there is only the firewall
                    findings.push(Finding { id: "service".into(), title: t!("Linksy's service isn't answering").into(), detail: e, action: None });
                }
            }
            // The firewall first, and "nothing looks wrong" not at all when it does
            if let Some(f) = firewall {
                findings.retain(|f| f.id != "fine");
                findings.insert(0, f);
            }
            for f in &findings {
                list.append(&w.finding_row(f, &dialog));
            }
            list.set_visible(true);
        });
    }

    fn finding_row(self: &Rc<Self>, f: &Finding, dialog: &adw::Dialog) -> gtk::ListBoxRow {
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(14).margin_bottom(14).margin_start(16).margin_end(16).build();
        column.append(&gtk::Label::builder().label(&f.title).wrap(true).xalign(0.0).css_classes(["heading"]).build());
        column.append(&gtk::Label::builder().label(&f.detail).wrap(true).xalign(0.0).css_classes(["dim-label"]).build());
        let row = gtk::ListBoxRow::builder().child(&column).activatable(false).selectable(false).build();
        let Some(action) = f.action.clone() else { return row };
        let label = match action {
            FindingAction::FinishSetup => t!("Finish Setup…"),
            FindingAction::ConnectByAddress { .. } => t!("Connect by Address…"),
            FindingAction::UsbConnect { .. } => t!("Connect by USB Cable…"),
        };
        let button = gtk::Button::builder().label(label).halign(gtk::Align::Start).margin_top(4).css_classes(["pill"]).build();
        let (w, d) = (self.clone(), dialog.clone());
        button.connect_clicked(move |_| {
            d.close();
            match action.clone() {
                FindingAction::FinishSetup => w.show_setup(),
                FindingAction::ConnectByAddress { address } => w.ask_pair_address(&address),
                FindingAction::UsbConnect { phone } => {
                    let Some(c) = w.client() else { return };
                    let call = w.bridge.call(&c, move |c| async move { c.usb_connect(&phone).await });
                    let w = w.clone();
                    glib::spawn_future_local(async move {
                        if let Err(e) = call.await {
                            w.toast_error(&e, None);
                        }
                    });
                }
            }
        });
        column.append(&button);
        row
    }
}
