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

    Kirigami.Theme.colorSet: Kirigami.Theme.View
    Kirigami.Theme.inherit: false
    implicitHeight: pill.implicitHeight + 8
    radius: height / 2
    color: Kirigami.Theme.backgroundColor
    border.color: Qt.alpha(Kirigami.Theme.textColor, 0.08)

    function kindIcon(kind) {
        return ({ phone: "smartphone", tablet: "tablet", laptop: "computer-laptop", desktop: "computer" })[kind] ?? "computer";
    }

    RowLayout {
        id: pill
        anchors.fill: parent
        anchors.margins: 4
        spacing: 4

        QQC2.AbstractButton {
            id: transfersPart
            implicitWidth: 44
            implicitHeight: 44
            hoverEnabled: true
            QQC2.ToolTip.text: island.app.t("Transfers")
            QQC2.ToolTip.visible: hovered
            onClicked: island.app.openTransfers()
            background: Rectangle {
                radius: width / 2
                color: transfersPart.hovered || island.app.transfersOpen ? Qt.alpha(Kirigami.Theme.highlightColor, 0.12) : "transparent"
            }
            contentItem: Item {
                Canvas {
                    id: ring
                    anchors.fill: parent
                    anchors.margins: 2
                    visible: island.files > 0
                    readonly property real value: island.fraction
                    onValueChanged: requestPaint()
                    onVisibleChanged: requestPaint()
                    onPaint: {
                        const ctx = getContext("2d");
                        ctx.reset();
                        const r = width / 2 - 2;
                        ctx.lineWidth = 3;
                        ctx.strokeStyle = Qt.alpha(Kirigami.Theme.highlightColor, 0.18);
                        ctx.beginPath();
                        ctx.arc(width / 2, height / 2, r, 0, Math.PI * 2);
                        ctx.stroke();
                        ctx.lineCap = "round";
                        ctx.strokeStyle = Kirigami.Theme.highlightColor;
                        ctx.beginPath();
                        ctx.arc(width / 2, height / 2, r, -Math.PI / 2, -Math.PI / 2 + value * Math.PI * 2);
                        ctx.stroke();
                    }
                }
                QQC2.Label {
                    anchors.centerIn: parent
                    visible: island.files > 0
                    text: island.files
                    font.bold: true
                }
                Kirigami.Icon {
                    anchors.centerIn: parent
                    visible: island.files === 0
                    width: Kirigami.Units.iconSizes.smallMedium
                    height: width
                    source: "network-transmit-receive"
                    color: transfersPart.hovered ? Kirigami.Theme.highlightColor : Kirigami.Theme.textColor
                }
            }
        }

        QQC2.AbstractButton {
            id: main
            Layout.fillWidth: true
            implicitHeight: 44
            hoverEnabled: true
            onClicked: card.show(island.none ? "add" : "send")
            background: Rectangle {
                radius: height / 2
                color: main.hovered ? Kirigami.Theme.highlightColor : Qt.alpha(Kirigami.Theme.textColor, 0.07)
                Behavior on color { ColorAnimation { duration: 150 } }
            }
            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Item { Layout.fillWidth: true }
                Kirigami.Icon {
                    implicitWidth: Kirigami.Units.iconSizes.small
                    implicitHeight: implicitWidth
                    source: island.none ? "list-add" : "document-send"
                    color: main.hovered ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                }
                QQC2.Label {
                    text: island.none ? island.app.t("Add Device") : island.app.t("Send Files")
                    font.bold: true
                    elide: Text.ElideRight
                    color: main.hovered ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                }
                Item { Layout.fillWidth: true }
            }
        }

        QQC2.AbstractButton {
            id: right
            implicitWidth: 44
            implicitHeight: 44
            hoverEnabled: true
            QQC2.ToolTip.text: island.none ? island.app.t("Send Files") : island.app.t("Add Device")
            QQC2.ToolTip.visible: hovered
            onClicked: card.show(island.none ? "send" : "add")
            background: Rectangle {
                radius: width / 2
                color: right.hovered ? Qt.alpha(Kirigami.Theme.highlightColor, 0.12) : "transparent"
            }
            contentItem: Item {
                Kirigami.Icon {
                    anchors.centerIn: parent
                    width: Kirigami.Units.iconSizes.smallMedium
                    height: width
                    source: island.none ? "document-send" : "list-add"
                    color: right.hovered ? Kirigami.Theme.highlightColor : Kirigami.Theme.textColor
                }
            }
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
                text: card.sending ? island.app.t("Send Files") : island.app.t("Add Device")
            }
            Repeater {
                model: card.sending ? island.paired : []
                QQC2.ItemDelegate {
                    required property var modelData
                    Layout.fillWidth: true
                    icon.name: island.kindIcon(modelData.device.kind)
                    text: modelData.device.name
                    onClicked: {
                        card.close();
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
