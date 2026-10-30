// Linksy for KDE Plasma: the window, what it knows about the service, and the questions
// the service asks (pairing, files, sessions).
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Dialogs
import org.kde.kirigami as Kirigami
import mg.diary.linksy

Kirigami.ApplicationWindow {
    id: root
    title: "Linksy"
    width: Kirigami.Units.gridUnit * 56
    height: Kirigami.Units.gridUnit * 40
    minimumWidth: Kirigami.Units.gridUnit * 22
    minimumHeight: Kirigami.Units.gridUnit * 24

    // ---- what the service told us ----
    property var devices: []
    property var status: null
    property var settings: null
    property var lockPrefs: ({ devices: [], grace_s: 60 })
    property var terminal: ({ code_set: false, allowed: [], locked: [], running: [] })
    property string selectedId: ""
    property var history: []
    property var progress: ({})
    // Transfers for the corner panel, any device: from their start to a few seconds after
    // the last one ends
    property var corner: []
    // The Transfers page: on its way (oldest first), ended lately, and direct links (id: up)
    readonly property bool transfersOpen: pageStack.lastItem?.objectName === "transfers"
    property var moving: []
    property var recent: []
    property var direct: ({})
    property var pictures: ({})
    property var sessions: []
    property var pending: ({})
    // Finish Setup's list (linksy-setup), null until checked
    property var setup: null
    // A newer Linksy, when the service found one: { version, notes }
    property var update: null

    // A session ended (a viewer showing it closes its picture)
    signal sessionEnded(string id)
    // A camera used here says what it can do and how it's set
    signal cameraState(string session, var state)

    readonly property var selected: devices.find(d => d.device.id === selectedId) ?? null

    Backend {
        id: backend
        onReply: (id, ok, json) => {
            const done = root.pending[id];
            delete root.pending[id];
            if (done) done(ok, JSON.parse(json));
        }
        onEvent: json => root.handleEvent(JSON.parse(json))
        onHistory: json => root.historyUpdated(JSON.parse(json))
        onConnectedChanged: {
            if (!connected) {
                root.corner = [];
                root.moving = [];
                return;
            }
            root.refreshAll();
            // Development aid: open a screen for scripts/qt-shot.sh
            const open = backend.env("LINKSY_SCREENSHOT_OPEN");
            if (open === "invite") root.openInvite();
            if (open === "prefs") pageStack.pushDialogLayer(Qt.resolvedUrl("PrefsPage.qml"), { app: root });
            if (open === "screen") openScreen.start();
            if (open === "setup") root.openSetup();
            if (open === "usb") root.openUsb();
            if (open === "news") whatsNew.showAnyTime();
            if (open === "upgrade") root.showUpgrade();
            if (open === "camera") root.previewCamera("", "Pixel");
        }
        onOffline: why => { if (why) root.toast(why) }
        onSetupChecked: json => root.setup = JSON.parse(json)
    }

    // `english` in the language in use, with {name}-style holes filled from `args`
    function t(english, args) {
        let out = backend.tr(english);
        for (const k in args) out = out.split("{" + k + "}").join(args[k]);
        return out;
    }

    function request(cmd, fields, done) {
        const id = backend.call(JSON.stringify(Object.assign({ cmd: cmd }, fields ?? {})));
        if (done) root.pending[id] = done;
        return id;
    }

    // "3.8 MB", as the GTK app writes sizes
    function size(bytes) {
        const units = [t("bytes"), t("KB"), t("MB"), t("GB"), t("TB")];
        if (bytes < 1000) return bytes === 1 ? t("1 byte") : t("{bytes} bytes", { bytes: bytes });
        let v = bytes, unit = 0;
        while (v >= 1000 && unit < units.length - 1) { v /= 1000; unit++; }
        return `${v < 10 ? v.toFixed(1) : v.toFixed(0)} ${units[unit]}`;
    }

    // How a device is reachable, as the GTK app's sidebar says it, with its battery
    function statusLine(d) {
        let text;
        if (!d.trusted) text = t("Nearby · Not paired");
        else if (d.connected) text = d.usb ? t("Connected by USB") : d.away ? t("Connected away from home") : t("Connected");
        else text = d.nearby ? t("Nearby") : t("Not nearby");
        const b = (d.connected || d.nearby) ? d.battery : null;
        if (b) text += " · " + (b.charging ? t("{level}%, charging", { level: b.level }) : `${b.level}%`);
        return text;
    }

    // "Linux laptop", "Android phone"
    function kindName(info) {
        const kind = t(info.kind);
        const os = { linux: "Linux", android: "Android", macos: "macOS", windows: "Windows", ios: "iOS" }[info.os] ?? info.os;
        return os ? t("{os} {kind}", { os: os, kind: kind }) : kind.charAt(0).toUpperCase() + kind.slice(1);
    }

    function remaining(s) {
        if (s < 5) return t("a few seconds left");
        if (s < 60) return t("{s} seconds left", { s: Math.round(s) });
        if (s < 3600) { const n = Math.round(s / 60); return n === 1 ? t("1 minute left") : t("{n} minutes left", { n: n }) }
        const n = Math.round(s / 3600);
        return n === 1 ? t("1 hour left") : t("{n} hours left", { n: n });
    }

    // "1.2 MB of 3.8 MB · 4.1 MB/s · 2 seconds left"
    function progressText(p) {
        let text = t("{done} of {total}", { done: size(p.bytes), total: size(p.total) });
        const [first, last] = [p.samples[0], p.samples[p.samples.length - 1]];
        const speed = first && last && last[0] > first[0] ? (last[1] - first[1]) / ((last[0] - first[0]) / 1000) : 0;
        if (speed > 0 && p.bytes < p.total)
            text += ` · ${t("{size}/s", { size: size(speed) })} · ${remaining((p.total - p.bytes) / speed)}`;
        return text;
    }

    // Show any second display another computer started here, or screen the tray asked to
    // see (also right after start-up, when the service opened the app for it)
    property var showingDisplays: []
    function joinDisplays() {
        request("pending_sessions", {}, (ok, list) => {
            if (!ok) return;
            for (const p of list.filter(p => (p.kind === "display" || p.kind === "screen") && !showingDisplays.includes(p.session))) {
                showingDisplays = showingDisplays.concat([p.session]);
                present();
                const stream = { session: p.session, socket: p.socket };
                pageStack.layers.push(Qt.resolvedUrl("ScreenPage.qml"), p.kind === "display" ? { app: root, device: p.peer.id, name: p.peer.name, display: stream }
                                                                                             : { app: root, device: p.peer.id, name: p.peer.name, started: stream });
            }
        });
    }

    function viewScreen(id) {
        pageStack.layers.push(Qt.resolvedUrl("ScreenPage.qml"), { app: root, device: id, name: deviceName(id) });
    }

    // What apps get from a phone's camera in use here, with its controls
    function previewCamera(session, name) {
        pageStack.layers.push(Qt.resolvedUrl("CameraPage.qml"), { app: root, session: session, name: backend.cameraName(name) });
    }

    function openUsb() {
        pageStack.pushDialogLayer(Qt.resolvedUrl("UsbPage.qml"), { app: root });
    }

    function openSetup() {
        pageStack.pushDialogLayer(Qt.resolvedUrl("SetupPage.qml"), { app: root });
    }

    function openInvite() {
        inviteDialog.open();
    }

    // Bring the window forward for a question the service asks
    function present() {
        root.show();
        if (root.visibility === Window.Minimized) root.showNormal();
        root.raise();
        root.requestActivate();
    }

    function toast(text) {
        showPassiveNotification(text);
    }

    // What a device can do for others; older versions don't say, so assume as the GTK app does
    function can(info, feature) {
        if (info.features && info.features.length) return info.features.includes(feature);
        return !(info.os === "android" && ["sms", "notifications", "media_state", "control", "clipboard_file", "clipboard_watch", "microphone", "locate"].includes(feature));
    }

    function refreshAll() {
        refreshDevices();
        request("status", {}, (ok, s) => { if (ok) root.status = s });
        request("settings", {}, (ok, s) => { if (ok) root.settings = s });
        request("lock_prefs", {}, (ok, p) => { if (ok) root.lockPrefs = p });
        request("terminal_status", {}, (ok, s) => { if (ok) root.terminal = s });
        request("active_sessions", {}, (ok, list) => { if (ok) root.sessions = list.filter(s => s.sharing || ["camera", "microphone"].includes(s.kind)) });
        joinDisplays();
        refreshHistory();
        refreshUpdates();
        // Transfers already running, newest first
        request("history", {}, (ok, list) => {
            if (!ok) return;
            const files = list.filter(e => e.what.kind === "files");
            root.corner = files.filter(e => e.status.state === "in_progress");
            root.moving = files.filter(e => e.status.state === "in_progress").reverse().map(e => movingFrom(e));
            root.recent = files.filter(e => e.status.state !== "in_progress").slice(0, 40);
        });
    }

    // The service checks once a day; ask it now and then what it found
    function refreshUpdates() {
        request("updates", {}, (ok, u) => { root.update = ok && u.prefs.check && u.latest && newerThanThis(u.latest) ? { version: u.latest, notes: u.notes } : null });
    }

    // A service still on the version before an update knows only older ones
    function newerThanThis(version) {
        const a = version.split(".").map(Number), b = backend.version().split(".").map(Number);
        for (let i = 0; i < Math.max(a.length, b.length); i++) {
            if ((a[i] || 0) !== (b[i] || 0)) return (a[i] || 0) > (b[i] || 0);
        }
        return false;
    }

    Timer {
        interval: 30 * 60 * 1000
        running: backend.connected
        repeat: true
        onTriggered: root.refreshUpdates()
    }

    function refreshDevices() {
        request("devices", {}, (ok, list) => {
            if (!ok) return;
            root.devices = list;
            for (const d of list.filter(d => d.trusted && !(d.device.id in root.pictures))) loadPicture(d.device.id);
            if (!root.selectedId) {
                const first = list.find(d => d.trusted);
                if (first) root.select(first.device.id);
            }
        });
    }

    function loadPicture(id) {
        request("device_picture", { device: id }, (ok, path) => {
            const p = Object.assign({}, root.pictures);
            // A new name each time, so a changed picture isn't served from the cache
            p[id] = ok && path ? "file://" + path + "?" + Date.now() : "";
            root.pictures = p;
        });
    }

    function refreshHistory() {
        if (!selectedId) return;
        request("history", { with: selectedId }, (ok, list) => { if (ok) root.history = list });
    }

    function select(id) {
        if (transfersOpen) while (pageStack.depth > 1) pageStack.pop();
        selectedId = id;
        history = [];
        refreshHistory();
        if (pageStack.depth < 2) pageStack.push(Qt.resolvedUrl("DevicePage.qml"), { app: root });
        else pageStack.currentIndex = 1;
    }

    function deviceName(id) {
        return devices.find(d => d.device.id === id)?.device.name ?? "";
    }

    function handleEvent(ev) {
        switch (ev.type) {
        case "device_found": case "device_lost": case "device_updated":
        case "connected": case "disconnected": case "battery":
            refreshDevices();
            break;
        case "pairing_request":
            pairDialog.ask(ev.request, ev.device, ev.code, ev.incoming, ev.older);
            break;
        case "paired":
            pairDialog.close();
            inviteDialog.close();
            toast(t("Paired with {name}", { name: ev.device.name }));
            refreshDevices();
            select(ev.device.id);
            break;
        case "pairing_failed":
            pairDialog.close();
            if (ev.reason.includes("other device")) toast(t("{name} didn't confirm the code", { name: ev.device.name }));
            break;
        case "transfer_request":
            offerDialog.ask(ev.transfer, ev.from, ev.files, ev.total);
            break;
        case "direct_wanted": case "direct_linked": case "direct_ended": case "direct": {
            const id = ev.type === "direct" ? ev.from.id : ev.id;
            const d = Object.assign({}, direct);
            if (ev.type === "direct_ended") delete d[id];
            else if (ev.type === "direct_linked") d[id] = true;
            else if (!(id in d)) d[id] = false;
            direct = d;
            break;
        }
        case "transfer_started": {
            const m = { id: ev.transfer, peer: ev.peer.id, peer_name: ev.peer.name, incoming: ev.direction === "incoming",
                        imported: ev["import"] === true, total: ev.total, files: ev.files.map(f => ({ name: f.path, size: f.size })) };
            const list = moving.filter(x => x.id !== ev.transfer);
            const i = moving.findIndex(x => x.id === ev.transfer);
            if (i >= 0) list.splice(i, 0, m); else list.push(m);
            moving = list;
        }
            if (offerDialog.transfer === ev.transfer) offerDialog.close();
            setProgress(ev.transfer, 0, ev.total);
            break;
        case "transfer_progress":
            setProgress(ev.transfer, ev.bytes, ev.total);
            break;
        case "transfer_complete": case "transfer_failed":
            if (offerDialog.transfer === ev.transfer) offerDialog.close();
            setProgress(ev.transfer, -1, 0);
            break;
        case "settings_changed":
            settings = ev.settings;
            request("status", {}, (ok, s) => { if (ok) root.status = s });
            break;
        case "session_request":
            sessionDialog.ask(ev.request, ev.from, ev.kind);
            break;
        case "session_started":
            if (sessionDialog.request === ev.session) sessionDialog.close();
            if (ev.sharing || ["camera", "microphone"].includes(ev.kind))
                sessions = sessions.concat([{ session: ev.session, peer: ev.peer, kind: ev.kind, sharing: ev.sharing }]);
            else if (ev.kind === "display" || ev.kind === "screen") joinDisplays();
            request("terminal_status", {}, (ok, s) => { if (ok) root.terminal = s });
            break;
        case "session_ended":
            sessions = sessions.filter(s => s.session !== ev.session);
            sessionEnded(ev.session);
            request("terminal_status", {}, (ok, s) => { if (ok) root.terminal = s });
            break;
        case "camera_state":
            cameraState(ev.session, ev.state);
            break;
        case "picture":
            loadPicture(ev.from.id);
            break;
        case "notification":
            toast(ev.body ? `${ev.from.name}: ${ev.title} — ${ev.body}` : `${ev.from.name}: ${ev.title}`);
            break;
        case "clipboard":
            if (root.active) backend.setClipboard(ev.text);
            toast(t("Copied from {name}", { name: ev.from.name }));
            break;
        case "ring":
            if (ev.on) toast(t("{name} is looking for this computer", { name: ev.from.name }));
            break;
        }
    }

    function historyUpdated(entry) {
        cornerUpdated(entry);
        if (entry.what.kind === "files") {
            if (entry.status.state === "in_progress") {
                if (!moving.some(m => m.id === entry.id)) moving = moving.concat([movingFrom(entry)]);
            } else {
                moving = moving.filter(m => m.id !== entry.id);
                recent = [entry].concat(recent.filter(r => r.id !== entry.id)).slice(0, 40);
            }
        }
        if (entry.peer !== selectedId) return;
        const list = history.slice();
        const i = list.findIndex(e => e.id === entry.id);
        if (i >= 0) list[i] = entry; else list.unshift(entry);
        history = list;
    }

    function movingFrom(e) {
        const sizes = e.what.sizes ?? [];
        return { id: e.id, peer: e.peer, peer_name: e.peer_name, incoming: e.direction === "incoming", imported: e.what["import"] === true,
                 total: e.what.total, files: e.what.files.map((f, i) => ({ name: f, size: sizes.length === e.what.files.length ? sizes[i] : null })) };
    }

    function openTransfers() {
        if (transfersOpen) return;
        selectedId = "";
        while (pageStack.depth > 1) pageStack.pop();
        pageStack.push(Qt.resolvedUrl("TransfersPage.qml"), { app: root });
    }

    // A transfer started, moved on or ended; one that ends before it was shown isn't news
    function cornerUpdated(entry) {
        if (entry.what.kind !== "files") return;
        const list = corner.slice();
        const i = list.findIndex(e => e.id === entry.id);
        if (i >= 0) list[i] = entry;
        else if (entry.status.state === "in_progress") list.unshift(entry);
        else return;
        corner = list;
        if (list.some(e => e.status.state === "in_progress")) cornerLinger.stop();
        else cornerLinger.restart();
    }

    // Away now, with the ended transfers
    function dismissCorner() {
        cornerLinger.stop();
        corner = corner.filter(e => e.status.state === "in_progress");
    }

    Timer {
        id: cornerLinger
        interval: 8000
        onTriggered: root.dismissCorner()
    }

    // bytes < 0: done, forget it. Keeps a few seconds of samples for the speed.
    function setProgress(id, bytes, total) {
        const p = Object.assign({}, progress);
        if (bytes < 0) delete p[id];
        else {
            const now = Date.now();
            const samples = (p[id]?.samples ?? []).concat([[now, bytes]]).filter(s => now - s[0] < 3000);
            p[id] = { bytes: bytes, total: total, samples: samples };
        }
        progress = p;
    }

    function sendFiles(id, urls) {
        sendPaths(id, urls.map(u => decodeURIComponent(u.toString().replace(/^file:\/\//, ""))));
    }

    function sendPaths(id, paths) {
        if (!paths.length) return;
        request("send", { to: id, paths: paths }, (ok, err) => { if (!ok) toast(err) });
    }

    // Files from the file manager (Send with Linksy, Open With): the island asks where to
    property var offered: []
    function offerFiles(paths) {
        offered = paths;
        toast(t("Pick a device to send to"));
    }

    function sessionTitle(s) {
        const name = s.peer.name;
        switch (s.kind) {
        case "camera": return s.sharing ? t("{name} is using your camera", { name: name })
                                        : t("{name}'s camera is on: “{camera}” in your apps", { name: name, camera: backend.cameraName(name) });
        case "microphone": return s.sharing ? t("{name} is using your microphone", { name: name })
                                            : t("{name}'s microphone is on: “{microphone}” in your apps", { name: name, microphone: `Linksy Microphone (${name})` });
        case "display": return t("{name} is your second display", { name: name });
        case "control": return t("{name} is controlling your screen", { name: name });
        case "input": return t("{name} is your touchpad and keyboard", { name: name });
        case "screen": return t("{name} is watching your screen", { name: name });
        default: return t("{name} has a terminal open here", { name: name });
        }
    }

    pageStack.initialPage: DevicesPage { app: root }
    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.ToolBar

    // What's being shared right now, with Stop
    header: ColumnLayout {
        spacing: 0
        Repeater {
            model: root.sessions
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: true
                type: Kirigami.MessageType.Information
                text: root.sessionTitle(modelData)
                actions: Kirigami.Action {
                    text: (modelData.sharing && ["screen", "control", "camera", "microphone"].includes(modelData.kind)) ? root.t("Stop Sharing") : root.t("Stop")
                    icon.name: "media-playback-stop"
                    onTriggered: root.request("end_session", { session: modelData.session })
                }
            }
        }
    }

    Shortcut {
        sequence: "Ctrl+T"
        onActivated: root.openTransfers()
    }

    // Under the dialogs, which open in the same layer later
    CornerPanel {
        parent: root.overlay
        app: root
    }

    // Before the service answers, or when it's gone
    Kirigami.PlaceholderMessage {
        parent: root.overlay
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.gridUnit * 4
        visible: !backend.connected
        icon.name: backend.starting ? "view-refresh" : "network-disconnect"
        text: backend.starting ? root.t("Starting…") : root.t("Linksy Isn't Running")
        explanation: backend.starting ? "" : root.t("The background service lets nearby devices find this one and receives files, even when this window is closed.")
        helpfulAction: Kirigami.Action {
            enabled: !backend.starting
            icon.name: "media-playback-start"
            text: root.t("Start Linksy")
            onTriggered: backend.start()
        }
    }

    // ---- the service's questions ----

    Kirigami.PromptDialog {
        id: pairDialog
        property string request
        property bool incoming
        property string code
        property string peer
        property bool older
        function ask(req, device, c, inc, old) {
            request = req; code = c; incoming = inc; peer = device.name; older = !!old;
            title = inc ? root.t("{name} Wants to Pair", { name: device.name }) : root.t("Pair with {name}", { name: device.name });
            root.present();
            open();
        }
        subtitle: incoming ? root.t("Pair only if {name} shows this same code.", { name: peer })
                           : root.t("Check that {name} shows this same code, then confirm on both devices.", { name: peer })
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: root.t("Pair")
                icon.name: "dialog-ok"
                onTriggered: { root.request("respond", { request: pairDialog.request, accept: true }); pairDialog.close() }
            },
            Kirigami.Action {
                text: root.t("Cancel")
                icon.name: "dialog-cancel"
                onTriggered: { root.request("respond", { request: pairDialog.request, accept: false }); pairDialog.close() }
            }
        ]
        ColumnLayout {
            Kirigami.Heading {
                Layout.alignment: Qt.AlignHCenter
                horizontalAlignment: Text.AlignHCenter
                level: 1
                font.letterSpacing: 4
                font.family: "monospace"
                text: pairDialog.code.length === 6 ? pairDialog.code.slice(0, 3) + " " + pairDialog.code.slice(3) : pairDialog.code
            }
            QQC2.Label {
                visible: pairDialog.older
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                opacity: 0.7
                font: Kirigami.Theme.smallFont
                text: root.t("Updating Linksy on {name} makes pairing even safer.", { name: pairDialog.peer })
            }
        }
    }

    Kirigami.PromptDialog {
        id: offerDialog
        property string transfer
        property string from
        property var files: []
        property real total: 0
        function ask(id, device, f, tot) {
            transfer = id; from = device.id; files = f; total = tot;
            title = root.t("{name} Wants to Share", { name: device.name });
            root.present();
            open();
        }
        subtitle: files.length === 1 ? `${files[0].path} (${root.size(total)})`
                                     : `${files.slice(0, 3).map(f => f.path.split("/").pop()).join(", ")}${files.length > 3 ? " " + root.t("and {count} more", { count: files.length - 3 }) : ""} (${root.size(total)})`
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: root.t("Accept")
                icon.name: "document-save"
                onTriggered: { root.request("respond", { request: offerDialog.transfer, accept: true }); offerDialog.close() }
            },
            Kirigami.Action {
                text: root.t("Always Accept")
                onTriggered: {
                    root.request("set_auto_accept", { device: offerDialog.from, on: true }, () => root.refreshDevices());
                    root.request("respond", { request: offerDialog.transfer, accept: true });
                    offerDialog.close();
                }
            },
            Kirigami.Action {
                text: root.t("Decline")
                icon.name: "dialog-cancel"
                onTriggered: { root.request("respond", { request: offerDialog.transfer, accept: false }); offerDialog.close() }
            }
        ]
    }

    Kirigami.PromptDialog {
        id: sessionDialog
        property string request
        property string from
        // Seeing or controlling the screen may be allowed for good (SessionKind::may_always_allow)
        property bool always: false
        function ask(req, device, kind) {
            request = req; from = device.id;
            always = ["screen", "control", "input"].includes(kind);
            const name = device.name;
            const asks = {
                camera: ["{name} Wants to Use Your Camera", "Your camera will turn on and {name} will see it until you stop."],
                display: ["{name} Wants to Use This Screen", "Linksy will fill this screen with {name}'s second display; your mouse and keyboard here will work on it. Press F11 to leave full screen."],
                microphone: ["{name} Wants to Use Your Microphone", "Your microphone will turn on and {name} will hear it until you stop."],
                input: ["{name} Wants to Use Your Mouse and Keyboard", "{name} becomes a touchpad and keyboard for this computer until you stop. It doesn't see your screen."],
                control: ["{name} Wants to Control Your Screen", "{name} will see your screen and use your mouse and keyboard until you stop. You'll pick which screen next."],
                screen: ["{name} Wants to See Your Screen", "{name} will see everything on your screen until you stop sharing. You'll pick which screen next."],
                terminal: ["{name} Wants a Terminal", "The right terminal code was typed on {name}. Its terminal runs commands as you until it's ended, here or on the phone. Linksy asks once a day."]
            };
            const [heading, body] = asks[kind] ?? asks.screen;
            title = root.t(heading, { name });
            subtitle = root.t(body, { name });
            root.present();
            open();
        }
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: root.t("Allow")
                onTriggered: { root.request("respond", { request: sessionDialog.request, accept: true }); sessionDialog.close() }
            },
            Kirigami.Action {
                text: root.t("Always Allow")
                visible: sessionDialog.always
                onTriggered: {
                    root.request("set_allow_screen", { device: sessionDialog.from, on: true }, () => root.refreshDevices());
                    root.request("respond", { request: sessionDialog.request, accept: true });
                    sessionDialog.close();
                }
            },
            Kirigami.Action {
                text: root.t("Don't Allow")
                onTriggered: { root.request("respond", { request: sessionDialog.request, accept: false }); sessionDialog.close() }
            }
        ]
    }

    // Linksy's own password prompt, where no polkit agent runs: the password goes to pkexec
    // on its terminal, nowhere else. Every question gets exactly one answer.
    Kirigami.PromptDialog {
        id: passwordDialog
        property bool answered: true
        property bool again: false
        property string who: ""
        function ask(w, a) {
            who = w; again = a; answered = false;
            passwordField.text = "";
            root.present();
            open();
            passwordField.forceActiveFocus();
        }
        function answer(given) {
            if (answered) return;
            answered = true;
            backend.answerPassword(given ? passwordField.text : "", given);
            passwordField.text = "";
            close();
        }
        title: root.t("Password")
        subtitle: again ? root.t("That password didn't work. Try again.")
                        : who ? root.t("Linksy needs the password of {who} to make changes to this computer.", { who: who })
                              : root.t("Linksy needs your password to make changes to this computer.")
        standardButtons: Kirigami.Dialog.NoButton
        onClosed: answer(false)
        customFooterActions: [
            Kirigami.Action {
                text: root.t("Continue")
                icon.name: "dialog-ok"
                onTriggered: passwordDialog.answer(true)
            },
            Kirigami.Action {
                text: root.t("Cancel")
                icon.name: "dialog-cancel"
                onTriggered: passwordDialog.answer(false)
            }
        ]
        ColumnLayout {
            Kirigami.PasswordField {
                id: passwordField
                Layout.fillWidth: true
                onAccepted: passwordDialog.answer(true)
            }
        }
    }
    Connections {
        target: backend
        function onPasswordWanted(who, again) { passwordDialog.ask(who, again) }
    }

    InviteDialog {
        id: inviteDialog
        app: root
    }

    WhatsNew {
        id: whatsNew
        app: root
    }

    UpgradeDialog {
        id: upgradeDialog
        app: root
    }

    function showUpgrade() {
        upgradeDialog.check();
    }

    function showNews() {
        whatsNew.showAnyTime();
    }

    // logs.txt for a bug report: versions, crashes and the recent log
    FileDialog {
        id: logsDialog
        title: root.t("Save Logs")
        fileMode: FileDialog.SaveFile
        defaultSuffix: "txt"
        selectedFile: "file://" + backend.env("HOME") + "/logs.txt"
        onAccepted: backend.saveLogs(decodeURIComponent(selectedFile.toString().replace(/^file:\/\//, "")))
    }
    Connections {
        target: backend
        function onLogsSaved(error) {
            root.toast(error ? root.t("Couldn't save the logs: {error}", { error: error }) : root.t("Logs saved. Attach logs.txt to your bug report."));
        }
    }

    function saveLogs() {
        logsDialog.open();
    }

    // Development aid: LINKSY_SCREENSHOT=<file.png> saves the window after
    // LINKSY_SCREENSHOT_DELAY ms and quits (scripts/qt-shot.sh)
    Timer {
        id: openScreen
        interval: 1500
        onTriggered: if (root.selectedId) root.viewScreen(root.selectedId)
    }

    Timer {
        id: shot
        interval: Number(backend.env("LINKSY_SCREENSHOT_DELAY")) || 2500
        onTriggered: { backend.saveWindow(backend.env("LINKSY_SCREENSHOT")); Qt.quit() }
    }

    Component.onCompleted: {
        const size = backend.env("LINKSY_SCREENSHOT_SIZE").split("x").map(Number);
        if (size.length === 2 && size[0] > 0) { width = size[0]; height = size[1] }
        backend.start();
        backend.checkSetup();
        // One thing at a time: Finish Setup when something's missing, otherwise, after an
        // update, what's new
        if (backend.offerSetup()) root.openSetup();
        else whatsNew.offer();
        // Crashed since last time: offer the logs once
        if (backend.unseenCrash()) showPassiveNotification(t("Linksy Quit Unexpectedly"), "long", t("Save Logs…"), () => root.saveLogs());
        if (backend.env("LINKSY_SCREENSHOT")) shot.start();
    }
}
