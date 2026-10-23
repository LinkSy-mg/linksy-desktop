// Connect by USB Cable: the steps from "plug it in" to "connected", each showing whether
// it's done, under way or waiting, live, with what to do next (docs/protocol.md §4.11). The
// service does the work (`usb_status`, `usb_connect`); this only follows along.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    required property var app
    title: app.t("Connect by USB Cable")

    property var status: null
    property string ready: backend.usbReady()
    property bool allowing: false

    readonly property var cabled: app.devices.find(d => d.trusted && d.connected && d.usb) ?? null
    readonly property bool paired: app.devices.some(d => d.trusted && d.device.os === "android")
    // The phone furthest along
    readonly property var phone: {
        const rank = { accessory: 3, switching: 2, plugged: 1, failed: 1, declined: 0 };
        let best = null;
        for (const p of status?.phones ?? []) if (!best || rank[p.state] > rank[best.state]) best = p;
        return best;
    }

    function poll() {
        ready = backend.usbReady();
        app.request("usb_status", {}, (ok, s) => { page.status = ok ? s : null });
    }

    function connect() {
        app.request("usb_connect", { phone: phone.key }, (ok, err) => { if (!ok) app.toast(err) });
    }

    Timer {
        interval: 1000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: page.poll()
    }

    Connections {
        target: backend
        function onSetupFinished(json) {
            if (!page.allowing) return;
            page.allowing = false;
            const r = JSON.parse(json);
            if (r.error) app.toast(r.error);
            else if (r.restart) app.toast(app.t("Restart your computer, then try again."));
            page.poll();
        }
    }

    // Each step: [state, title, text, button, action]; state is done, busy or todo
    readonly property var steps: {
        const t = app.t, c = cabled, p = phone;
        const access = c || !ready ? ["done", t("Let Linksy Use the Cable"), t("Allowed")]
                                   : ["todo", t("Let Linksy Use the Cable"), ready, allowing ? "" : t("Allow"), "allow"];
        let plug;
        if (status && !status.watching && !c) plug = ["todo", t("Plug In Your Phone"), t("Linksy's service isn't watching USB. Restart Linksy, then try again.")];
        else if (c) plug = ["done", t("Plug In Your Phone"), t("{name} is plugged in", { name: c.device.name })];
        else if (p) plug = ["done", t("Plug In Your Phone"), t("{name} is plugged in", { name: p.name })];
        else plug = ["busy", t("Plug In Your Phone"), t("Unlock the phone and plug it in. Use a cable that carries data: some only charge.")];
        let sw;
        const offPlug = t("File transfer by cable stops until you unplug it.");
        if (c || p?.state === "accessory") sw = ["done", t("Switch It to Linksy"), t("Switched: file transfer by cable is off until you unplug it")];
        else if (p?.state === "switching") sw = ["busy", t("Switch It to Linksy"), t("Switching…")];
        else if (p?.state === "failed") sw = ["todo", t("Switch It to Linksy"), t("Didn't work: {error}. Unplug the phone, plug it in again, and try again.", { error: p.error ?? "" }), ready ? "" : t("Try Again"), "connect"];
        else if (p) sw = ["todo", t("Switch It to Linksy"), p.state === "declined" ? t("You chose Not This Phone before. File transfer by cable stops until you unplug it.") : offPlug, ready ? "" : t("Connect"), "connect"];
        else sw = ["todo", t("Switch It to Linksy"), offPlug];
        let open;
        if (c) open = ["done", t("Open Linksy on the Phone"), t("Opened")];
        else if (p?.state === "accessory") open = ["busy", t("Open Linksy on the Phone"), t("The phone asks which app to open: choose Linksy, then Always. Nothing asked? Unlock the phone, or unplug it and plug it in again.")];
        else open = ["todo", t("Open Linksy on the Phone"), t("The phone asks which app to open: choose Linksy, then Always.")];
        const done = c ? ["done", t("Connected"), t("{name} is connected by USB. Start the webcam or microphone from its page.", { name: c.device.name })]
                       : ["todo", t("Connected"), t("Your phone's page then says Connected by USB.")];
        return [access, plug, sw, open, done];
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: !page.paired
            type: Kirigami.MessageType.Information
            text: app.t("Pair your phone over Wi-Fi first; the cable then connects it.")
        }

        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            text: app.t("A faster webcam and microphone, and no Wi-Fi needed. Nothing to turn on in the phone's settings.")
        }

        Repeater {
            model: page.steps
            delegate: Kirigami.AbstractCard {
                id: card
                required property var modelData
                required property int index
                readonly property string step: modelData[0]
                Layout.fillWidth: true
                contentItem: RowLayout {
                    spacing: Kirigami.Units.largeSpacing
                    // Its number (waiting), a spinner (under way) or a check (done)
                    Item {
                        Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                        Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                        Layout.alignment: Qt.AlignTop
                        QQC2.Label {
                            anchors.centerIn: parent
                            visible: card.step === "todo"
                            text: card.index + 1
                            font.bold: true
                        }
                        QQC2.BusyIndicator {
                            anchors.fill: parent
                            visible: card.step === "busy"
                            running: visible
                        }
                        Kirigami.Icon {
                            anchors.fill: parent
                            visible: card.step === "done"
                            source: "checkmark"
                            color: Kirigami.Theme.positiveTextColor
                            isMask: true
                        }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        // Steps still waiting are dimmed, not their buttons
                        opacity: card.step === "todo" ? 0.6 : 1
                        Kirigami.Heading {
                            Layout.fillWidth: true
                            level: 4
                            text: card.modelData[1]
                            wrapMode: Text.Wrap
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: card.modelData[2]
                            wrapMode: Text.Wrap
                            opacity: 0.7
                        }
                    }
                    QQC2.Button {
                        visible: (card.modelData[3] ?? "") !== ""
                        text: card.modelData[3] ?? ""
                        onClicked: {
                            if (card.modelData[4] === "allow") { page.allowing = true; backend.runSetup(JSON.stringify(["usb"])) }
                            else page.connect();
                        }
                    }
                }
            }
        }
    }
}
