//! A phone's deck, from this computer's side: the editor of its layouts in place of the
//! device's page, with Back, and the user's own commands.

mod art;
mod editor;
mod face;
mod fit;
mod grid;
mod model;
mod style;

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use linksy_i18n::t;
use linksy_ipc::linksy_core::Fingerprint;
use linksy_ipc::{ClientError, DeckCommand};

pub(super) use editor::Editor;

use super::Window;
use crate::fmt;

impl Window {
    /// The deck's page, in place of the device's, with Back.
    pub(super) fn show_deck(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move {
            let prefs = c.deck_prefs(id.to_string()).await?;
            // A service that doesn't keep layouts yet still lets the editor start
            let layouts = c.deck_layouts(id.to_string()).await.map_err(|e| e.to_string());
            Ok::<_, ClientError>((prefs, layouts))
        });
        glib::spawn_future_local(async move {
            match call.await {
                Ok((prefs, layouts)) if w.state.borrow().selected == Some(id) => {
                    let device = w.device(&id);
                    let name = device.as_ref().map(|d| d.device.name.clone()).unwrap_or_default();
                    let battery = device.and_then(|d| d.battery).map(|b| b.level);
                    let editor = Editor::new(&w, id, name.clone(), battery, prefs, layouts);
                    w.deck_page.set_child(Some(&editor.root));
                    {
                        let mut state = w.state.borrow_mut();
                        state.deck = Some(id);
                        state.deck_editor = Some(editor);
                    }
                    w.camera_title.set_title(t!("Deck"));
                    w.camera_title.set_subtitle(&name);
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
            self.state.borrow_mut().deck_editor = None;
            self.camera_title.set_subtitle(t!("Camera Preview"));
            self.content_header.set_show_title(false);
            self.camera_back.set_visible(false);
            self.page.menu.set_visible(true);
            self.deck_page.set_child(gtk::Widget::NONE);
        }
    }

    /// Back, asking first about changes that aren't saved.
    pub(super) fn leave_deck(self: &Rc<Self>) {
        let editor = self.state.borrow().deck_editor.clone();
        let Some(editor) = editor.filter(|e| e.dirty()) else {
            self.back_from_deck();
            return;
        };
        let ask = adw::AlertDialog::builder()
            .heading(t!("Save Changes?"))
            .body(t!("The deck for {device} has changes that aren't saved. Saved, they show on the phone.", device = editor.device()))
            .build();
        ask.add_responses(&[("cancel", t!("Cancel")), ("discard", t!("Discard")), ("save", t!("Save"))]);
        ask.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        ask.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        ask.set_default_response(Some("save"));
        ask.set_close_response("cancel");
        let w = self.clone();
        ask.connect_response(None, move |_, response| match response {
            "discard" => w.back_from_deck(),
            "save" => {
                let w = w.clone();
                editor.save(Some(Box::new(move || w.back_from_deck())));
            }
            _ => {}
        });
        ask.present(Some(&self.window));
    }

    fn back_from_deck(&self) {
        self.close_deck();
        self.render_page();
    }

    pub(super) fn save_deck_commands(self: &Rc<Self>, id: Fingerprint, commands: Vec<DeckCommand>) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move {
            c.set_deck_commands(commands).await?;
            c.deck_prefs(id.to_string()).await
        });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(prefs) if w.state.borrow().deck == Some(id) => {
                    let editor = w.state.borrow().deck_editor.clone();
                    if let Some(editor) = editor {
                        editor.set_prefs(prefs);
                    }
                }
                Ok(_) => {}
                Err(e) => w.toast(&fmt::capitalize(&e)),
            }
        });
    }

    /// A name and a command line, added to the user's `commands`.
    pub(super) fn ask_deck_command(self: &Rc<Self>, id: Fingerprint, commands: Vec<DeckCommand>) {
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
        "Time" => t!("Time").into(),
        "System" => t!("System").into(),
        "Network" => t!("Network").into(),
        "Power" => t!("Power").into(),
        "Commands" => t!("Commands").into(),
        other => other.into(),
    }
}
