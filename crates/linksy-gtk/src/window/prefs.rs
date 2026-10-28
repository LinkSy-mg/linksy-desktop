//! Preferences: device name, where files go, how files are received, whether to stay
//! connected, and phone notifications. Every change is saved right away, like GNOME's own
//! settings.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use linksy_ipc::{NotificationPrefs, PhotoPrefs, UpdateInfo, UpdatePrefs};
use linksy_ipc::linksy_core::Settings;
use linksy_i18n::t;

use super::Window;
use crate::fmt;

impl Window {
    pub(super) fn show_preferences(self: &Rc<Self>) {
        let Some(c) = self.client() else { return };
        let w = self.clone();
        let call = self.bridge.call(&c, |c| async move {
            // The optional parts never hold the dialog up: an older service may not answer.
            let optional = std::time::Duration::from_secs(3);
            let updates = tokio::time::timeout(optional, c.updates()).await.ok().and_then(|r| r.ok());
            let terminals = tokio::time::timeout(optional, c.terminal_status()).await.ok().and_then(|r| r.ok());
            Ok((c.settings().await?, c.notification_prefs().await?, c.photo_prefs().await?, updates, terminals))
        });
        glib::spawn_future_local(async move {
            match call.await {
                Ok((s, n, p, u, t)) => w.present_preferences(s, n, p, u, t),
                Err(e) => w.toast_error(&e, None),
            }
        });
    }

    fn present_preferences(self: &Rc<Self>, settings: Settings, notifications: NotificationPrefs, photos: PhotoPrefs, updates: Option<UpdateInfo>, terminals: Option<linksy_ipc::TerminalStatus>) {
        let dialog = adw::PreferencesDialog::builder().title(t!("Preferences")).search_enabled(false).build();
        let current = Rc::new(RefCell::new(settings.clone()));
        // Set while a widget is being reset after a failed save, so it doesn't save again.
        let reverting = Rc::new(Cell::new(false));

        let name = adw::EntryRow::builder().title(t!("Device Name")).text(&settings.name).show_apply_button(true).build();
        let device = adw::PreferencesGroup::builder().title(t!("This Device")).description(t!("The name other devices see.")).build();
        device.add(&name);
        // For "Pair by Address" on networks that hide devices from each other.
        if let Some(ip) = fmt::local_ip() {
            let port = self.state.borrow().port.unwrap_or(0);
            let shown = if port == 47_200 { ip.to_string() } else { std::net::SocketAddr::new(ip, port).to_string() };
            let address = adw::ActionRow::builder().title(t!("Address")).subtitle(&shown).subtitle_selectable(true).build();
            let copy = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text(t!("Copy Address")).valign(gtk::Align::Center).css_classes(["flat"]).build();
            let d = dialog.clone();
            copy.connect_clicked(move |b| {
                b.clipboard().set_text(&shown);
                d.add_toast(adw::Toast::new(t!("Address copied")));
            });
            address.add_suffix(&copy);
            device.add(&address);
        }

        let folder = adw::ActionRow::builder().title(t!("Save Files To")).subtitle(fmt::home_relative(&settings.download_dir)).build();
        let choose = gtk::Button::builder().icon_name("folder-open-symbolic").tooltip_text(t!("Choose Folder")).valign(gtk::Align::Center).css_classes(["flat"]).build();
        folder.add_suffix(&choose);
        folder.set_activatable_widget(Some(&choose));
        let auto = adw::SwitchRow::builder()
            .title(t!("Accept From Paired Devices"))
            .subtitle(t!("Files from devices you paired arrive without asking."))
            .active(settings.auto_accept)
            .build();
        let receiving = adw::PreferencesGroup::builder().title(t!("Receiving")).build();
        receiving.add(&folder);
        receiving.add(&auto);
        receiving.add(&self.photo_import_row(&dialog, photos));

        let stay = adw::SwitchRow::builder()
            .title(t!("Stay Connected"))
            .subtitle(t!("Keep paired devices connected for instant sharing. Uses more battery; otherwise devices connect only when there's something to share."))
            .active(settings.stay_connected)
            .build();
        let away = adw::SwitchRow::builder()
            .title(t!("Away From Home"))
            .subtitle(t!("Reach your paired devices over the internet too, for a week after they were last on the same Wi-Fi. Everything stays encrypted end to end; Linksy's relay only helps them find each other."))
            .active(settings.away)
            .build();
        // Self-hosting: a relay of the user's own, instead of Linksy's (linksy-server).
        let relay = adw::EntryRow::builder().title(t!("Relay Server")).text(settings.relay.as_deref().unwrap_or_default()).show_apply_button(true).build();
        let connection = adw::PreferencesGroup::builder()
            .title(t!("Connection"))
            .description(t!("Away from home goes through Linksy's relay unless you give one you host yourself, as https://relay.example.com. Set the same relay on every device."))
            .build();
        connection.add(&stay);
        connection.add(&away);
        connection.add(&relay);

        let page = adw::PreferencesPage::new();
        page.add(&self.language_group(&dialog));
        page.add(&device);
        page.add(&receiving);
        page.add(&connection);
        page.add(&self.screen_group(&dialog));
        // Only if a paired phone can send its notifications at all.
        let any = self.state.borrow().devices.iter().any(|d| d.trusted && super::can(&d.device, "notifications") && d.device.os == "android");
        if any {
            page.add(&self.phone_notifications_group(&dialog, notifications));
        }
        if let Some(t) = terminals {
            page.add(&self.terminal_group(&dialog, t.code_set));
        }
        if let Some(u) = updates {
            page.add(&self.updates_group(&dialog, u));
        }
        dialog.add(&page);

        // Save `change` applied to the current settings; `undo` resets the widget on failure.
        let save = {
            let (w, dialog, current) = (self.clone(), dialog.clone(), current.clone());
            Rc::new(move |change: Box<dyn FnOnce(&mut Settings)>, undo: Box<dyn FnOnce(&Settings)>| {
                let Some(c) = w.client() else { return };
                let mut next = current.borrow().clone();
                change(&mut next);
                let call = w.bridge.call(&c, move |c| async move { c.update_settings(next).await });
                let (dialog, current) = (dialog.clone(), current.clone());
                glib::spawn_future_local(async move {
                    match call.await {
                        Ok(applied) => *current.borrow_mut() = applied,
                        Err(e) => {
                            dialog.add_toast(adw::Toast::new(&glib::markup_escape_text(&fmt::capitalize(&e))));
                            undo(&current.borrow());
                        }
                    }
                });
            })
        };

        let (s, r) = (save.clone(), reverting.clone());
        name.connect_apply(move |row| {
            let text = row.text().to_string();
            let row = row.clone();
            let r = r.clone();
            s(
                Box::new(move |n| n.name = text),
                Box::new(move |old| {
                    r.set(true);
                    row.set_text(&old.name);
                    r.set(false);
                }),
            );
        });

        let (s, r) = (save.clone(), reverting.clone());
        relay.connect_apply(move |row| {
            // Blank: back to Linksy's own relay.
            let text = row.text().trim().to_string();
            let row = row.clone();
            let r = r.clone();
            s(
                Box::new(move |n| n.relay = (!text.is_empty()).then_some(text)),
                Box::new(move |old| {
                    r.set(true);
                    row.set_text(old.relay.as_deref().unwrap_or_default());
                    r.set(false);
                }),
            );
        });

        for (row, set) in [(&auto, (|s: &mut Settings, v| s.auto_accept = v) as fn(&mut Settings, bool)), (&stay, |s, v| s.stay_connected = v), (&away, |s, v| s.away = v)] {
            let (s, r) = (save.clone(), reverting.clone());
            row.connect_active_notify(move |row| {
                if r.get() {
                    return;
                }
                let value = row.is_active();
                let (row, r) = (row.clone(), r.clone());
                s(
                    Box::new(move |n| set(n, value)),
                    Box::new(move |_| {
                        r.set(true);
                        row.set_active(!value);
                        r.set(false);
                    }),
                );
            });
        }

        let (s, w) = (save.clone(), Rc::downgrade(self));
        let row = folder.clone();
        choose.connect_clicked(move |_| {
            let Some(w) = w.upgrade() else { return };
            let picker = gtk::FileDialog::builder().title(t!("Save Files To")).modal(true).build();
            picker.set_initial_folder(Some(&gio::File::for_path(&current.borrow().download_dir)));
            let (s, row) = (s.clone(), row.clone());
            picker.select_folder(Some(&w.window), gio::Cancellable::NONE, move |res| {
                let Some(path) = res.ok().and_then(|f| f.path()) else { return };
                row.set_subtitle(&fmt::home_relative(&path));
                let reset = row.clone();
                s(
                    Box::new(move |n| n.download_dir = path),
                    Box::new(move |old| reset.set_subtitle(&fmt::home_relative(&old.download_dir))),
                );
            });
        });

        dialog.present(Some(&self.window));
    }

    /// "Language": the system's, or one of Linksy's by its own name. It's read when Linksy
    /// starts, so the toast offers to start Linksy (and its service) again.
    fn language_group(self: &Rc<Self>, dialog: &adw::PreferencesDialog) -> adw::PreferencesGroup {
        let mut names = vec![t!("System Default")];
        names.extend(linksy_i18n::LANGUAGES.iter().map(|(_, name)| *name));
        let chosen = linksy_i18n::chosen();
        let selected = chosen.as_deref().and_then(|code| linksy_i18n::LANGUAGES.iter().position(|(c, _)| *c == code)).map_or(0, |i| i + 1);
        let row = adw::ComboRow::builder().title(t!("Language")).model(&gtk::StringList::new(&names)).selected(selected as u32).build();
        let group = adw::PreferencesGroup::new();
        group.add(&row);
        // One "Restart Now" at a time, however often the language changes.
        let shown: Rc<RefCell<Option<adw::Toast>>> = Rc::default();
        let (w, dialog) = (Rc::downgrade(self), dialog.clone());
        row.connect_selected_notify(move |row| {
            let code = match row.selected() {
                gtk::INVALID_LIST_POSITION => return,
                0 => None,
                i => linksy_i18n::LANGUAGES.get(i as usize - 1).map(|(c, _)| *c),
            };
            if let Some(old) = shown.borrow_mut().take() {
                old.dismiss();
            }
            if let Err(e) = linksy_i18n::choose(code) {
                dialog.add_toast(adw::Toast::new(&glib::markup_escape_text(&fmt::capitalize(&e.to_string()))));
                return;
            }
            let toast = adw::Toast::builder().title(t!("Linksy uses it the next time it starts")).button_label(t!("Restart Now")).timeout(0).build();
            let w = w.clone();
            toast.connect_button_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    w.restart_app();
                }
            });
            dialog.add_toast(toast.clone());
            *shown.borrow_mut() = Some(toast);
        });
        group
    }

    /// Start Linksy again, in the language just chosen: the service quits first (it reads
    /// the language when it starts too), and the new window starts it as at any start.
    pub(super) fn restart_app(self: &Rc<Self>) {
        let app = self.window.application();
        // A service started by hand (LINKSY_SOCKET, development) is left running.
        let client = self.client().filter(|_| !linksy_ipc::custom_socket());
        let quit = client.map(|c| self.bridge.call(&c, |c| async move { tokio::time::timeout(std::time::Duration::from_secs(3), c.quit()).await.unwrap_or(Err(linksy_ipc::ClientError::Disconnected)) }));
        glib::spawn_future_local(async move {
            if let Some(quit) = quit {
                let _ = quit.await;
                // Wait for it to go, so the new window starts a new one.
                for _ in 0..20 {
                    if std::fs::metadata(linksy_ipc::socket_path()).is_err() {
                        break;
                    }
                    glib::timeout_future(std::time::Duration::from_millis(150)).await;
                }
            }
            // Once this process has ended (and let go of the app's D-Bus name)
            linksy_upgrade::relaunch();
            if let Some(app) = app {
                app.quit();
            }
        });
    }

    /// "Phone Notifications": on or off, and the apps muted from a notification's Mute.
    fn phone_notifications_group(self: &Rc<Self>, dialog: &adw::PreferencesDialog, prefs: NotificationPrefs) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(t!("Phone Notifications"))
            .description(t!("Messages and alerts from your phone show up here, and you can reply to them. Your phone asks for permission first."))
            .build();
        let show = adw::SwitchRow::builder().title(t!("Show Phone Notifications")).active(prefs.show).build();
        group.add(&show);
        let prefs = Rc::new(RefCell::new(prefs));
        let save = {
            let (w, dialog, prefs) = (self.clone(), dialog.clone(), prefs.clone());
            Rc::new(move || {
                let Some(c) = w.client() else { return };
                let next = prefs.borrow().clone();
                let call = w.bridge.call(&c, move |c| async move { c.set_notification_prefs(next).await });
                let dialog = dialog.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = call.await {
                        dialog.add_toast(adw::Toast::new(&glib::markup_escape_text(&fmt::capitalize(&e))));
                    }
                });
            })
        };
        let (p, s) = (prefs.clone(), save.clone());
        show.connect_active_notify(move |row| {
            p.borrow_mut().show = row.is_active();
            s();
        });
        let muted = prefs.borrow().muted.clone();
        for app in muted {
            let row = adw::ActionRow::builder().title(glib::markup_escape_text(&app.app_name)).subtitle(t!("Muted")).build();
            let unmute = gtk::Button::builder().label(t!("Unmute")).valign(gtk::Align::Center).css_classes(["flat"]).build();
            row.add_suffix(&unmute);
            group.add(&row);
            let (p, s, g) = (prefs.clone(), save.clone(), group.clone());
            unmute.connect_clicked(move |_| {
                p.borrow_mut().muted.retain(|m| m.app != app.app);
                g.remove(&row);
                s();
            });
        }
        group
    }

    /// The screen other devices see: the one picked in the desktop's dialog the first time,
    /// remembered, until the user asks to pick again (another monitor, say).
    fn screen_group(self: &Rc<Self>, dialog: &adw::PreferencesDialog) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder().title(t!("Screen Sharing")).build();
        let row = adw::ActionRow::builder()
            .title(t!("Choose the Screen Again"))
            .subtitle(t!("Linksy remembers the screen you picked when a device first asked to see or control this computer. Choose again to pick another monitor the next time one asks."))
            .build();
        row.add_prefix(&gtk::Image::builder().icon_name("video-display-symbolic").build());
        let again = gtk::Button::builder().label(t!("Choose Again")).valign(gtk::Align::Center).build();
        row.add_suffix(&again);
        group.add(&row);
        let (w, dialog) = (self.clone(), dialog.clone());
        again.connect_clicked(move |button| {
            let Some(c) = w.client() else { return };
            let call = w.bridge.call(&c, |c| async move { c.forget_screen().await });
            let (dialog, button) = (dialog.clone(), button.clone());
            glib::spawn_future_local(async move {
                match call.await {
                    Ok(_) => {
                        button.set_sensitive(false);
                        dialog.add_toast(adw::Toast::new(t!("The next time a device asks, you pick the screen.")));
                    }
                    Err(e) => dialog.add_toast(adw::Toast::new(&glib::markup_escape_text(&fmt::capitalize(&e)))),
                }
            });
        });
        group
    }

    /// The terminal code: what a phone types to open a terminal here (only its hash is
    /// kept, by the service). Without one, no phone can open a terminal.
    fn terminal_group(self: &Rc<Self>, dialog: &adw::PreferencesDialog, code_set: bool) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(t!("Terminal"))
            .description(t!("Phones you allow (on their page) can open a shell on this computer after typing this code. It's checked here; the phone never keeps it."))
            .build();
        let row = adw::ActionRow::builder()
            .title(t!("Terminal Code"))
            .subtitle(if code_set { t!("Set") } else { t!("Not set: no phone can open a terminal") })
            .build();
        row.add_prefix(&gtk::Image::builder().icon_name("utilities-terminal-symbolic").build());
        let set = gtk::Button::builder().label(if code_set { t!("Change…") } else { t!("Set Code…") }).valign(gtk::Align::Center).build();
        row.add_suffix(&set);
        if code_set {
            let off = gtk::Button::builder().label(t!("Turn Off")).valign(gtk::Align::Center).css_classes(["flat", "destructive-label"]).tooltip_text(t!("Forget the code: no phone can open a terminal, and running ones end")).build();
            let (w, dialog, row2, off2, set2) = (self.clone(), dialog.clone(), row.clone(), off.clone(), set.clone());
            off.connect_clicked(move |_| {
                let Some(c) = w.client() else { return };
                let call = w.bridge.call(&c, |c| async move { c.set_terminal_code(None).await });
                let (w, dialog, row, off, set) = (w.clone(), dialog.clone(), row2.clone(), off2.clone(), set2.clone());
                glib::spawn_future_local(async move {
                    match call.await {
                        Ok(()) => {
                            row.set_subtitle(t!("Not set: no phone can open a terminal"));
                            set.set_label(t!("Set Code…"));
                            off.set_visible(false);
                        }
                        Err(e) => dialog.add_toast(adw::Toast::new(&glib::markup_escape_text(&fmt::capitalize(&e)))),
                    }
                    w.refresh_terminals();
                });
            });
            row.add_suffix(&off);
        }
        let (w, dialog, r) = (self.clone(), dialog.clone(), row.clone());
        set.connect_clicked(move |b| {
            let (dialog2, r, b) = (dialog.clone(), r.clone(), b.clone());
            w.ask_terminal_code(&dialog, move |result| match result {
                Ok(()) => {
                    r.set_subtitle(t!("Set"));
                    b.set_label(t!("Change…"));
                    dialog2.add_toast(adw::Toast::new(t!("Terminal code set")));
                }
                Err(e) => dialog2.add_toast(adw::Toast::new(&glib::markup_escape_text(&fmt::capitalize(&e)))),
            });
        });
        group.add(&row);
        group
    }

    /// Ask for a new terminal code (twice) over `parent` and save it; `done` hears how
    /// that went (not called when cancelled).
    pub(super) fn ask_terminal_code(self: &Rc<Self>, parent: &impl IsA<gtk::Widget>, done: impl Fn(Result<(), String>) + 'static) {
        let ask = adw::AlertDialog::new(Some(t!("Terminal Code")), Some(t!("At least 6 characters (digits are easiest on a phone). Your phone asks for it every time a terminal opens; only this computer knows it. 5 wrong in a row lock it out.")));
        let first = gtk::PasswordEntry::builder().show_peek_icon(true).placeholder_text(t!("New code")).activates_default(true).build();
        let again = gtk::PasswordEntry::builder().show_peek_icon(true).placeholder_text(t!("Same code again")).activates_default(true).build();
        let fields = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        fields.append(&first);
        fields.append(&again);
        ask.set_extra_child(Some(&fields));
        ask.add_responses(&[("cancel", t!("Cancel")), ("save", t!("Save"))]);
        ask.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        ask.set_default_response(Some("save"));
        ask.set_close_response("cancel");
        ask.set_response_enabled("save", false);
        let check = {
            let (ask, first, again) = (ask.clone(), first.clone(), again.clone());
            move || {
                let (a, b) = (first.text(), again.text());
                ask.set_response_enabled("save", a.chars().count() >= 6 && a == b);
            }
        };
        let c1 = check.clone();
        first.connect_changed(move |_| c1());
        again.connect_changed(move |_| check());
        let w = self.clone();
        let done = Rc::new(done);
        ask.connect_response(Some("save"), move |_, _| {
            let code = first.text().to_string();
            let Some(c) = w.client() else { return };
            let call = w.bridge.call(&c, move |c| async move { c.set_terminal_code(Some(code)).await });
            let (w, done) = (w.clone(), done.clone());
            glib::spawn_future_local(async move {
                done(call.await);
                w.refresh_terminals();
            });
        });
        ask.present(Some(parent));
    }

    /// "Check for Updates": once a day from the website, nothing about the user sent.
    fn updates_group(self: &Rc<Self>, dialog: &adw::PreferencesDialog, info: UpdateInfo) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder().title(t!("Updates")).description(t!("This is Linksy {version}.", version = info.current)).build();
        let row = adw::SwitchRow::builder()
            .title(t!("Check for Updates"))
            .subtitle(t!("Once a day, Linksy asks linksy.diary.mg for the newest version. Nothing about you or your devices is sent."))
            .active(info.prefs.check)
            .build();
        let (w, dialog) = (self.clone(), dialog.clone());
        row.connect_active_notify(move |row| {
            let Some(c) = w.client() else { return };
            let next = UpdatePrefs { check: row.is_active() };
            let call = w.bridge.call(&c, move |c| async move { c.set_update_prefs(next).await });
            let (w, dialog) = (w.clone(), dialog.clone());
            glib::spawn_future_local(async move {
                if let Err(e) = call.await {
                    dialog.add_toast(adw::Toast::new(&glib::markup_escape_text(&fmt::capitalize(&e))));
                }
                w.refresh_updates();
            });
        });
        group.add(&row);
        group
    }

    /// "Import Photos From Phones": on or off; they go to Pictures/<phone name>.
    fn photo_import_row(self: &Rc<Self>, dialog: &adw::PreferencesDialog, prefs: PhotoPrefs) -> adw::SwitchRow {
        let row = adw::SwitchRow::builder()
            .title(t!("Import Photos From Phones"))
            .subtitle(t!("New photos from your phone go to {folder}, in a folder named after it, without asking.", folder = fmt::home_relative(&prefs.folder)))
            .active(prefs.import)
            .build();
        let (w, dialog) = (self.clone(), dialog.clone());
        row.connect_active_notify(move |row| {
            let Some(c) = w.client() else { return };
            let next = PhotoPrefs { import: row.is_active(), ..prefs.clone() };
            let call = w.bridge.call(&c, move |c| async move { c.set_photo_prefs(next).await });
            let dialog = dialog.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = call.await {
                    dialog.add_toast(adw::Toast::new(&glib::markup_escape_text(&fmt::capitalize(&e))));
                }
            });
        });
        row
    }
}
