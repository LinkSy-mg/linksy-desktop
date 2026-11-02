//! A phone's Files tab: the folder it shares, one folder at a time. Only the rows in view
//! are built (a camera folder holds thousands of photos); a file opens through the same
//! bridge as the Files sidebar.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use linksy_ipc::linksy_core::{Fingerprint, FsEntry};
use linksy_i18n::t;

use super::Window;
use crate::fmt;

pub struct FilesView {
    pub root: gtk::Box,
    crumbs: gtk::Box,
    store: gio::ListStore,
    list: gtk::ListView,
    stack: gtk::Stack,
    status: gtk::Label,
    open_folder: gtk::Button,
    state: RefCell<FilesState>,
}

#[derive(Default)]
struct FilesState {
    phone: Option<Fingerprint>,
    /// The folder shown, "/" first, always ending in "/".
    path: String,
    /// Where apps open the phone's files, once asked.
    place: Option<String>,
}

impl FilesView {
    pub fn new() -> Rc<Self> {
        let crumbs = gtk::Box::builder().spacing(2).hexpand(true).build();
        let open_folder = gtk::Button::builder().label(t!("Open in Files")).valign(gtk::Align::Center).build();
        let top = gtk::Box::builder().spacing(8).build();
        top.append(&gtk::ScrolledWindow::builder().child(&crumbs).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).hexpand(true).build());
        top.append(&open_folder);

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let icon = gtk::Image::builder().pixel_size(22).build();
            let name = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["inbox-name"]).build();
            let detail = gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label", "numeric"]).build();
            let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
            text.append(&name);
            text.append(&detail);
            let next = gtk::Image::builder().icon_name("go-next-symbolic").css_classes(["dim-label"]).build();
            let b = gtk::Box::builder().spacing(14).margin_top(9).margin_bottom(9).margin_start(10).margin_end(10).build();
            b.append(&icon);
            b.append(&text);
            b.append(&next);
            item.set_child(Some(&b));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let (Some(obj), Some(row)) = (item.item().and_downcast::<glib::BoxedAnyObject>(), item.child()) else { return };
            let e = obj.borrow::<FsEntry>();
            let icon = row.first_child().and_downcast::<gtk::Image>().expect("icon");
            let text = icon.next_sibling().expect("text");
            let next = text.next_sibling().expect("next");
            let name = text.first_child().and_downcast::<gtk::Label>().expect("name");
            let detail = name.next_sibling().and_downcast::<gtk::Label>().expect("detail");
            icon.set_icon_name(Some(if e.dir { "folder-symbolic" } else { file_icon(&e.name) }));
            name.set_label(&e.name);
            let when = (e.modified_ms > 0).then(|| super::page::day_label(e.modified_ms));
            let what = if e.dir { None } else { Some(fmt::size(e.size)) };
            detail.set_label(&[what, when].into_iter().flatten().collect::<Vec<_>>().join(" · "));
            detail.set_visible(!detail.label().is_empty());
            next.set_visible(e.dir);
        });
        let list = gtk::ListView::builder().model(&gtk::NoSelection::new(Some(store.clone()))).factory(&factory).single_click_activate(true).css_classes(["files-list"]).build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(520)
            .css_classes(["files-box"])
            .build();
        let status = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label"]).margin_top(28).margin_bottom(28).build();
        let stack = gtk::Stack::builder().vhomogeneous(false).build();
        stack.add_named(&scroller, Some("list"));
        stack.add_named(&status, Some("status"));
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        root.append(&top);
        root.append(&stack);
        Rc::new(Self { root, crumbs, store, list, stack, status, open_folder, state: RefCell::default() })
    }

    fn status(&self, text: &str) {
        self.status.set_label(text);
        self.stack.set_visible_child_name("status");
    }
}

impl Window {
    pub(super) fn setup_files(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.files.list.connect_activate(move |list, position| {
            let Some(w) = w.upgrade() else { return };
            let Some(obj) = list.model().and_then(|m| m.item(position)).and_downcast::<glib::BoxedAnyObject>() else { return };
            let e = obj.borrow::<FsEntry>().clone();
            let path = format!("{}{}", w.files.state.borrow().path, e.name);
            if e.dir {
                w.files_open(format!("{path}/"));
            } else {
                w.files_launch(path);
            }
        });
        let w = Rc::downgrade(self);
        self.files.open_folder.connect_clicked(move |_| {
            let Some(w) = w.upgrade() else { return };
            let (path, phone) = {
                let state = w.files.state.borrow();
                (state.path.clone(), state.phone)
            };
            match phone {
                // Its top folder also goes in the Files sidebar.
                Some(phone) if path == "/" => w.browse_files(phone),
                _ => w.files_launch(path),
            }
        });
    }

    /// The Files tab shows: the phone's top folder the first time, else where it was.
    pub(super) fn files_shown(self: &Rc<Self>, phone: Fingerprint) {
        let same = self.files.state.borrow().phone == Some(phone);
        if !same {
            *self.files.state.borrow_mut() = FilesState { phone: Some(phone), path: "/".into(), place: None };
        }
        let path = self.files.state.borrow().path.clone();
        self.files_open(path);
    }

    fn files_open(self: &Rc<Self>, path: String) {
        let (Some(c), Some(phone)) = (self.client(), self.files.state.borrow().phone) else { return };
        self.files.state.borrow_mut().path = path.clone();
        self.files_crumbs(phone, &path);
        if self.files.store.n_items() == 0 {
            self.files.status(t!("Loading…"));
        }
        let p = path.clone();
        let call = self.bridge.call(&c, move |c| async move { c.fs_list(phone.to_string(), p).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            if w.files.state.borrow().path != path || w.files.state.borrow().phone != Some(phone) {
                return;
            }
            match result {
                Ok(entries) => {
                    let objects: Vec<glib::BoxedAnyObject> = entries.into_iter().filter(|e| !e.name.starts_with('.')).map(glib::BoxedAnyObject::new).collect();
                    w.files.store.splice(0, w.files.store.n_items(), &objects);
                    if objects.is_empty() {
                        w.files.status(t!("This folder is empty."));
                    } else {
                        w.files.stack.set_visible_child_name("list");
                        w.files.list.scroll_to(0, gtk::ListScrollFlags::NONE, None);
                    }
                }
                Err(e) => {
                    w.files.store.remove_all();
                    w.files.status(&fmt::capitalize(&e));
                }
            }
        });
    }

    /// The phone's name, then each folder down to this one, each a button back to it.
    fn files_crumbs(self: &Rc<Self>, phone: Fingerprint, path: &str) {
        let crumbs = &self.files.crumbs;
        while let Some(c) = crumbs.first_child() {
            crumbs.remove(&c);
        }
        let name = self.device(&phone).map(|d| d.device.name).unwrap_or_default();
        let mut at = String::from("/");
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        for (i, label) in std::iter::once(name.as_str()).chain(parts.iter().copied()).enumerate() {
            if i > 0 {
                at.push_str(label);
                at.push('/');
                crumbs.append(&gtk::Image::builder().icon_name("go-next-symbolic").pixel_size(12).css_classes(["dim-label"]).build());
            }
            let last = i == parts.len();
            let b = gtk::Button::builder().label(label).css_classes(if last { vec!["flat", "crumb", "current"] } else { vec!["flat", "crumb"] }).build();
            let (w, target) = (Rc::downgrade(self), at.clone());
            b.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    w.files_open(target.clone());
                }
            });
            crumbs.append(&b);
        }
    }

    /// Open a file, or a folder in the file manager, through the phone's bridge.
    fn files_launch(self: &Rc<Self>, path: String) {
        let (Some(c), Some(phone)) = (self.client(), self.files.state.borrow().phone) else { return };
        let known = self.files.state.borrow().place.clone();
        let call = self.bridge.call(&c, move |c| async move {
            match known {
                Some(place) => Ok(place),
                None => c.fs_place(phone.to_string()).await,
            }
        });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(place) => {
                    w.files.state.borrow_mut().place = Some(place.clone());
                    let escaped: Vec<String> = path.split('/').map(|p| glib::Uri::escape_string(p, None, false).to_string()).collect();
                    let file = gio::File::for_uri(&format!("{}{}", place.trim_end_matches('/'), escaped.join("/")));
                    let w2 = w.clone();
                    gtk::FileLauncher::new(Some(&file)).launch(Some(&w.window), gio::Cancellable::NONE, move |r| {
                        if let Err(e) = r
                            && !e.matches(gtk::DialogError::Dismissed)
                        {
                            w2.toast(&fmt::capitalize(&e.to_string()));
                        }
                    });
                }
                Err(e) => w.toast(&fmt::capitalize(&e)),
            }
        });
    }
}

fn file_icon(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "dng" => "image-x-generic-symbolic",
        "mp4" | "mkv" | "webm" | "mov" | "3gp" => "video-x-generic-symbolic",
        "mp3" | "m4a" | "ogg" | "opus" | "flac" | "wav" | "aac" => "audio-x-generic-symbolic",
        _ => "text-x-generic-symbolic",
    }
}
