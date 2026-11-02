//! A phone's deck, from this computer's side: which actions the phone may run here (a switch
//! each, by group), and the user's own commands. The phone lays the buttons out.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use linksy_i18n::t;
use linksy_ipc::linksy_core::Fingerprint;
use linksy_ipc::{DeckCommand, DeckPrefs};

use super::Window;
use crate::fmt;

impl Window {
    /// The deck's page, in place of the device's, with Back.
    pub(super) fn show_deck(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.deck_prefs(id.to_string()).await });
        glib::spawn_future_local(async move {
            match call.await {
                Ok(prefs) if w.state.borrow().selected == Some(id) => {
                    w.fill_deck(id, prefs);
                    w.state.borrow_mut().deck = Some(id);
                    w.camera_title.set_title(t!("Deck"));
                    w.camera_title.set_subtitle(&w.device(&id).map(|d| d.device.name).unwrap_or_default());
                    w.content_header.set_show_title(true);
                    w.camera_back.set_visible(true);
                    w.page.menu.set_visible(false);
                    w.content_stack.set_visible_child_name("deck");
                }
                Ok(_) => {}
                Err(e) => w.toast_error(&e, Some(id)),
            }
        });
    }

    /// Back from the deck's page, if it shows.
    pub(super) fn close_deck(&self) {
        if self.state.borrow_mut().deck.take().is_some() {
            self.camera_title.set_subtitle(t!("Camera Preview"));
            self.content_header.set_show_title(false);
            self.camera_back.set_visible(false);
            self.page.menu.set_visible(true);
            self.deck_page.set_child(gtk::Widget::NONE);
        }
    }

    /// The page, made again whenever the commands change.
    fn fill_deck(self: &Rc<Self>, id: Fingerprint, prefs: DeckPrefs) {
        let name = self.device(&id).map(|d| d.device.name).unwrap_or_default();
        let page = adw::PreferencesPage::builder()
            .description(glib::markup_escape_text(&t!("What {device} may do on this computer. Arrange the buttons on {device}.", device = name)))
            .build();
        let mut groups: Vec<(String, adw::PreferencesGroup)> = Vec::new();
        for option in prefs.actions {
            let group = match groups.iter().find(|(g, _)| *g == option.action.group) {
                Some((_, group)) => group.clone(),
                None => {
                    let group = adw::PreferencesGroup::builder().title(group_title(&option.action.group)).build();
                    groups.push((option.action.group.clone(), group.clone()));
                    group
                }
            };
            let row = adw::SwitchRow::builder().title(glib::markup_escape_text(&option.action.name)).active(option.on).build();
            let (w, action, reverting) = (self.clone(), option.action.id, Rc::new(Cell::new(false)));
            row.connect_active_notify(move |row| {
                if reverting.get() {
                    return;
                }
                let Some(c) = w.client() else { return };
                let (on, action) = (row.is_active(), action.clone());
                let call = w.bridge.call(&c, move |c| async move { c.set_deck_action(id.to_string(), action, on).await });
                let (w, row, reverting) = (w.clone(), row.clone(), reverting.clone());
                glib::spawn_future_local(async move {
                    if let Err(e) = call.await {
                        w.toast(&fmt::capitalize(&e));
                        reverting.set(true);
                        row.set_active(!on);
                        reverting.set(false);
                    }
                });
            });
            group.add(&row);
        }
        for (_, group) in &groups {
            page.add(group);
        }

        let mine = adw::PreferencesGroup::builder()
            .title(t!("Your Commands"))
            .description(glib::markup_escape_text(&t!("Each runs on this computer as you, with sh -c. Turn it on under Commands for {device}.", device = name)))
            .build();
        for c in &prefs.commands {
            let row = adw::ActionRow::builder().title(glib::markup_escape_text(&c.name)).subtitle(glib::markup_escape_text(&c.command)).subtitle_lines(2).build();
            let delete = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text(t!("Remove Command")).valign(gtk::Align::Center).css_classes(["flat"]).build();
            let rest: Vec<DeckCommand> = prefs.commands.iter().filter(|o| o.id != c.id).cloned().collect();
            let w = self.clone();
            delete.connect_clicked(move |_| w.save_deck_commands(id, rest.clone()));
            row.add_suffix(&delete);
            mine.add(&row);
        }
        let add = adw::ActionRow::builder().title(t!("Add Command…")).activatable(true).build();
        add.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
        let (w, commands) = (self.clone(), prefs.commands);
        add.connect_activated(move |_| w.ask_deck_command(id, commands.clone()));
        mine.add(&add);
        page.add(&mine);

        // Where it was, after the commands changed
        let scroller = |p: &gtk::Widget| p.first_child().and_downcast::<gtk::ScrolledWindow>();
        let y = self.deck_page.child().as_ref().and_then(scroller).map(|s| s.vadjustment().value());
        self.deck_page.set_child(Some(&page));
        if let (Some(y), Some(s)) = (y, scroller(page.upcast_ref())) {
            glib::idle_add_local_once(move || s.vadjustment().set_value(y));
        }
    }

    fn save_deck_commands(self: &Rc<Self>, id: Fingerprint, commands: Vec<DeckCommand>) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move {
            c.set_deck_commands(commands).await?;
            c.deck_prefs(id.to_string()).await
        });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(prefs) if w.state.borrow().deck == Some(id) => w.fill_deck(id, prefs),
                Ok(_) => {}
                Err(e) => w.toast(&fmt::capitalize(&e)),
            }
        });
    }

    /// A name and a command line, added to the user's `commands`.
    fn ask_deck_command(self: &Rc<Self>, id: Fingerprint, commands: Vec<DeckCommand>) {
        let name = adw::EntryRow::builder().title(t!("Name")).activates_default(true).build();
        let line = adw::EntryRow::builder().title(t!("Command")).activates_default(true).build();
        let fields = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
        fields.append(&name);
        fields.append(&line);
        let ask = adw::AlertDialog::builder().heading(t!("Add Command")).body(t!("A button for the deck that runs this on this computer, as if typed in a terminal.")).extra_child(&fields).build();
        ask.add_responses(&[("cancel", t!("Cancel")), ("add", t!("Add"))]);
        ask.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        ask.set_default_response(Some("add"));
        ask.set_close_response("cancel");
        ask.set_response_enabled("add", false);
        let check = {
            let (ask, name, line) = (ask.clone(), name.clone(), line.clone());
            move || ask.set_response_enabled("add", !name.text().trim().is_empty() && !line.text().trim().is_empty())
        };
        let c1 = check.clone();
        name.connect_changed(move |_| c1());
        line.connect_changed(move |_| check());
        let (w, field) = (self.clone(), name.clone());
        ask.connect_response(Some("add"), move |_, _| {
            let mut next = commands.clone();
            next.push(DeckCommand { id: String::new(), name: field.text().trim().into(), command: line.text().trim().into() });
            w.save_deck_commands(id, next);
        });
        ask.present(Some(&self.window));
        ask.set_focus(Some(&name));
    }
}

/// The service names its groups in English; each shows in the app's language.
fn group_title(group: &str) -> String {
    match group {
        "Media" => t!("Media").into(),
        "Sound" => t!("Sound").into(),
        "System" => t!("System").into(),
        "Commands" => t!("Commands").into(),
        other => other.into(),
    }
}
