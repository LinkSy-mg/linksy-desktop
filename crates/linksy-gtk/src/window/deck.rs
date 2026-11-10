//! A phone's deck, from this computer's side, in place of the device's page, with Back: what
//! the phone may do here (a switch each, by group), what its saved deck asked for, and the
//! user's own commands. The phone lays the buttons out and edits them (docs/protocol.md §4.26).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use linksy_i18n::t;
use linksy_ipc::linksy_core::Fingerprint;
use linksy_ipc::{CommandKind, DeckCommand, DeckOption, DeckPrefs};

use super::Window;
use crate::fmt;

/// The group a command with none shows in (the service names it so).
const DEFAULT_GROUP: &str = "Commands";

/// Told how saving the commands went: nothing, or why not.
type Done = Box<dyn FnOnce(Result<(), String>)>;

/// The page of one phone's deck.
pub(super) struct DeckView {
    win: Weak<Window>,
    phone: Fingerprint,
    device: String,
    /// What's shown: the page the window puts in place of the device's.
    pub root: adw::PreferencesPage,
    search: gtk::SearchEntry,
    /// Everything below the search box, rebuilt when what it shows changes.
    groups: RefCell<Vec<adw::PreferencesGroup>>,
    prefs: RefCell<DeckPrefs>,
    /// A switch put back after the service refused: its handler does nothing then.
    reverting: Cell<bool>,
}

impl DeckView {
    fn new(win: &Rc<Window>, phone: Fingerprint, device: String, prefs: DeckPrefs) -> Rc<Self> {
        let root = adw::PreferencesPage::new();
        let search = gtk::SearchEntry::builder().placeholder_text(t!("Search Actions")).hexpand(true).build();
        let finder = adw::PreferencesGroup::new();
        finder.add(&search);
        root.add(&finder);
        let view = Rc::new(Self { win: Rc::downgrade(win), phone, device, root, search, groups: RefCell::default(), prefs: RefCell::new(prefs), reverting: Cell::new(false) });
        let v = view.clone();
        view.search.connect_search_changed(move |_| v.render());
        view.render();
        view
    }

    /// The service's answer, shown (the search box stays as it was).
    fn set_prefs(self: &Rc<Self>, prefs: DeckPrefs) {
        *self.prefs.borrow_mut() = prefs;
        self.render();
    }

    /// The prefs again, from the service: after a phone saved its deck, say.
    pub fn reload(self: &Rc<Self>) {
        let Some(win) = self.win.upgrade() else { return };
        let Some(c) = win.client() else { return };
        let id = self.phone.to_string();
        let call = win.bridge.call(&c, move |c| async move { c.deck_prefs(id).await });
        let view = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(prefs) = call.await {
                view.set_prefs(prefs);
            }
        });
    }

    fn render(self: &Rc<Self>) {
        for group in self.groups.take() {
            self.root.remove(&group);
        }
        let prefs = self.prefs.borrow().clone();
        let query = self.search.text().to_lowercase();
        let mut made = vec![self.info_group(&prefs)];
        if query.is_empty() {
            made.extend(self.asked_group(&prefs));
        }
        made.extend(self.action_groups(&prefs, &query));
        if query.is_empty() {
            made.push(self.commands_group(&prefs));
        }
        for group in &made {
            self.root.add(group);
        }
        *self.groups.borrow_mut() = made;
    }

    // ---- what the phone saved ----

    fn info_group(&self, prefs: &DeckPrefs) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .description(t!("What {device} may do on this computer. Arrange its buttons on {device}: open its deck and tap the pencil.", device = self.device))
            .build();
        let (title, subtitle) = match &prefs.layout {
            Some(l) => {
                let saved = l.saved_at.map(when).map(|at| t!("Saved from {device} on {when}", device = self.device, when = at)).unwrap_or_else(|| t!("Made on this computer").to_string());
                (t!("Layout: {name}", name = l.name), t!("{saved}, version {version}", saved = saved, version = l.version))
            }
            None => (t!("No layout saved yet").to_string(), t!("Until {device} saves one, it shows a first layout of what is offered here.", device = self.device)),
        };
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&title)).subtitle(glib::markup_escape_text(&subtitle)).activatable(false).build();
        group.add(&row);
        group
    }

    /// What its saved tiles run that is off here: the phone asked, the user decides.
    fn asked_group(self: &Rc<Self>, prefs: &DeckPrefs) -> Option<adw::PreferencesGroup> {
        let asked: Vec<&DeckOption> = prefs.asked.iter().filter_map(|id| prefs.actions.iter().find(|a| a.action.id == *id)).collect();
        if asked.is_empty() {
            return None;
        }
        let group = adw::PreferencesGroup::builder()
            .title(t!("Asked for by {device}", device = self.device))
            .description(t!("Its deck has buttons for these, which stay off until you allow them."))
            .build();
        let all = gtk::Button::builder().label(t!("Allow All")).valign(gtk::Align::Center).css_classes(["flat"]).build();
        let (view, ids) = (self.clone(), asked.iter().map(|a| a.action.id.clone()).collect::<Vec<_>>());
        all.connect_clicked(move |_| view.set_many(ids.clone(), true));
        group.set_header_suffix(Some(&all));
        for option in asked {
            group.add(&self.action_row(option));
        }
        Some(group)
    }

    // ---- what the phone may do ----

    /// One list per group, in the order the service gives them; a search keeps the matches.
    fn action_groups(self: &Rc<Self>, prefs: &DeckPrefs, query: &str) -> Vec<adw::PreferencesGroup> {
        let mut titles: Vec<&str> = Vec::new();
        for option in &prefs.actions {
            if !titles.contains(&option.action.group.as_str()) {
                titles.push(&option.action.group);
            }
        }
        let mut made = Vec::new();
        for title in titles {
            let shown: Vec<&DeckOption> = prefs.actions.iter().filter(|a| a.action.group == title).filter(|a| query.is_empty() || a.action.name.to_lowercase().contains(query) || group_title(title).to_lowercase().contains(query)).collect();
            if shown.is_empty() {
                continue;
            }
            let group = adw::PreferencesGroup::builder().title(glib::markup_escape_text(&group_title(title))).build();
            // The ones that ask first keep their own switch: a button for all of them could do harm
            let plain: Vec<String> = shown.iter().filter(|a| a.action.confirm.is_none()).map(|a| a.action.id.clone()).collect();
            if plain.len() > 1 {
                let every = shown.iter().filter(|a| a.action.confirm.is_none()).all(|a| a.on);
                let all = gtk::Button::builder().label(if every { t!("Turn All Off") } else { t!("Turn All On") }).valign(gtk::Align::Center).css_classes(["flat"]).build();
                let view = self.clone();
                all.connect_clicked(move |_| view.set_many(plain.clone(), !every));
                group.set_header_suffix(Some(&all));
            }
            for option in shown {
                group.add(&self.action_row(option));
            }
            made.push(group);
        }
        made
    }

    fn action_row(self: &Rc<Self>, option: &DeckOption) -> adw::SwitchRow {
        let row = adw::SwitchRow::builder().title(glib::markup_escape_text(&option.action.name)).active(option.on).build();
        if option.action.confirm.is_some() {
            row.set_subtitle(&t!("Asks first on {device}", device = self.device));
        }
        let (view, id) = (self.clone(), option.action.id.clone());
        row.connect_active_notify(move |row| {
            if !view.reverting.get() {
                view.set_action(row, &id, row.is_active());
            }
        });
        row
    }

    /// Saved, or the switch goes back with why.
    fn set_action(self: &Rc<Self>, row: &adw::SwitchRow, id: &str, on: bool) {
        let Some(win) = self.win.upgrade() else { return };
        let Some(c) = win.client() else { return };
        let (phone, action) = (self.phone.to_string(), id.to_string());
        let call = win.bridge.call(&c, move |c| async move { c.set_deck_action(phone, action, on).await });
        let (view, row, id) = (self.clone(), row.clone(), id.to_string());
        glib::spawn_future_local(async move {
            match call.await {
                Ok(()) => {
                    let asked = {
                        let mut prefs = view.prefs.borrow_mut();
                        if let Some(o) = prefs.actions.iter_mut().find(|o| o.action.id == id) {
                            o.on = on;
                        }
                        let before = prefs.asked.len();
                        if on {
                            prefs.asked.retain(|a| *a != id);
                        }
                        prefs.asked.len() != before
                    };
                    if asked {
                        view.render();
                    }
                }
                Err(e) => {
                    view.reverting.set(true);
                    row.set_active(!on);
                    view.reverting.set(false);
                    if let Some(win) = view.win.upgrade() {
                        win.toast(&fmt::capitalize(&e));
                    }
                }
            }
        });
    }

    /// Several at once (a group's buttons, or all it asked for); then the list as it is.
    fn set_many(self: &Rc<Self>, ids: Vec<String>, on: bool) {
        let Some(win) = self.win.upgrade() else { return };
        let Some(c) = win.client() else { return };
        let phone = self.phone.to_string();
        let call = win.bridge.call(&c, move |c| async move {
            for id in ids {
                c.set_deck_action(phone.clone(), id, on).await?;
            }
            c.deck_prefs(phone).await
        });
        let view = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(prefs) => view.set_prefs(prefs),
                Err(e) => {
                    if let Some(win) = view.win.upgrade() {
                        win.toast(&fmt::capitalize(&e));
                    }
                    view.reload();
                }
            }
        });
    }

    // ---- the user's own commands ----

    fn commands_group(self: &Rc<Self>, prefs: &DeckPrefs) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(t!("Your Commands"))
            .description(t!("Run anything on this computer from {device}'s deck: a script, a shortcut, a switch for a VPN. Each runs as you, with sh -c. Turn it on under its group above.", device = self.device))
            .build();
        let add = gtk::Button::builder().label(t!("Add Command…")).valign(gtk::Align::Center).css_classes(["flat"]).build();
        let view = self.clone();
        add.connect_clicked(move |_| view.edit_command(None));
        group.set_header_suffix(Some(&add));
        for (i, c) in prefs.commands.iter().enumerate() {
            let line = if c.kind == CommandKind::Toggle { format!("{} / {}", c.command, if c.off.is_empty() { t!("same command") } else { c.off.as_str() }) } else { c.command.clone() };
            let row = adw::ActionRow::builder().title(glib::markup_escape_text(&c.name)).subtitle(glib::markup_escape_text(&one_line(&line))).subtitle_lines(1).activatable(true).build();
            let edit_view = self.clone();
            row.connect_activated(move |_| edit_view.edit_command(Some(i)));
            let remove = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text(t!("Remove Command")).valign(gtk::Align::Center).css_classes(["flat"]).build();
            remove.update_property(&[gtk::accessible::Property::Label(t!("Remove Command"))]);
            let remove_view = self.clone();
            remove.connect_clicked(move |_| remove_view.remove_command(i));
            row.add_suffix(&remove);
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            group.add(&row);
        }
        group
    }

    fn remove_command(self: &Rc<Self>, index: usize) {
        let mut list = self.prefs.borrow().commands.clone();
        if index < list.len() {
            list.remove(index);
            self.save_commands(list, None);
        }
    }

    /// All the commands go back (a new one without an id); then what is offered, with their
    /// actions. `then` hears how it went.
    fn save_commands(self: &Rc<Self>, commands: Vec<DeckCommand>, then: Option<Done>) {
        let Some(win) = self.win.upgrade() else { return };
        let Some(c) = win.client() else { return };
        let phone = self.phone.to_string();
        let call = win.bridge.call(&c, move |c| async move {
            c.set_deck_commands(commands).await?;
            c.deck_prefs(phone).await
        });
        let view = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(prefs) => {
                    view.set_prefs(prefs);
                    if let Some(then) = then {
                        then(Ok(()));
                    }
                }
                Err(e) => match then {
                    Some(then) => then(Err(fmt::capitalize(&e))),
                    None => {
                        if let Some(win) = view.win.upgrade() {
                            win.toast(&fmt::capitalize(&e));
                        }
                    }
                },
            }
        });
    }

    /// The form for a command: a new one, or the one at `index`.
    fn edit_command(self: &Rc<Self>, index: Option<usize>) {
        let Some(win) = self.win.upgrade() else { return };
        let prefs = self.prefs.borrow().clone();
        let current = index.and_then(|i| prefs.commands.get(i).cloned());
        let c = current.clone().unwrap_or_default();

        let name = adw::EntryRow::builder().title(t!("Name")).text(&c.name).build();
        let focus = name.clone();
        let kind = adw::ComboRow::builder().title(t!("Type")).model(&gtk::StringList::new(&[t!("Button"), t!("Switch")])).selected(u32::from(c.kind == CommandKind::Toggle)).build();
        let on = adw::EntryRow::builder().title(t!("Command")).text(&c.command).build();
        let off = adw::EntryRow::builder().title(t!("Command to turn it off (optional)")).text(&c.off).build();
        let state = adw::EntryRow::builder().title(t!("Check: succeeds while it is on")).text(&c.state).build();
        let result = gtk::Label::builder().wrap(true).xalign(0.0).selectable(true).css_classes(["dim-label", "monospace"]).visible(false).build();
        for row in [&on, &off, &state] {
            row.add_suffix(&test_button(&win, row, &result));
        }

        // Groups: the default, the ones the commands use, and a new one
        let mut groups: Vec<String> = vec![group_title(DEFAULT_GROUP)];
        let mut keys: Vec<String> = vec![String::new()];
        for g in prefs.commands.iter().map(|c| c.group.clone()).filter(|g| !g.is_empty()) {
            if !keys.contains(&g) {
                groups.push(g.clone());
                keys.push(g);
            }
        }
        let new_at = groups.len() as u32;
        groups.push(t!("New Group…").to_string());
        let labels: Vec<&str> = groups.iter().map(String::as_str).collect();
        let group = adw::ComboRow::builder().title(t!("Group")).subtitle(t!("The list it shows in on the phone.")).model(&gtk::StringList::new(&labels)).selected(keys.iter().position(|k| *k == c.group).unwrap_or(0) as u32).build();
        let new_group = adw::EntryRow::builder().title(t!("New group's name")).visible(false).build();

        let icons: Vec<String> = std::iter::once(String::new()).chain(prefs.icons.iter().cloned()).collect();
        let titles: Vec<String> = icons.iter().map(|k| if k.is_empty() { t!("Default").to_string() } else { icon_title(k) }).collect();
        let title_refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        let icon = adw::ComboRow::builder().title(t!("Icon")).model(&gtk::StringList::new(&title_refs)).selected(icons.iter().position(|k| *k == c.icon).unwrap_or(0) as u32).build();
        let ask = adw::SwitchRow::builder().title(t!("Ask First")).subtitle(t!("The phone asks \"Run it?\" before it does.")).active(c.confirm).build();

        let basics = adw::PreferencesGroup::builder().description(t!("Runs on this computer as you, with sh -c. A switch turns something on and off; its check says which it is, by succeeding while it is on.")).build();
        for row in [name.upcast_ref::<gtk::Widget>(), kind.upcast_ref(), on.upcast_ref(), off.upcast_ref(), state.upcast_ref()] {
            basics.add(row);
        }
        let looks = adw::PreferencesGroup::new();
        for row in [group.upcast_ref::<gtk::Widget>(), new_group.upcast_ref(), icon.upcast_ref(), ask.upcast_ref()] {
            looks.add(row);
        }
        let outcome = adw::PreferencesGroup::new();
        outcome.add(&result);
        let banner = adw::Banner::builder().revealed(false).build();
        let page = adw::PreferencesPage::new();
        page.add(&basics);
        page.add(&looks);
        page.add(&outcome);

        let save = gtk::Button::builder().label(if current.is_some() { t!("Save") } else { t!("Add") }).css_classes(["suggested-action"]).sensitive(false).build();
        let cancel = gtk::Button::builder().label(t!("Cancel")).build();
        let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
        header.pack_start(&cancel);
        header.pack_end(&save);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&banner);
        toolbar.set_content(Some(&page));
        let dialog = adw::Dialog::builder().title(if current.is_some() { t!("Edit Command") } else { t!("Add Command") }).content_width(480).content_height(640).child(&toolbar).build();

        // A switch needs its other lines; a new group its name; Save waits for what's needed
        let sync = {
            let (kind, off, state, group, new_group, name, on, save) = (kind.clone(), off.clone(), state.clone(), group.clone(), new_group.clone(), name.clone(), on.clone(), save.clone());
            move || {
                let switch = kind.selected() == 1;
                off.set_visible(switch);
                state.set_visible(switch);
                new_group.set_visible(group.selected() == new_at);
                let wants_group = group.selected() != new_at || !new_group.text().trim().is_empty();
                save.set_sensitive(!name.text().trim().is_empty() && !on.text().trim().is_empty() && (!switch || !state.text().trim().is_empty()) && wants_group);
            }
        };
        sync();
        for row in [&name, &on, &off, &state, &new_group] {
            let sync = sync.clone();
            row.connect_changed(move |_| sync());
        }
        for combo in [&kind, &group] {
            let sync = sync.clone();
            combo.connect_selected_notify(move |_| sync());
        }

        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
        let (view, d, banner2) = (self.clone(), dialog.clone(), banner.clone());
        let list = prefs.commands.clone();
        save.connect_clicked({
            let save = save.clone();
            move |_| {
                let switch = kind.selected() == 1;
                let chosen = if group.selected() == new_at { new_group.text().trim().to_string() } else { keys.get(group.selected() as usize).cloned().unwrap_or_default() };
                let command = DeckCommand {
                    id: current.as_ref().map(|c| c.id.clone()).unwrap_or_default(),
                    name: name.text().trim().into(),
                    command: on.text().trim().into(),
                    group: chosen,
                    icon: icons.get(icon.selected() as usize).cloned().unwrap_or_default(),
                    confirm: ask.is_active(),
                    kind: if switch { CommandKind::Toggle } else { CommandKind::Button },
                    off: if switch { off.text().trim().into() } else { String::new() },
                    state: if switch { state.text().trim().into() } else { String::new() },
                };
                let mut next = list.clone();
                match index {
                    Some(i) if i < next.len() => next[i] = command,
                    _ => next.push(command),
                }
                save.set_sensitive(false);
                let (d, banner, save) = (d.clone(), banner2.clone(), save.clone());
                view.save_commands(
                    next,
                    Some(Box::new(move |done| match done {
                        Ok(()) => {
                            d.close();
                        }
                        Err(why) => {
                            banner.set_title(&why);
                            banner.set_revealed(true);
                            save.set_sensitive(true);
                        }
                    })),
                );
            }
        });
        dialog.present(Some(&win.window));
        focus.grab_focus();
    }
}

/// A Test button beside a command line: runs it once on this computer and says how it went
/// in `result`.
fn test_button(win: &Rc<Window>, line: &adw::EntryRow, result: &gtk::Label) -> gtk::Button {
    let button = gtk::Button::builder().icon_name("media-playback-start-symbolic").tooltip_text(t!("Try It Now")).valign(gtk::Align::Center).css_classes(["flat"]).build();
    button.update_property(&[gtk::accessible::Property::Label(t!("Try It Now"))]);
    let (win, line, result) = (Rc::downgrade(win), line.clone(), result.clone());
    button.connect_clicked(move |button| {
        let (Some(win), text) = (win.upgrade(), line.text().trim().to_string()) else { return };
        let Some(c) = win.client() else { return };
        if text.is_empty() {
            return;
        }
        button.set_sensitive(false);
        result.set_visible(true);
        result.set_label(t!("Running…"));
        let call = win.bridge.call(&c, move |c| async move { c.test_deck_command(text).await });
        let (button, result) = (button.clone(), result.clone());
        glib::spawn_future_local(async move {
            let said = match call.await {
                Ok(run) if run.timed_out => t!("It was still running after 10 seconds, so it was stopped.").to_string(),
                Ok(run) => {
                    let code = run.code.map_or_else(|| t!("It didn't start.").to_string(), |c| t!("Ended with code {code}.", code = c));
                    if run.output.is_empty() { code } else { format!("{code}\n{}", run.output) }
                }
                Err(e) => fmt::capitalize(&e),
            };
            result.set_label(&said);
            button.set_sensitive(true);
        });
    });
    button
}

impl Window {
    /// The deck's page, in place of the device's, with Back.
    pub(super) fn show_deck(self: &Rc<Self>, id: Fingerprint) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, move |c| async move { c.deck_prefs(id.to_string()).await });
        glib::spawn_future_local(async move {
            match call.await {
                Ok(prefs) if w.state.borrow().selected == Some(id) => {
                    let name = w.device(&id).map(|d| d.device.name.clone()).unwrap_or_default();
                    let view = DeckView::new(&w, id, name.clone(), prefs);
                    w.deck_page.set_child(Some(&view.root));
                    {
                        let mut state = w.state.borrow_mut();
                        state.deck = Some(id);
                        state.deck_view = Some(view);
                    }
                    // Development aid (screenshots): LINKSY_SCREENSHOT_DECK_DIALOG=add or edit (the second command)
                    let (open, shown) = (std::env::var("LINKSY_SCREENSHOT_DECK_DIALOG").ok(), w.state.borrow().deck_view.clone());
                    if let (Some(open), Some(view)) = (open, shown) {
                        glib::timeout_add_local_once(Duration::from_millis(600), move || view.edit_command(if open == "edit" { Some(1) } else { None }));
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
            self.state.borrow_mut().deck_view = None;
            self.camera_title.set_subtitle(t!("Camera Preview"));
            self.content_header.set_show_title(false);
            self.camera_back.set_visible(false);
            self.page.menu.set_visible(true);
            self.deck_page.set_child(gtk::Widget::NONE);
        }
    }

    pub(super) fn back_from_deck(&self) {
        self.close_deck();
        self.render_page();
    }

    /// A phone saved its deck: what it asked for may have changed, so the page asks again
    /// once the service has had time to keep it.
    pub(super) fn deck_saved_by(&self, phone: &Fingerprint) {
        if self.state.borrow().deck != Some(*phone) {
            return;
        }
        let Some(view) = self.state.borrow().deck_view.clone() else { return };
        glib::timeout_add_local_once(Duration::from_millis(700), move || view.reload());
    }
}

/// The service names its groups in English; each shows in the app's language. One of the
/// user's own shows as they typed it.
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

/// The phone's icon names, as words.
fn icon_title(key: &str) -> String {
    match key {
        "play_pause" => t!("Play or Pause").into(),
        "play" => t!("Play").into(),
        "pause" => t!("Pause").into(),
        "previous" => t!("Previous").into(),
        "next" => t!("Next").into(),
        "speaker" => t!("Speaker").into(),
        "speaker_off" => t!("Speaker Off").into(),
        "microphone" => t!("Microphone").into(),
        "microphone_off" => t!("Microphone Off").into(),
        "lock" => t!("Lock").into(),
        "terminal" => t!("Terminal").into(),
        "lightning" => t!("Lightning").into(),
        "star" => t!("Star").into(),
        "moon" => t!("Moon").into(),
        "sun" => t!("Sun").into(),
        "house" => t!("House").into(),
        "bell" => t!("Bell").into(),
        "code" => t!("Code").into(),
        "power" => t!("Power").into(),
        "rocket" => t!("Rocket").into(),
        "game" => t!("Game").into(),
        "camera" => t!("Camera").into(),
        "heart" => t!("Heart").into(),
        "coffee" => t!("Coffee").into(),
        "music" => t!("Music").into(),
        "monitor" => t!("Monitor").into(),
        "broadcast" => t!("Broadcast").into(),
        "flag" => t!("Flag").into(),
        "folder" => t!("Folder").into(),
        "globe" => t!("Globe").into(),
        "cloud" => t!("Cloud").into(),
        "timer" => t!("Timer").into(),
        other => other.replace('_', " "),
    }
}

/// A command line as one line, for a row's subtitle.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A moment, as the person's locale writes a date and a time.
fn when(ms: u64) -> String {
    glib::DateTime::from_unix_local((ms / 1000) as i64).ok().and_then(|d| d.format("%x %H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}
