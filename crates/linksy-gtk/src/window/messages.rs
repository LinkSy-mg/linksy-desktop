//! A phone's Inbox tab: its conversations, made from its message notifications, and its
//! other notifications on the left; the open conversation on the right with a field to
//! answer. One side at a time when the page is narrow. Rows are built only while the tab
//! shows.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use linksy_ipc::linksy_core::Fingerprint;
use linksy_ipc::{Alert, Conversation, InboxMessage};
use linksy_i18n::t;

use super::Window;
use super::page::Tab;
use crate::fmt;

#[derive(Clone, Copy, PartialEq)]
enum Show {
    All,
    Chats,
    Texts,
    Mail,
    Other,
}

impl Show {
    fn conversation(self, kind: &str) -> bool {
        match self {
            Show::All => true,
            Show::Chats => kind == "chat" || kind == "social",
            Show::Texts => kind == "sms",
            Show::Mail => kind == "mail",
            Show::Other => false,
        }
    }
}

pub struct MessagesView {
    pub root: gtk::Box,
    list_side: gtk::Box,
    divider: gtk::Separator,
    thread_side: gtk::Box,
    search: gtk::SearchEntry,
    show: Cell<Show>,
    pills: Vec<(Show, gtk::ToggleButton)>,
    list: gtk::ListBox,
    alerts_head: gtk::Box,
    alerts: gtk::ListBox,
    clear: gtk::Button,
    empty: gtk::Label,
    back: gtk::Button,
    avatar: adw::Avatar,
    title: gtk::Label,
    subtitle: gtk::Label,
    bubbles: gtk::Box,
    scroller: gtk::ScrolledWindow,
    entry: gtk::Entry,
    send: gtk::Button,
    compose: gtk::Box,
    no_reply: gtk::Label,
    thread_stack: gtk::Stack,
    narrow: Cell<bool>,
    state: RefCell<ViewState>,
}

#[derive(Default)]
struct ViewState {
    phone: Option<Fingerprint>,
    /// Newest first, like the rows.
    conversations: Vec<Conversation>,
    alerts: Vec<Alert>,
    open: Option<String>,
    built: bool,
}

impl MessagesView {
    pub fn new() -> Rc<Self> {
        let search = gtk::SearchEntry::builder().placeholder_text(t!("Search")).build();
        let chips = gtk::Box::builder().spacing(6).build();
        let mut pills = Vec::new();
        let mut first: Option<gtk::ToggleButton> = None;
        for (label, which) in [(t!("All"), Show::All), (t!("Chats"), Show::Chats), (t!("Texts"), Show::Texts), (t!("Mail"), Show::Mail), (t!("Other"), Show::Other)] {
            let b = gtk::ToggleButton::builder().label(label).css_classes(["filter-pill"]).active(which == Show::All).build();
            b.set_group(first.as_ref());
            first.get_or_insert(b.clone());
            chips.append(&b);
            pills.push((which, b));
        }
        let chips_scroller = gtk::ScrolledWindow::builder().child(&chips).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).build();

        let list = gtk::ListBox::builder().css_classes(["inbox-list"]).build();
        let alerts_head = gtk::Box::builder().spacing(6).margin_top(14).margin_start(10).visible(false).build();
        alerts_head.append(&gtk::Label::builder().label(t!("Other Notifications")).xalign(0.0).hexpand(true).css_classes(["caption-heading", "dim-label"]).build());
        let clear = gtk::Button::builder().label(t!("Clear All")).css_classes(["flat", "small"]).build();
        alerts_head.append(&clear);
        let alerts = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["inbox-list"]).build();
        let empty = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label"]).margin_top(24).margin_start(12).margin_end(12).visible(false).build();
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        column.append(&list);
        column.append(&alerts_head);
        column.append(&alerts);
        column.append(&empty);
        let list_scroller = gtk::ScrolledWindow::builder().child(&column).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let list_side = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).width_request(300).margin_end(12).build();
        list_side.append(&search);
        list_side.append(&chips_scroller);
        list_side.append(&list_scroller);

        let back = gtk::Button::builder().icon_name("go-previous-symbolic").tooltip_text(t!("Back")).css_classes(["flat", "circular"]).valign(gtk::Align::Center).visible(false).build();
        let avatar = adw::Avatar::builder().size(40).show_initials(true).build();
        let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build();
        let subtitle = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label"]).build();
        let titles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
        titles.append(&title);
        titles.append(&subtitle);
        let head = gtk::Box::builder().spacing(12).margin_bottom(10).build();
        head.append(&back);
        head.append(&avatar);
        head.append(&titles);

        let bubbles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(12).margin_bottom(12).valign(gtk::Align::End).build();
        let scroller = gtk::ScrolledWindow::builder().child(&bubbles).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let entry = gtk::Entry::builder().placeholder_text(t!("Reply")).hexpand(true).css_classes(["reply-entry"]).build();
        let send = gtk::Button::builder().icon_name("mail-send-symbolic").tooltip_text(t!("Send")).css_classes(["circular", "suggested-action"]).valign(gtk::Align::Center).sensitive(false).build();
        let compose = gtk::Box::builder().spacing(8).margin_top(10).build();
        compose.append(&entry);
        compose.append(&send);
        let no_reply = gtk::Label::builder()
            .label(t!("This one can't be answered from here. Answer it on the phone."))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .margin_top(12)
            .visible(false)
            .build();
        let thread = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        thread.append(&head);
        thread.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        thread.append(&scroller);
        thread.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        thread.append(&compose);
        thread.append(&no_reply);
        let thread_stack = gtk::Stack::new();
        thread_stack.add_named(&gtk::Label::builder().label(t!("No Conversation Selected")).vexpand(true).css_classes(["dim-label"]).build(), Some("empty"));
        thread_stack.add_named(&thread, Some("open"));
        let thread_side = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).margin_start(18).build();
        thread_side.append(&thread_stack);

        let divider = gtk::Separator::new(gtk::Orientation::Vertical);
        let root = gtk::Box::builder().height_request(560).css_classes(["inbox"]).build();
        root.append(&list_side);
        root.append(&divider);
        root.append(&thread_side);
        let s = send.clone();
        entry.connect_changed(move |e| s.set_sensitive(!e.text().trim().is_empty()));
        Rc::new(Self {
            root,
            list_side,
            divider,
            thread_side,
            search,
            show: Cell::new(Show::All),
            pills,
            list,
            alerts_head,
            alerts,
            clear,
            empty,
            back,
            avatar,
            title,
            subtitle,
            bubbles,
            scroller,
            entry,
            send,
            compose,
            no_reply,
            thread_stack,
            narrow: Cell::new(false),
            state: RefCell::default(),
        })
    }

    /// Narrow: the list, or the open conversation with Back.
    pub fn set_narrow(&self, narrow: bool) {
        self.narrow.set(narrow);
        self.layout();
    }

    fn layout(&self) {
        let open = self.state.borrow().open.is_some();
        let narrow = self.narrow.get();
        self.list_side.set_visible(!narrow || !open);
        self.list_side.set_hexpand(narrow);
        self.divider.set_visible(!narrow);
        self.thread_side.set_visible(!narrow || open);
        self.thread_side.set_margin_start(if narrow { 0 } else { 18 });
        self.back.set_visible(narrow);
    }

    fn clear_rows(&self) {
        for list in [&self.list, &self.alerts] {
            while let Some(row) = list.first_child() {
                list.remove(&row);
            }
        }
        while let Some(child) = self.bubbles.first_child() {
            self.bubbles.remove(&child);
        }
    }

    /// Which rows show, by the filter and the search.
    fn apply_filter(&self) {
        let show = self.show.get();
        let needle = self.search.text().to_lowercase();
        let matches = |text: &str| needle.is_empty() || text.to_lowercase().contains(&needle);
        let state = self.state.borrow();
        let mut any = false;
        let mut row = self.list.first_child();
        while let Some(r) = row {
            let c = state.conversations.iter().find(|c| c.id == r.widget_name().as_str());
            let visible = c.is_some_and(|c| show.conversation(&c.kind) && (matches(&c.title) || matches(&preview(c))));
            r.set_visible(visible);
            any |= visible;
            row = r.next_sibling();
        }
        let mut alerts = false;
        let mut row = self.alerts.first_child();
        while let Some(r) = row {
            let a = state.alerts.iter().find(|a| a.key == r.widget_name().as_str());
            let visible = matches!(show, Show::All | Show::Other) && a.is_some_and(|a| matches(&a.title) || matches(&a.body) || matches(&a.app_name));
            r.set_visible(visible);
            alerts |= visible;
            row = r.next_sibling();
        }
        self.alerts_head.set_visible(alerts);
        self.clear.set_visible(needle.is_empty());
        let nothing = !any && !alerts;
        self.empty.set_visible(nothing);
        if nothing {
            self.empty.set_label(if state.conversations.is_empty() && state.alerts.is_empty() {
                t!("New messages and notifications from the phone show up here.")
            } else {
                t!("Nothing matches.")
            });
        }
    }

    fn row_of(&self, id: &str) -> Option<gtk::ListBoxRow> {
        let mut child = self.list.first_child();
        while let Some(c) = child {
            if c.widget_name() == id {
                return c.downcast().ok();
            }
            child = c.next_sibling();
        }
        None
    }

    fn select_open(&self) {
        let open = self.state.borrow().open.clone();
        match open.and_then(|id| self.row_of(&id)) {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
    }

    fn conversation(&self, id: &str) -> Option<Conversation> {
        self.state.borrow().conversations.iter().find(|c| c.id == id).cloned()
    }

    fn scroll_to_end(&self) {
        let adj = self.scroller.vadjustment();
        glib::idle_add_local_once(move || adj.set_value(adj.upper()));
    }

    /// Unread conversations and other notifications, for the tab's badge.
    fn count(&self) -> u32 {
        let state = self.state.borrow();
        state.conversations.iter().map(|c| c.unread.min(1)).sum::<u32>() + state.alerts.len() as u32
    }
}

impl Window {
    pub(super) fn setup_messages(self: &Rc<Self>) {
        let m = &self.messages;
        let w = Rc::downgrade(self);
        m.list.connect_row_activated(move |_, row| {
            let Some(w) = w.upgrade() else { return };
            let id = row.widget_name().to_string();
            w.open_conversation(id);
        });
        let w = Rc::downgrade(self);
        let send = move || {
            if let Some(w) = w.upgrade() {
                w.send_reply();
            }
        };
        let s = send.clone();
        m.send.connect_clicked(move |_| s());
        m.entry.connect_activate(move |_| send());
        let w = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |win| {
            if let Some(w) = w.upgrade().filter(|_| win.is_active()) {
                w.read_open();
            }
        });
        let w = Rc::downgrade(self);
        m.search.connect_search_changed(move |_| {
            if let Some(w) = w.upgrade() {
                w.messages.apply_filter();
            }
        });
        for (which, b) in &m.pills {
            let (w, which) = (Rc::downgrade(self), *which);
            b.connect_toggled(move |b| {
                if let Some(w) = w.upgrade().filter(|_| b.is_active()) {
                    w.messages.show.set(which);
                    w.messages.apply_filter();
                }
            });
        }
        let w = Rc::downgrade(self);
        m.back.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.messages.state.borrow_mut().open = None;
                w.messages.select_open();
                w.messages.thread_stack.set_visible_child_name("empty");
                w.messages.layout();
            }
        });
        let w = Rc::downgrade(self);
        m.clear.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.dismiss_alert(None);
            }
        });
    }

    /// The phone's inbox: what's in it, for the tab's badge, and its rows when the tab shows.
    pub(super) fn load_inbox(self: &Rc<Self>, phone: Fingerprint) {
        let Some(c) = self.client() else { return };
        if self.messages.state.borrow().phone != Some(phone) {
            *self.messages.state.borrow_mut() = ViewState { phone: Some(phone), ..Default::default() };
            self.messages.clear_rows();
            self.messages.thread_stack.set_visible_child_name("empty");
            self.messages.layout();
        }
        let call = self.bridge.call(&c, |c| async move {
            let inbox = c.inbox().await?;
            // An older service has no other notifications to give.
            let alerts = c.alerts().await.unwrap_or_default();
            Ok((inbox, alerts))
        });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            if w.messages.state.borrow().phone != Some(phone) {
                return;
            }
            match result {
                Ok((inbox, alerts)) => {
                    let id = phone.to_string();
                    {
                        let mut state = w.messages.state.borrow_mut();
                        state.conversations = inbox.into_iter().filter(|c| c.phone == id).collect();
                        state.alerts = alerts.into_iter().filter(|a| a.phone == id).collect();
                    }
                    w.page.set_inbox_count(w.messages.count());
                    if w.page.tab() == Tab::Inbox {
                        w.build_inbox();
                    } else {
                        w.messages.state.borrow_mut().built = false;
                        w.messages.clear_rows();
                    }
                }
                Err(e) => {
                    w.messages.empty.set_label(&fmt::capitalize(&e));
                    w.messages.empty.set_visible(true);
                }
            }
        });
    }

    fn build_inbox(self: &Rc<Self>) {
        let m = &self.messages;
        m.clear_rows();
        let (conversations, alerts) = {
            let state = m.state.borrow();
            (state.conversations.clone(), state.alerts.clone())
        };
        for c in &conversations {
            m.list.append(&self.conversation_row(c));
        }
        for a in &alerts {
            m.alerts.append(&self.alert_row(a));
        }
        m.state.borrow_mut().built = true;
        m.select_open();
        m.apply_filter();
        let open = m.state.borrow().open.clone();
        match open.and_then(|id| m.conversation(&id)) {
            Some(c) => self.load_thread(&c),
            None => {
                // Development aid (screenshots): LINKSY_SCREENSHOT_THREAD=<title> opens one.
                if let Ok(title) = std::env::var("LINKSY_SCREENSHOT_THREAD")
                    && let Some(id) = conversations.iter().find(|c| c.title == title).map(|c| c.id.clone())
                {
                    let w = self.clone();
                    glib::idle_add_local_once(move || w.open_conversation(id));
                }
            }
        }
    }

    /// The Inbox tab shows: its rows, built now if they weren't.
    pub(super) fn inbox_shown(self: &Rc<Self>, phone: Fingerprint) {
        let loaded = self.messages.state.borrow().phone == Some(phone);
        if loaded && !self.messages.state.borrow().built {
            self.build_inbox();
        }
        self.load_inbox(phone);
    }

    /// Open the phone's inbox, in its page.
    pub(super) fn open_messages(self: &Rc<Self>, phone: Fingerprint) {
        self.select(Some(phone));
        self.page.select_tab(Tab::Inbox);
        self.show_content();
    }

    fn open_conversation(self: &Rc<Self>, id: String) {
        let Some(c) = self.messages.conversation(&id) else { return };
        let m = &self.messages;
        m.state.borrow_mut().open = Some(id.clone());
        m.title.set_label(&c.title);
        m.subtitle.set_label(&c.app_name);
        m.avatar.set_text(Some(&c.title));
        m.avatar.set_show_initials(!c.group);
        m.avatar.set_icon_name(c.group.then_some("system-users-symbolic"));
        m.avatar.set_custom_image(c.picture.as_ref().and_then(|p| gdk::Texture::from_filename(p).ok()).as_ref());
        m.select_open();
        self.show_reply(&c);
        m.thread_stack.set_visible_child_name("open");
        m.layout();
        if c.can_reply {
            m.entry.grab_focus();
        }
        self.mark_read(&c);
        self.load_thread(&c);
    }

    fn show_reply(&self, c: &Conversation) {
        self.messages.compose.set_visible(c.can_reply);
        self.messages.no_reply.set_visible(!c.can_reply);
        self.messages.entry.set_placeholder_text(Some(&t!("Reply on {app}", app = c.app_name)));
    }

    fn mark_read(&self, c: &Conversation) {
        let Some(client) = self.client() else { return };
        if c.unread == 0 {
            return;
        }
        let id = c.id.clone();
        let read = self.bridge.call(&client, move |c| async move { c.inbox_read(id).await });
        glib::spawn_future_local(async move {
            let _ = read.await;
        });
    }

    /// Back to the window with a conversation open: what came in meanwhile is read now.
    fn read_open(&self) {
        if self.page.tab() != Tab::Inbox {
            return;
        }
        let open = self.messages.state.borrow().open.clone();
        if let Some(c) = open.and_then(|id| self.messages.conversation(&id)) {
            self.mark_read(&c);
        }
    }

    fn load_thread(self: &Rc<Self>, c: &Conversation) {
        let Some(client) = self.client() else { return };
        let id = c.id.clone();
        let i = id.clone();
        let call = self.bridge.call(&client, move |c| async move { c.inbox_thread(i).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            if w.messages.state.borrow().open.as_deref() != Some(id.as_str()) {
                return;
            }
            match result {
                Ok(messages) => w.show_bubbles(&id, &messages),
                Err(e) => w.toast(&fmt::capitalize(&e)),
            }
        });
    }

    fn show_bubbles(self: &Rc<Self>, id: &str, messages: &[InboxMessage]) {
        let bubbles = &self.messages.bubbles;
        while let Some(child) = bubbles.first_child() {
            bubbles.remove(&child);
        }
        let group = self.messages.conversation(id).is_some_and(|c| c.group);
        let mut day = String::new();
        for m in messages {
            let today = super::page::day_label(m.at_ms);
            if today != day {
                bubbles.append(&gtk::Label::builder().label(&today).css_classes(["caption-heading", "dim-label"]).margin_top(10).margin_bottom(4).build());
                day = today;
            }
            let line = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).halign(if m.mine { gtk::Align::End } else { gtk::Align::Start }).build();
            if group && !m.mine
                && let Some(sender) = &m.sender
            {
                line.append(&gtk::Label::builder().label(sender).xalign(0.0).margin_start(12).css_classes(["caption-heading", "dim-label"]).build());
            }
            line.append(
                &gtk::Label::builder()
                    .label(&m.text)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .xalign(0.0)
                    .selectable(true)
                    .max_width_chars(48)
                    .css_classes(["bubble", if m.mine { "mine" } else { "theirs" }])
                    .build(),
            );
            if let Some(code) = &m.code {
                line.append(&self.copy_chip(code));
            }
            line.append(&gtk::Label::builder().label(super::page::time_label(m.at_ms)).xalign(if m.mine { 1.0 } else { 0.0 }).margin_start(12).margin_end(12).css_classes(["caption", "dim-label", "numeric"]).build());
            bubbles.append(&line);
        }
        self.messages.scroll_to_end();
    }

    /// "Copy 482913", as a chip.
    fn copy_chip(self: &Rc<Self>, code: &str) -> gtk::Button {
        let content = adw::ButtonContent::builder().icon_name("edit-copy-symbolic").label(t!("Copy {code}", code = code)).build();
        let copy = gtk::Button::builder().child(&content).halign(gtk::Align::Start).css_classes(["code-chip"]).build();
        let (w, code) = (Rc::downgrade(self), code.to_string());
        copy.connect_clicked(move |b| {
            b.clipboard().set_text(&code);
            if let Some(w) = w.upgrade() {
                w.toast(&t!("{code} copied", code = code));
            }
        });
        copy
    }

    fn send_reply(self: &Rc<Self>) {
        let text = self.messages.entry.text().trim().to_string();
        let open = self.messages.state.borrow().open.clone();
        let (Some(c), Some(id)) = (self.client(), open) else { return };
        if text.is_empty() {
            return;
        }
        self.messages.compose.set_sensitive(false);
        let call = self.bridge.call(&c, move |c| async move { c.inbox_reply(id, text).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            w.messages.compose.set_sensitive(true);
            match result {
                Ok(()) => {
                    w.messages.entry.set_text("");
                    w.messages.entry.grab_focus();
                }
                Err(e) => w.toast(&t!("Not sent: {error}", error = fmt::capitalize(&e))),
            }
        });
    }

    /// Dismiss one of the phone's other notifications there, or all of them.
    fn dismiss_alert(self: &Rc<Self>, key: Option<String>) {
        let (Some(c), Some(phone)) = (self.client(), self.messages.state.borrow().phone) else { return };
        let call = self.bridge.call(&c, move |c| async move { c.dismiss_alert(phone.to_string(), key).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = call.await {
                w.toast(&fmt::capitalize(&e));
            }
            w.load_inbox(phone);
        });
    }

    /// A conversation changed: a message came in, was read or answered.
    pub(super) fn inbox_changed(self: &Rc<Self>, c: Conversation) {
        let phone = self.messages.state.borrow().phone;
        if phone.is_none_or(|p| p.to_string() != c.phone) {
            return;
        }
        let open = self.messages.state.borrow().open.as_deref() == Some(c.id.as_str());
        let at = {
            let mut state = self.messages.state.borrow_mut();
            // Read or answerable again, nothing newer: it keeps its place.
            let same = state.conversations.iter().position(|o| o.id == c.id && o.at_ms == c.at_ms);
            state.conversations.retain(|o| o.id != c.id);
            let at = same.unwrap_or_else(|| state.conversations.iter().position(|o| o.at_ms <= c.at_ms).unwrap_or(state.conversations.len()));
            state.conversations.insert(at, c.clone());
            at
        };
        self.page.set_inbox_count(self.messages.count());
        if !self.messages.state.borrow().built {
            return;
        }
        if let Some(old) = self.messages.row_of(&c.id) {
            self.messages.list.remove(&old);
        }
        self.messages.list.insert(&self.conversation_row(&c), at as i32);
        self.messages.select_open();
        self.messages.apply_filter();
        if open {
            self.show_reply(&c);
            if self.window.is_active() {
                self.mark_read(&c);
            }
            self.load_thread(&c);
        }
    }

    /// A notification came or went on the selected phone: its other notifications again.
    pub(super) fn alerts_changed(self: &Rc<Self>, phone: &Fingerprint) {
        if self.messages.state.borrow().phone == Some(*phone) {
            self.load_inbox(*phone);
        }
    }

    fn conversation_row(self: &Rc<Self>, c: &Conversation) -> gtk::ListBoxRow {
        let avatar = adw::Avatar::builder().size(44).text(&c.title).show_initials(!c.group).build();
        if c.group {
            avatar.set_icon_name(Some("system-users-symbolic"));
        }
        if let Some(picture) = c.picture.as_ref().and_then(|p| gdk::Texture::from_filename(p).ok()) {
            avatar.set_custom_image(Some(&picture));
        }
        let face = gtk::Overlay::builder().child(&avatar).valign(gtk::Align::Start).build();
        let app = gtk::Image::builder().pixel_size(14).tooltip_text(&c.app_name).build();
        match &c.icon {
            Some(icon) => app.set_from_file(Some(icon)),
            None => app.set_icon_name(Some(kind_icon(&c.kind))),
        }
        let badge = gtk::Box::builder().halign(gtk::Align::End).valign(gtk::Align::End).css_classes(["app-badge"]).build();
        badge.append(&app);
        face.add_overlay(&badge);

        let unread = c.unread > 0;
        let name = gtk::Label::builder().label(&c.title).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes([if unread { "heading" } else { "inbox-name" }]).build();
        let when = gtk::Label::builder().label(fmt_when(c.at_ms)).css_classes(["caption", "numeric", if unread { "accent" } else { "dim-label" }]).build();
        let top = gtk::Box::builder().spacing(8).build();
        top.append(&name);
        top.append(&when);
        let line = gtk::Label::builder().label(preview(c)).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim-label"]).build();
        let bottom = gtk::Box::builder().spacing(8).build();
        bottom.append(&line);
        if unread {
            bottom.append(&gtk::Label::builder().label(c.unread.to_string()).valign(gtk::Align::Center).css_classes(["count-badge", "numeric"]).build());
        }
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
        text.append(&top);
        text.append(&bottom);
        if let Some(code) = c.last.as_ref().and_then(|m| m.code.as_ref()) {
            let chip = self.copy_chip(code);
            chip.set_margin_top(6);
            text.append(&chip);
        }
        let b = gtk::Box::builder().spacing(12).margin_top(8).margin_bottom(8).margin_start(8).margin_end(8).build();
        b.append(&face);
        b.append(&text);
        let row = gtk::ListBoxRow::builder().child(&b).build();
        row.set_widget_name(&c.id);
        row
    }

    fn alert_row(self: &Rc<Self>, a: &Alert) -> gtk::ListBoxRow {
        let icon = gtk::Image::builder().pixel_size(22).css_classes(["alert-icon"]).valign(gtk::Align::Start).build();
        match &a.icon {
            Some(path) => icon.set_from_file(Some(path)),
            None => icon.set_icon_name(Some("preferences-system-notifications-symbolic")),
        }
        let title = gtk::Label::builder().label(&a.title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["inbox-name"]).build();
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
        text.append(&title);
        if !a.body.is_empty() {
            text.append(&gtk::Label::builder().label(a.body.lines().next().unwrap_or("")).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim-label"]).build());
        }
        text.append(&gtk::Label::builder().label(format!("{} · {}", a.app_name, fmt_when(a.at_ms))).xalign(0.0).css_classes(["caption", "dim-label", "numeric"]).build());
        if let Some(code) = &a.code {
            let chip = self.copy_chip(code);
            chip.set_margin_top(6);
            text.append(&chip);
        }
        let dismiss = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text(t!("Dismiss")).css_classes(["flat", "circular"]).valign(gtk::Align::Center).build();
        let (w, key) = (Rc::downgrade(self), a.key.clone());
        dismiss.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.dismiss_alert(Some(key.clone()));
            }
        });
        let b = gtk::Box::builder().spacing(12).margin_top(8).margin_bottom(8).margin_start(8).margin_end(4).build();
        b.append(&icon);
        b.append(&text);
        b.append(&dismiss);
        let row = gtk::ListBoxRow::builder().child(&b).activatable(false).build();
        row.set_widget_name(&a.key);
        row
    }
}

/// The newest line, with who said it when that isn't plain.
fn preview(c: &Conversation) -> String {
    let Some(m) = &c.last else { return String::new() };
    let text = m.text.lines().next().unwrap_or("");
    if m.mine {
        t!("You: {text}", text = text)
    } else if let Some(sender) = m.sender.as_ref().filter(|_| c.group) {
        format!("{sender}: {text}")
    } else {
        text.to_string()
    }
}

fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "sms" => "phone-symbolic",
        "mail" => "mail-unread-symbolic",
        "social" => "system-users-symbolic",
        _ => "chat-message-new-symbolic",
    }
}

/// "14:05" today, "Mon" this week, "12 Oct" before.
fn fmt_when(ms: u64) -> String {
    let Ok(dt) = glib::DateTime::from_unix_local((ms / 1000) as i64) else { return String::new() };
    let Ok(now) = glib::DateTime::now_local() else { return String::new() };
    let age = now.to_unix() - dt.to_unix();
    let format = if dt.ymd() == now.ymd() {
        "%H:%M"
    } else if age < 6 * 86_400 {
        "%a"
    } else {
        "%e %b"
    };
    dt.format(format).map(|s| s.trim().to_string()).unwrap_or_default()
}
