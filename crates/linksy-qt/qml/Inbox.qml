// A phone's inbox, kept by its page: the conversations made from its message notifications,
// its other notifications, the conversation open, and how the Inbox tab filters and sorts them
import QtQuick

QtObject {
    id: inbox
    required property var app
    // The phone, "" for none
    property string phone: ""
    // While the Inbox tab shows
    property bool shown: false
    // Newest first
    property var conversations: []
    property var alerts: []
    property bool loaded: false
    property string error: ""
    property string open: ""
    property var thread: []
    // Earlier messages asked of the phone; when they land, the ones shown stay in place
    property bool fetchingOlder: false
    property bool keepPlace: false
    property bool sending: false
    property string show: "all"
    property bool oldestFirst: false
    property string search: ""
    readonly property var current: conversations.find(c => c.id === open) ?? null
    // One of the other notifications shown in full instead, by key; gone, it shows no more
    property string alert: ""
    readonly property var currentAlert: alerts.find(a => a.key === alert) ?? null
    // Unread conversations and other notifications, for the tab's badge
    readonly property int count: conversations.reduce((n, c) => n + Math.min(c.unread, 1), 0) + alerts.length

    signal sent()

    function capitalize(text) {
        return text.charAt(0).toUpperCase() + text.slice(1);
    }

    function reset(id) {
        phone = id;
        conversations = [];
        alerts = [];
        loaded = false;
        error = "";
        open = "";
        alert = "";
        thread = [];
        if (id) load();
    }

    function load() {
        const id = phone;
        if (!id) return;
        app.request("inbox", {}, (ok, list) => {
            if (id !== inbox.phone) return;
            if (!ok) {
                inbox.error = inbox.capitalize(list);
                inbox.loaded = true;
                return;
            }
            // An older service has no other notifications to give
            inbox.app.request("alerts", {}, (ok2, alerts) => {
                if (id !== inbox.phone) return;
                inbox.conversations = list.filter(c => c.phone === id);
                inbox.alerts = ok2 ? alerts.filter(a => a.phone === id) : [];
                inbox.error = "";
                inbox.loaded = true;
                inbox.openWanted();
            });
        });
    }

    // Development aid (screenshots): LINKSY_SCREENSHOT_THREAD=<title> opens one
    function openWanted() {
        const title = backend.env("LINKSY_SCREENSHOT_THREAD");
        const c = title && !open && shown ? conversations.find(c => c.title === title) : null;
        if (c) openConversation(c.id);
    }

    // A conversation changed: read or answerable again with nothing newer, it keeps its place
    function changed(c) {
        if (c.phone !== phone) return;
        const before = conversations.find(o => o.id === c.id);
        const newer = !before || before.at_ms !== c.at_ms || JSON.stringify(before.last) !== JSON.stringify(c.last);
        const same = conversations.findIndex(o => o.id === c.id && o.at_ms === c.at_ms);
        const list = conversations.filter(o => o.id !== c.id);
        let at = same >= 0 ? same : list.findIndex(o => o.at_ms <= c.at_ms);
        if (at < 0) at = list.length;
        list.splice(at, 0, c);
        conversations = list;
        if (c.id !== open || !shown) return;
        if (app.active) markRead(c);
        if (newer) loadThread();
    }

    // One open at a time: a conversation or another notification
    function openAlert(key) {
        open = "";
        thread = [];
        alert = key;
    }

    // Back to the list: neither open
    function close() {
        open = "";
        alert = "";
    }

    function openConversation(id) {
        alert = "";
        open = id;
        keepPlace = false;
        thread = [];
        const c = current;
        if (!c) return;
        markRead(c);
        loadThread();
    }

    function markRead(c) {
        if (c.unread > 0) app.request("inbox_read", { conversation: c.id });
    }

    function loadThread() {
        const id = open;
        if (!id) return;
        app.request("inbox_thread", { conversation: id }, (ok, list) => {
            if (id !== inbox.open) return;
            if (!ok) return inbox.app.toast(inbox.capitalize(list));
            inbox.thread = list;
        });
    }

    // Earlier messages from the phone; it answers the whole conversation
    function loadOlder() {
        const id = open;
        if (!id || fetchingOlder) return;
        fetchingOlder = true;
        app.request("inbox_older", { conversation: id }, (ok, list) => {
            inbox.fetchingOlder = false;
            if (id !== inbox.open) return;
            if (!ok) return inbox.app.toast(inbox.capitalize(list));
            inbox.keepPlace = true;
            inbox.thread = list;
        });
    }

    function reply(text) {
        if (!text || !open || sending) return;
        sending = true;
        app.request("inbox_reply", { conversation: open, text: text }, (ok, err) => {
            inbox.sending = false;
            if (!ok) return inbox.app.toast(inbox.app.t("Not sent: {error}", { error: inbox.capitalize(err) }));
            inbox.sent();
        });
    }

    // One of the phone's other notifications dismissed there, or all of them (no key)
    function dismiss(key) {
        const id = phone;
        app.request("dismiss_alert", { phone: id, key: key ?? null }, (ok, err) => {
            if (!ok) inbox.app.toast(inbox.capitalize(err));
            if (id === inbox.phone) inbox.load();
        });
    }
}
