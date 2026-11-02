//! A phone's conversations, made from its message notifications: the list on the left,
//! the open one on the right with a field to answer, one pane at a time in narrow windows.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use linksy_ipc::linksy_core::Fingerprint;
use linksy_ipc::{Conversation, InboxMessage};
use linksy_i18n::t;

use super::Window;
use crate::fmt;

pub struct MessagesView {
    pub root: adw::NavigationSplitView,
    list: gtk::ListBox,
    list_stack: gtk::Stack,
    list_status: adw::StatusPage,
    title: adw::WindowTitle,
    bubbles: gtk::Box,
    scroller: gtk::ScrolledWindow,
    entry: gtk::Entry,
    send: gtk::Button,
    compose: gtk::Box,
    no_reply: gtk::Label,
    conversation_stack: gtk::Stack,
    state: RefCell<ViewState>,
}

#[derive(Default)]
struct ViewState {
    phone: Option<Fingerprint>,
    /// Newest first, like the rows.
    conversations: Vec<Conversation>,
    open: Option<String>,
}

impl MessagesView {
    pub fn new() -> Rc<Self> {
        let list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        let list_status = adw::StatusPage::builder().icon_name("chat-message-new-symbolic").build();
        let list_stack = gtk::Stack::new();
        list_stack.add_named(&gtk::ScrolledWindow::builder().child(&list).vexpand(true).build(), Some("list"));
        list_stack.add_named(&list_status, Some("status"));
        let side_header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
        let side_view = adw::ToolbarView::new();
        side_view.add_top_bar(&side_header);
        side_view.set_content(Some(&list_stack));
        let sidebar = adw::NavigationPage::builder().title(t!("Conversations")).child(&side_view).build();

        let title = adw::WindowTitle::new("", "");
        let content_header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).title_widget(&title).build();
        let bubbles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(12).margin_bottom(12).margin_start(16).margin_end(16).valign(gtk::Align::End).build();
        let clamp = adw::Clamp::builder().maximum_size(720).child(&bubbles).build();
        let scroller = gtk::ScrolledWindow::builder().child(&clamp).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let entry = gtk::Entry::builder().placeholder_text(t!("Reply")).hexpand(true).build();
        let send = gtk::Button::builder().icon_name("mail-send-symbolic").tooltip_text(t!("Send")).css_classes(["circular", "suggested-action"]).sensitive(false).build();
        let compose = gtk::Box::builder().spacing(8).margin_top(8).margin_bottom(10).margin_start(12).margin_end(12).build();
        compose.append(&entry);
        compose.append(&send);
        let no_reply = gtk::Label::builder()
            .label(t!("This one can't be answered from here. Answer it on the phone."))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .margin_top(12)
            .margin_bottom(14)
            .margin_start(12)
            .margin_end(12)
            .visible(false)
            .build();
        let conversation = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        conversation.append(&scroller);
        conversation.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        conversation.append(&compose);
        conversation.append(&no_reply);
        let conversation_stack = gtk::Stack::new();
        conversation_stack.add_named(&adw::StatusPage::builder().icon_name("chat-message-new-symbolic").title(t!("No Conversation Selected")).build(), Some("empty"));
        conversation_stack.add_named(&conversation, Some("open"));
        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&content_header);
        content_view.set_content(Some(&conversation_stack));
        let content = adw::NavigationPage::builder().title(t!("Messages")).child(&content_view).build();

        let root = adw::NavigationSplitView::builder().sidebar(&sidebar).content(&content).min_sidebar_width(260.0).build();
        let s = send.clone();
        entry.connect_changed(move |e| s.set_sensitive(!e.text().trim().is_empty()));
        Rc::new(Self { root, list, list_stack, list_status, title, bubbles, scroller, entry, send, compose, no_reply, conversation_stack, state: RefCell::default() })
    }

    fn status(&self, title: &str, description: &str) {
        self.list_status.set_title(title);
        self.list_status.set_description(Some(description));
        self.list_stack.set_visible_child_name("status");
    }

    fn show_list(&self, conversations: Vec<Conversation>, phone_name: &str) {
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        for c in &conversations {
            self.list.append(&row(c));
        }
        self.state.borrow_mut().conversations = conversations;
        self.select_open();
        self.empty_or_list(phone_name);
    }

    fn empty_or_list(&self, phone_name: &str) {
        if self.state.borrow().conversations.is_empty() {
            self.status(t!("No Messages"), &t!("New messages from {device}'s chats, texts and mail show up here.", device = phone_name));
        } else {
            self.list_stack.set_visible_child_name("list");
        }
    }

    /// Put `c` in its place in the list, newest first.
    fn upsert(&self, c: Conversation) {
        let at = {
            let mut state = self.state.borrow_mut();
            state.conversations.retain(|o| o.id != c.id);
            let at = state.conversations.iter().position(|o| o.at_ms <= c.at_ms).unwrap_or(state.conversations.len());
            state.conversations.insert(at, c.clone());
            at
        };
        if let Some(old) = self.row_of(&c.id) {
            self.list.remove(&old);
        }
        self.list.insert(&row(&c), at as i32);
        self.select_open();
        self.list_stack.set_visible_child_name("list");
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
    }

    /// Open the phone's conversations in the content area.
    pub(super) fn open_messages(self: &Rc<Self>, phone: Fingerprint) {
        let name = self.device(&phone).map(|d| d.device.name).unwrap_or_default();
        *self.messages.state.borrow_mut() = ViewState { phone: Some(phone), ..Default::default() };
        self.state.borrow_mut().messages_open = Some(phone);
        self.messages.conversation_stack.set_visible_child_name("empty");
        self.messages.root.set_show_content(false);
        self.messages.status(t!("Loading…"), "");
        self.camera_title.set_title(t!("Messages"));
        self.camera_title.set_subtitle(&name);
        self.content_header.set_show_title(true);
        self.camera_back.set_visible(true);
        self.page.menu.set_visible(false);
        self.content_stack.set_visible_child_name("messages");
        self.show_content();
        self.load_conversations();
    }

    pub(super) fn close_messages(&self) {
        if self.state.borrow_mut().messages_open.take().is_some() {
            *self.messages.state.borrow_mut() = ViewState::default();
            while let Some(row) = self.messages.list.first_child() {
                self.messages.list.remove(&row);
            }
            while let Some(child) = self.messages.bubbles.first_child() {
                self.messages.bubbles.remove(&child);
            }
            self.camera_title.set_subtitle(t!("Camera Preview"));
            self.content_header.set_show_title(false);
            self.camera_back.set_visible(false);
            self.page.menu.set_visible(true);
        }
    }

    fn load_conversations(self: &Rc<Self>) {
        let (Some(c), Some(phone)) = (self.client(), self.messages.state.borrow().phone) else { return };
        let call = self.bridge.call(&c, |c| async move { c.inbox().await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            if w.messages.state.borrow().phone != Some(phone) {
                return;
            }
            let name = w.device(&phone).map(|d| d.device.name).unwrap_or_default();
            match result {
                Ok(all) => {
                    let phone = phone.to_string();
                    w.messages.show_list(all.into_iter().filter(|c| c.phone == phone).collect(), &name);
                    // Development aid (screenshots): LINKSY_SCREENSHOT_THREAD=<title> opens one.
                    if let Ok(title) = std::env::var("LINKSY_SCREENSHOT_THREAD")
                        && let Some(c) = w.messages.state.borrow().conversations.iter().find(|c| c.title == title).map(|c| c.id.clone())
                    {
                        let w = w.clone();
                        glib::idle_add_local_once(move || w.open_conversation(c));
                    }
                }
                Err(e) => w.messages.status(t!("Can't Reach Its Messages"), &fmt::capitalize(&e)),
            }
        });
    }

    fn open_conversation(self: &Rc<Self>, id: String) {
        let Some(c) = self.messages.conversation(&id) else { return };
        self.messages.state.borrow_mut().open = Some(id.clone());
        self.messages.title.set_title(&c.title);
        self.messages.title.set_subtitle(&c.app_name);
        self.messages.select_open();
        self.show_reply(&c);
        self.messages.conversation_stack.set_visible_child_name("open");
        self.messages.root.set_show_content(true);
        if c.can_reply {
            self.messages.entry.grab_focus();
        }
        self.mark_read(&c);
        self.load_thread(&c);
    }

    fn show_reply(&self, c: &Conversation) {
        self.messages.compose.set_visible(c.can_reply);
        self.messages.no_reply.set_visible(!c.can_reply);
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
        for m in messages {
            let line = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).halign(if m.mine { gtk::Align::End } else { gtk::Align::Start }).build();
            if group && !m.mine
                && let Some(sender) = &m.sender
            {
                line.append(&gtk::Label::builder().label(sender).xalign(0.0).css_classes(["caption", "dim-label"]).build());
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
                let content = adw::ButtonContent::builder().icon_name("edit-copy-symbolic").label(t!("Copy {code}", code = code)).build();
                let copy = gtk::Button::builder().child(&content).halign(gtk::Align::Start).css_classes(["flat"]).build();
                let (w, code) = (Rc::downgrade(self), code.clone());
                copy.connect_clicked(move |b| {
                    b.clipboard().set_text(&code);
                    if let Some(w) = w.upgrade() {
                        w.toast(t!("Copied"));
                    }
                });
                line.append(&copy);
            }
            line.append(&gtk::Label::builder().label(fmt_when(m.at_ms)).xalign(if m.mine { 1.0 } else { 0.0 }).css_classes(["caption", "dim-label", "numeric"]).build());
            bubbles.append(&line);
        }
        self.messages.scroll_to_end();
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

    /// A conversation changed: a message came in, was read or answered.
    pub(super) fn inbox_changed(self: &Rc<Self>, c: Conversation) {
        let phone = self.messages.state.borrow().phone;
        if phone.is_none_or(|p| p.to_string() != c.phone) {
            return;
        }
        let open = self.messages.state.borrow().open.as_deref() == Some(c.id.as_str());
        self.messages.upsert(c.clone());
        if open {
            self.show_reply(&c);
            if self.window.is_active() {
                self.mark_read(&c);
            }
            self.load_thread(&c);
        }
    }
}

fn row(c: &Conversation) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&c.title))
        .subtitle(glib::markup_escape_text(&preview(c)))
        .subtitle_lines(1)
        .activatable(true)
        .build();
    let avatar = adw::Avatar::builder().size(40).text(&c.title).show_initials(!c.group).build();
    if c.group {
        avatar.set_icon_name(Some("system-users-symbolic"));
    }
    if let Some(picture) = c.picture.as_ref().and_then(|p| gdk::Texture::from_filename(p).ok()) {
        avatar.set_custom_image(Some(&picture));
    }
    row.add_prefix(&avatar);
    let side = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).valign(gtk::Align::Center).build();
    let when = gtk::Label::builder().label(fmt_when(c.at_ms)).halign(gtk::Align::End).css_classes(["caption", "numeric"]).build();
    when.add_css_class(if c.unread > 0 { "accent" } else { "dim-label" });
    side.append(&when);
    let app = gtk::Image::builder().pixel_size(16).halign(gtk::Align::End).tooltip_text(&c.app_name).build();
    match &c.icon {
        Some(icon) => app.set_from_file(Some(icon)),
        None => app.set_icon_name(Some(kind_icon(&c.kind))),
    }
    side.append(&app);
    row.add_suffix(&side);
    if c.unread > 0 {
        row.add_css_class("unread");
    }
    row.set_widget_name(&c.id);
    row
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
