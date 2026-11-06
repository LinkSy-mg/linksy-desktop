// Why Can't I See My Phone?: what the service finds wrong from this computer (`doctor`), and what
// Finish Setup says of the firewall, one card each, with a button where Linksy can act on it.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    required property var app
    title: app.t("Why Can't I See My Phone?")

    // The service's findings, and the firewall's (null: fine, or not known yet)
    property var found: null
    property var firewall: null
    property bool firewallDone: false
    readonly property bool waiting: found === null || !firewallDone
    readonly property var findings: {
        const list = (found ?? []).filter(f => !(firewall && f.id === "fine"));
        return firewall ? [firewall, ...list] : list;
    }

    Component.onCompleted: {
        backend.checkFirewall();
        app.request("doctor", {}, (ok, got) => {
            page.found = ok ? got : [{ id: "service", title: app.t("Linksy's service isn't answering"), detail: String(got), action: null }];
        });
    }

    Connections {
        target: backend
        function onFirewallChecked(json) {
            page.firewall = json === "" ? null : JSON.parse(json);
            page.firewallDone = true;
        }
    }

    function act(action) {
        if (!action) return;
        app.pageStack.layers.pop();
        switch (action["do"]) {
        case "finish_setup":
            app.openSetup();
            break;
        case "connect_by_address":
            app.openInvite(action.address);
            break;
        case "usb_connect":
            app.request("usb_connect", { phone: action.phone }, (ok, err) => { if (!ok) app.toast(err) });
            break;
        }
    }

    function label(action) {
        switch (action?.["do"]) {
        case "finish_setup": return app.t("Finish Setup…");
        case "connect_by_address": return app.t("Connect by Address…");
        case "usb_connect": return app.t("Connect by USB Cable…");
        }
        return "";
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            text: app.t("Linksy looks for what could keep a phone from finding this computer, and says what to do about it.")
        }

        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            visible: page.waiting
            QQC2.BusyIndicator { running: page.waiting }
            QQC2.Label { text: app.t("Looking…"); opacity: 0.7 }
        }

        Repeater {
            model: page.waiting ? [] : page.findings
            delegate: Kirigami.AbstractCard {
                id: card
                required property var modelData
                Layout.fillWidth: true
                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing
                    Kirigami.Heading {
                        Layout.fillWidth: true
                        level: 4
                        wrapMode: Text.Wrap
                        text: card.modelData.title
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        opacity: 0.7
                        text: card.modelData.detail
                    }
                    QQC2.Button {
                        visible: !!card.modelData.action
                        text: page.label(card.modelData.action)
                        onClicked: page.act(card.modelData.action)
                    }
                }
            }
        }
    }
}
