//! Texting through a phone, on a page of its own: conversations on the left, the open
//! one on the right with a field to answer, one pane at a time in narrow windows.
//! Everything is asked of the phone when shown; nothing is kept on this computer.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use linksy_ipc::linksy_core::{Fingerprint, SmsMessage, SmsThread};
use linksy_i18n::t;

use super::Window;
use crate::fmt;

pub struct MessagesView {
    pub root: adw::NavigationSplitView,
    threads: gtk::ListBox,
    threads_stack: gtk::Stack,
    threads_status: adw::StatusPage,
    title: adw::WindowTitle,
    bubbles: gtk::Box,
    scroller: gtk::ScrolledWindow,
    earlier: gtk::Button,
    entry: gtk::Entry,
    send: gtk::Button,
    compose: gtk::Box,
    conversation_stack: gtk::Stack,
    new_message: gtk::Button,
    state: RefCell<ViewState>,
}

#[derive(Default)]
struct ViewState {
    phone: Option<Fingerprint>,
    threads: Vec<SmsThread>,
    /// The open conversation (`None` with `to` set: a new one).
    thread: Option<String>,
    to: Vec<String>,
    messages: Vec<SmsMessage>,
}

impl MessagesView {
    pub fn new() -> Rc<Self> {
        let threads = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        let threads_status = adw::StatusPage::builder().icon_name("chat-message-new-symbolic").build();
        let threads_stack = gtk::Stack::new();
        threads_stack.add_named(&gtk::ScrolledWindow::builder().child(&threads).vexpand(true).build(), Some("list"));
        threads_stack.add_named(&threads_status, Some("status"));
        let new_message = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text(t!("New Message")).build();
        let side_header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
        side_header.pack_start(&new_message);
        let side_view = adw::ToolbarView::new();
        side_view.add_top_bar(&side_header);
        side_view.set_content(Some(&threads_stack));
        let sidebar = adw::NavigationPage::builder().title(t!("Conversations")).child(&side_view).build();

        let title = adw::WindowTitle::new("", "");
        let content_header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).title_widget(&title).build();
        let earlier = gtk::Button::builder().label(t!("Earlier Messages")).halign(gtk::Align::Center).css_classes(["flat"]).visible(false).build();
        let bubbles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(12).margin_bottom(12).margin_start(16).margin_end(16).valign(gtk::Align::End).build();
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        column.append(&earlier);
        column.append(&bubbles);
        let clamp = adw::Clamp::builder().maximum_size(720).child(&column).build();
        let scroller = gtk::ScrolledWindow::builder().child(&clamp).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let entry = gtk::Entry::builder().placeholder_text(t!("Text Message")).hexpand(true).build();
        let send = gtk::Button::builder().icon_name("mail-send-symbolic").tooltip_text(t!("Send")).css_classes(["circular", "suggested-action"]).sensitive(false).build();
        let compose = gtk::Box::builder().spacing(8).margin_top(8).margin_bottom(10).margin_start(12).margin_end(12).build();
        compose.append(&entry);
        compose.append(&send);
        let conversation = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        conversation.append(&scroller);
        conversation.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        conversation.append(&compose);
        let conversation_stack = gtk::Stack::new();
        conversation_stack.add_named(&adw::StatusPage::builder().icon_name("chat-message-new-symbolic").title(t!("No Conversation Selected")).build(), Some("empty"));
        conversation_stack.add_named(&conversation, Some("open"));
        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&content_header);
        content_view.set_content(Some(&conversation_stack));
        let content = adw::NavigationPage::builder().title(t!("Messages")).child(&content_view).build();

        let root = adw::NavigationSplitView::builder().sidebar(&sidebar).content(&content).min_sidebar_width(240.0).build();
        let s = send.clone();
        entry.connect_changed(move |e| s.set_sensitive(!e.text().trim().is_empty()));
        Rc::new(Self {
            root,
            threads,
            threads_stack,
            threads_status,
            title,
            bubbles,
            scroller,
            earlier,
            entry,
            send,
            compose,
            conversation_stack,
            new_message,
            state: RefCell::default(),
        })
    }

    fn status(&self, title: &str, description: &str) {
        self.threads_status.set_title(title);
        self.threads_status.set_description(Some(description));
        self.threads_stack.set_visible_child_name("status");
    }

    fn show_threads(&self, threads: Vec<SmsThread>) {
        while let Some(row) = self.threads.first_child() {
            self.threads.remove(&row);
        }
        if threads.is_empty() {
            self.status(t!("No Messages"), t!("Texts on the phone show up here."));
        } else {
            self.threads_stack.set_visible_child_name("list");
        }
        let open = self.state.borrow().thread.clone();
        for t in &threads {
            let row = adw::ActionRow::builder().title(glib::markup_escape_text(&who(t))).subtitle(glib::markup_escape_text(&one_line(&t.snippet))).subtitle_lines(1).activatable(true).build();
            row.add_prefix(&adw::Avatar::builder().size(36).text(who(t)).show_initials(true).build());
            let when = gtk::Label::builder().label(fmt_when(t.date_ms)).css_classes(["caption", "dim-label"]).valign(gtk::Align::Start).margin_top(10).build();
            row.add_suffix(&when);
            if t.unread {
                row.add_css_class("unread");
            }
            row.set_widget_name(&t.id);
            self.threads.append(&row);
            if open.as_deref() == Some(t.id.as_str()) {
                self.threads.select_row(Some(&row));
            }
        }
        self.state.borrow_mut().threads = threads;
    }

    /// Replace (or with `older`, extend) the open conversation's messages.
    fn show_messages(&self, page: Vec<SmsMessage>, more: bool, older: bool) {
        {
            let mut state = self.state.borrow_mut();
            // Pages come newest first; the list keeps oldest first.
            let mut page = page;
            page.reverse();
            if older {
                page.extend(std::mem::take(&mut state.messages));
                state.messages = page;
            } else {
                state.messages = page;
            }
        }
        self.earlier.set_visible(more);
        self.render_bubbles();
        if !older {
            self.scroll_to_end();
        }
    }

    fn render_bubbles(&self) {
        while let Some(child) = self.bubbles.first_child() {
            self.bubbles.remove(&child);
        }
        let state = self.state.borrow();
        let group = state.thread.as_ref().and_then(|id| state.threads.iter().find(|t| &t.id == id)).is_some_and(|t| t.addresses.len() > 1);
        for m in &state.messages {
            let bubble = gtk::Label::builder()
                .label(&m.body)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .xalign(0.0)
                .selectable(true)
                .max_width_chars(48)
                .css_classes(["bubble", if m.from_me { "mine" } else { "theirs" }])
                .build();
            let line = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).halign(if m.from_me { gtk::Align::End } else { gtk::Align::Start }).build();
            if group && !m.from_me && !m.address.is_empty() {
                line.append(&gtk::Label::builder().label(&m.address).xalign(0.0).css_classes(["caption", "dim-label"]).build());
            }
            line.append(&bubble);
            line.append(&gtk::Label::builder().label(fmt_when(m.date_ms)).xalign(if m.from_me { 1.0 } else { 0.0 }).css_classes(["caption", "dim-label", "numeric"]).build());
            self.bubbles.append(&line);
        }
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
        m.threads.connect_row_activated(move |_, row| {
            let Some(w) = w.upgrade() else { return };
            let id = row.widget_name().to_string();
            w.open_thread(id);
        });
        let w = Rc::downgrade(self);
        m.earlier.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.load_messages(true);
            }
        });
        let w = Rc::downgrade(self);
        let send = move || {
            if let Some(w) = w.upgrade() {
                w.send_text();
            }
        };
        let s = send.clone();
        m.send.connect_clicked(move |_| s());
        m.entry.connect_activate(move |_| send());
        let w = Rc::downgrade(self);
        m.new_message.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.new_conversation();
            }
        });
    }

    /// Open the phone's texts in the content area.
    pub(super) fn open_messages(self: &Rc<Self>, phone: Fingerprint) {
        let name = self.device(&phone).map(|d| d.device.name).unwrap_or_default();
        {
            let mut state = self.messages.state.borrow_mut();
            *state = ViewState { phone: Some(phone), ..Default::default() };
        }
        self.state.borrow_mut().messages_open = Some(phone);
        self.messages.conversation_stack.set_visible_child_name("empty");
        self.messages.root.set_show_content(false);
        self.messages.status(t!("Loading…"), &t!("Asking {name} for its messages.", name));
        self.camera_title.set_title(t!("Messages"));
        self.camera_title.set_subtitle(&name);
        self.content_header.set_show_title(true);
        self.camera_back.set_visible(true);
        self.page.menu.set_visible(false);
        self.content_stack.set_visible_child_name("messages");
        self.show_content();
        self.load_threads();
    }

    pub(super) fn close_messages(&self) {
        if self.state.borrow_mut().messages_open.take().is_some() {
            *self.messages.state.borrow_mut() = ViewState::default();
            self.camera_title.set_subtitle(t!("Camera Preview"));
            self.content_header.set_show_title(false);
            self.camera_back.set_visible(false);
            self.page.menu.set_visible(true);
        }
    }

    fn load_threads(self: &Rc<Self>) {
        let (Some(c), Some(phone)) = (self.client(), self.messages.state.borrow().phone) else { return };
        let call = self.bridge.call(&c, move |c| async move { c.sms_threads(phone.to_string()).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            if w.messages.state.borrow().phone != Some(phone) {
                return;
            }
            match result {
                Ok(threads) => {
                    w.messages.show_threads(threads);
                    // Development aid (screenshots): LINKSY_SCREENSHOT_THREAD=<id> opens one.
                    if let Ok(id) = std::env::var("LINKSY_SCREENSHOT_THREAD")
                        && w.messages.state.borrow().thread.is_none()
                    {
                        w.open_thread(id);
                    }
                }
                Err(e) => w.messages.status(t!("Can't Reach Its Messages"), &fmt::capitalize(&e)),
            }
        });
    }

    fn open_thread(self: &Rc<Self>, id: String) {
        let to = {
            let mut state = self.messages.state.borrow_mut();
            let to = state.threads.iter().find(|t| t.id == id).map(|t| t.addresses.clone()).unwrap_or_default();
            state.thread = Some(id.clone());
            state.to = to.clone();
            state.messages.clear();
            to
        };
        let label = self.messages.state.borrow().threads.iter().find(|t| t.id == id).map(who).unwrap_or_default();
        self.messages.title.set_title(&label);
        self.messages.title.set_subtitle(&to.join(", "));
        let mut row = self.messages.threads.first_child();
        while let Some(r) = row {
            if r.widget_name() == id.as_str()
                && let Some(r) = r.downcast_ref::<gtk::ListBoxRow>()
            {
                self.messages.threads.select_row(Some(r));
            }
            row = r.next_sibling();
        }
        self.messages.render_bubbles();
        self.messages.conversation_stack.set_visible_child_name("open");
        self.messages.root.set_show_content(true);
        self.messages.entry.grab_focus();
        self.load_messages(false);
    }

    fn load_messages(self: &Rc<Self>, older: bool) {
        let (phone, thread, before) = {
            let state = self.messages.state.borrow();
            let before = if older { state.messages.first().map(|m| m.date_ms) } else { None };
            (state.phone, state.thread.clone(), before)
        };
        let (Some(c), Some(phone), Some(thread)) = (self.client(), phone, thread) else { return };
        let t = thread.clone();
        let call = self.bridge.call(&c, move |c| async move { c.sms_messages(phone.to_string(), t, before).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            if w.messages.state.borrow().thread.as_deref() != Some(thread.as_str()) {
                return;
            }
            match result {
                Ok((messages, more)) => w.messages.show_messages(messages, more, older),
                Err(e) => w.toast(&fmt::capitalize(&e)),
            }
        });
    }

    fn send_text(self: &Rc<Self>) {
        let text = self.messages.entry.text().trim().to_string();
        let (phone, to) = {
            let state = self.messages.state.borrow();
            (state.phone, state.to.clone())
        };
        let (Some(c), Some(phone)) = (self.client(), phone) else { return };
        if text.is_empty() || to.is_empty() {
            return;
        }
        self.messages.compose.set_sensitive(false);
        let call = self.bridge.call(&c, move |c| async move { c.sms_send(phone.to_string(), to, text).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            w.messages.compose.set_sensitive(true);
            match result {
                Ok(()) => {
                    w.messages.entry.set_text("");
                    w.messages.entry.grab_focus();
                    // The phone files it; ask for the conversation as it is now.
                    if w.messages.state.borrow().thread.is_some() {
                        w.load_messages(false);
                    }
                    w.load_threads();
                }
                Err(e) => w.toast(&t!("Not sent: {error}", error = fmt::capitalize(&e))),
            }
        });
    }

    /// Ask for a number, then write to it.
    fn new_conversation(self: &Rc<Self>) {
        let entry = adw::EntryRow::builder().title(t!("Phone Number")).input_purpose(gtk::InputPurpose::Phone).build();
        let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
        list.append(&entry);
        let dialog = adw::AlertDialog::builder().heading(t!("New Message")).extra_child(&list).build();
        dialog.add_response("cancel", t!("Cancel"));
        dialog.add_response("write", t!("Write"));
        dialog.set_response_appearance("write", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("write"));
        let w = self.clone();
        dialog.connect_response(None, move |_, response| {
            let number = entry.text().trim().to_string();
            if response != "write" || number.is_empty() {
                return;
            }
            {
                let mut state = w.messages.state.borrow_mut();
                // An existing conversation with that number opens instead.
                if let Some(t) = state.threads.iter().find(|t| t.addresses == [number.clone()]).cloned() {
                    drop(state);
                    w.open_thread(t.id);
                    return;
                }
                state.thread = None;
                state.to = vec![number.clone()];
                state.messages.clear();
            }
            w.messages.threads.unselect_all();
            w.messages.title.set_title(&number);
            w.messages.title.set_subtitle(t!("New Message"));
            w.messages.earlier.set_visible(false);
            w.messages.render_bubbles();
            w.messages.conversation_stack.set_visible_child_name("open");
            w.messages.root.set_show_content(true);
            w.messages.entry.grab_focus();
        });
        dialog.present(Some(&self.window));
    }

    /// A text came in or went out on the phone.
    pub(super) fn sms_arrived(self: &Rc<Self>, from: Fingerprint, thread: String, message: SmsMessage) {
        let open = {
            let state = self.messages.state.borrow();
            (state.phone == Some(from)).then(|| state.thread.clone())
        };
        let Some(open) = open else { return };
        if open.as_deref() == Some(thread.as_str()) {
            let added = {
                let mut state = self.messages.state.borrow_mut();
                let new = !state.messages.iter().any(|m| m.id == message.id);
                if new {
                    state.messages.push(message);
                }
                new
            };
            if added {
                self.messages.render_bubbles();
                self.messages.scroll_to_end();
            }
        }
        self.load_threads();
    }
}

/// Who a conversation is with: contacts' names where known, else numbers.
fn who(t: &SmsThread) -> String {
    let names: Vec<&str> = t.addresses.iter().enumerate().map(|(i, a)| t.names.get(i).map(String::as_str).filter(|n| !n.is_empty()).unwrap_or(a)).collect();
    names.join(", ")
}

fn one_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
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
