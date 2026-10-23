// Add a device: a QR code the phone scans, or pairing a computer by its address
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog
    required property var app
    property string link: ""
    title: app.t("Add a Device")
    padding: Kirigami.Units.largeSpacing * 2
    preferredWidth: Kirigami.Units.gridUnit * 24
    standardButtons: Kirigami.Dialog.Close

    onOpened: app.request("create_invite", {}, (ok, uri) => { dialog.link = ok ? uri : "" })
    onClosed: { app.request("cancel_invite"); link = "" }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing * 2

        Image {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: Kirigami.Units.gridUnit * 12
            Layout.preferredHeight: Kirigami.Units.gridUnit * 12
            source: dialog.link ? backend.qrCode(dialog.link) : ""
            sourceSize: Qt.size(width * 2, height * 2)
            smooth: false
            visible: dialog.link !== ""
        }
        QQC2.BusyIndicator {
            Layout.alignment: Qt.AlignHCenter
            visible: dialog.link === ""
            running: visible
        }
        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
            text: dialog.app.t("On your phone, open Linksy and scan this code. Whoever scans it can pair, so only show it to your own devices.")
        }
        QQC2.Button {
            Layout.alignment: Qt.AlignHCenter
            icon.name: "edit-copy"
            text: dialog.app.t("Copy Link")
            enabled: dialog.link !== ""
            onClicked: { backend.setClipboard(dialog.link); dialog.app.toast(dialog.app.t("Link copied")) }
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Kirigami.Heading {
            level: 4
            text: dialog.app.t("Pair by Address")
        }
        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            text: dialog.app.t("If a device doesn't show up, enter the address shown in its Linksy Preferences.")
        }
        RowLayout {
            Layout.fillWidth: true
            QQC2.TextField {
                id: address
                Layout.fillWidth: true
                placeholderText: "192.168.1.20:47200"
                onAccepted: pairButton.clicked()
            }
            QQC2.Button {
                id: pairButton
                text: dialog.app.t("Pair")
                enabled: address.text.trim() !== ""
                onClicked: dialog.app.request("pair", { target: address.text.trim() }, (ok, err) => { if (!ok) dialog.app.toast(err) })
            }
        }
    }
}
