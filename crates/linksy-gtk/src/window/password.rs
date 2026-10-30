//! Linksy's own password prompt, for pkexec where no polkit agent runs (Openbox, Hyprland,
//! Sway…): the password goes to pkexec on its terminal, nowhere else (desktop#67).

use adw::prelude::*;
use gtk::glib;
use linksy_i18n::t;
use linksy_setup::Prompt;

type Answer = std::sync::mpsc::Sender<Option<String>>;

/// For the thread that runs pkexec: asks in a dialog over `parent`, and waits for the answer
/// (None: cancelled).
pub(super) fn asker(parent: &gtk::Widget) -> impl FnMut(&Prompt) -> Option<String> + Send + 'static {
    let (tx, rx) = async_channel::unbounded::<(Prompt, Answer)>();
    let parent = parent.clone();
    glib::spawn_future_local(async move {
        while let Ok((prompt, answer)) = rx.recv().await {
            ask(&parent, &prompt, answer);
        }
    });
    move |prompt: &Prompt| {
        let (answer, reply) = std::sync::mpsc::channel();
        tx.send_blocking((prompt.clone(), answer)).ok()?;
        reply.recv().ok().flatten()
    }
}

fn ask(parent: &gtk::Widget, prompt: &Prompt, answer: Answer) {
    let entry = adw::PasswordEntryRow::builder().title(t!("Password")).activates_default(true).build();
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    list.append(&entry);
    let body = match (prompt.again, prompt.who.is_empty()) {
        (true, _) => t!("That password didn't work. Try again.").to_string(),
        (false, true) => t!("Linksy needs your password to make changes to this computer.").to_string(),
        (false, false) => t!("Linksy needs the password of {who} to make changes to this computer.", who = prompt.who),
    };
    let dialog = adw::AlertDialog::builder().heading(t!("Password")).body(body).extra_child(&list).build();
    dialog.add_response("cancel", t!("Cancel"));
    dialog.add_response("continue", t!("Continue"));
    dialog.set_response_appearance("continue", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("continue"));
    dialog.set_close_response("cancel");
    let answer = std::cell::Cell::new(Some(answer));
    dialog.connect_response(None, move |_, response| {
        if let Some(answer) = answer.take() {
            let _ = answer.send((response == "continue").then(|| entry.text().to_string()));
        }
        entry.set_text("");
    });
    dialog.present(Some(parent));
}
