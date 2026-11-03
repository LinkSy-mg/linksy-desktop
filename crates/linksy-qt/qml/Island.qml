// The device list's island, as on the phone: Transfers, Send Files and Add Device in one pill
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Dialogs
import org.kde.kirigami as Kirigami

Rectangle {
    id: island
    required property var app
    readonly property var paired: app.devices.filter(d => d.trusted)
    readonly property bool none: paired.length === 0
    readonly property int files: app.moving.reduce((n, m) => n + m.files.length, 0)
    readonly property real fraction: {
        let bytes = 0, total = 0;
        for (const m of app.moving) {
            const p = app.progress[m.id];
            if (p) { bytes += p.bytes; total += p.total }
        }
        return total > 0 ? bytes / total : 0;
    }
    property string sendTo: ""
    readonly property var offered: app.offered
    onOfferedChanged: offerSoon()
    onPairedChanged: offerSoon()
    function offerSoon() {
        if (offered.length && !none && !card.opened) card.show("send");
    }

    implicitWidth: pill.implicitWidth
    implicitHeight: pill.implicitHeight
    color: "transparent"

    function kindIcon(kind) {
        return ({ phone: "smartphone", tablet: "tablet", laptop: "computer-laptop", desktop: "computer" })[kind] ?? "computer";
    }

    // Copies going on, when there are any, then Send Files (Add Device with nothing paired yet)
    ColumnLayout {
        id: pill
        anchors.fill: parent
        spacing: Kirigami.Units.smallSpacing

        QQC2.ItemDelegate {
            Layout.fillWidth: true
            visible: island.files > 0
            onClicked: island.app.openTransfers()
            contentItem: ColumnLayout {
                spacing: Kirigami.Units.smallSpacing
                RowLayout {
                    spacing: Kirigami.Units.smallSpacing
                    Kirigami.Icon {
                        implicitWidth: Kirigami.Units.iconSizes.small
                        implicitHeight: implicitWidth
                        source: island.app.sym("exchange-positions")
                        fallback: "exchange-positions"
                        color: Kirigami.Theme.linkColor
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: island.app.t("Transfers")
                        elide: Text.ElideRight
                    }
                    QQC2.Label {
                        text: Math.round(island.fraction * 100) + "%"
                        opacity: 0.75
                    }
                }
                QQC2.ProgressBar {
                    Layout.fillWidth: true
                    value: island.fraction
                }
            }
        }

        QQC2.Button {
            id: main
            Layout.fillWidth: true
            icon.name: island.app.sym(island.none ? "list-add" : "document-send")
            text: island.none ? island.app.t("Add Device") : island.app.t("Send Files")
            onClicked: card.show(island.none ? "add" : "send")
        }
    }

    QQC2.Popup {
        id: card
        property string what: ""
        readonly property bool sending: what === "send" && !island.none
        function show(w) {
            if (opened && what === w) return close();
            what = w;
            open();
        }
        x: 0
        y: island.height - height
        width: island.width
        padding: Kirigami.Units.largeSpacing
        closePolicy: QQC2.Popup.CloseOnEscape | QQC2.Popup.CloseOnPressOutside
        onClosed: island.app.offered = []
        background: Rectangle {
            radius: 24
            color: Kirigami.Theme.backgroundColor
            border.color: Qt.alpha(Kirigami.Theme.textColor, 0.1)
        }
        enter: Transition {
            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 180 }
            NumberAnimation { property: "scale"; from: 0.94; to: 1; duration: 280; easing.type: Easing.OutCubic }
        }
        exit: Transition {
            NumberAnimation { property: "opacity"; to: 0; duration: 140 }
        }
        contentItem: ColumnLayout {
            spacing: 2
            Kirigami.Heading {
                level: 3
                Layout.bottomMargin: Kirigami.Units.smallSpacing
                text: !card.sending ? island.app.t("Add Device")
                    : island.offered.length ? island.app.t("Pick a device to send to") : island.app.t("Send Files")
            }
            Repeater {
                model: card.sending ? island.paired : []
                QQC2.ItemDelegate {
                    required property var modelData
                    Layout.fillWidth: true
                    icon.name: island.kindIcon(modelData.device.kind)
                    text: modelData.device.name
                    onClicked: {
                        const files = island.offered;
                        card.close();
                        if (files.length) return island.app.sendPaths(modelData.device.id, files);
                        island.sendTo = modelData.device.id;
                        picker.open();
                    }
                }
            }
            Repeater {
                model: card.sending ? [] : [["list-add", "Add Device…", "invite"], ["network-connect", "Pair by Address…", "invite"], ["drive-removable-media-usb", "Connect by USB Cable…", "usb"]]
                QQC2.ItemDelegate {
                    required property var modelData
                    Layout.fillWidth: true
                    icon.name: modelData[0]
                    text: island.app.t(modelData[1])
                    onClicked: {
                        card.close();
                        if (modelData[2] === "usb") island.app.openUsb();
                        else island.app.openInvite();
                    }
                }
            }
        }
    }

    FileDialog {
        id: picker
        title: island.app.t("Send to {name}", { name: island.app.deviceName(island.sendTo) })
        fileMode: FileDialog.OpenFiles
        onAccepted: island.app.sendFiles(island.sendTo, selectedFiles)
    }
}
