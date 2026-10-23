// The left column: your devices, then the ones nearby you haven't paired
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigamiaddons.components as Components

Kirigami.ScrollablePage {
    id: page
    required property var app
    title: "Linksy"

    actions: [
        Kirigami.Action {
            icon.name: "list-add"
            text: app.t("Add Device")
            onTriggered: app.openInvite()
        },
        Kirigami.Action {
            icon.name: "configure"
            text: app.t("Preferences")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            onTriggered: app.pageStack.pushDialogLayer(Qt.resolvedUrl("PrefsPage.qml"), { app: app })
        },
        Kirigami.Action {
            icon.name: "help-about"
            text: app.t("About Linksy")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            onTriggered: app.toast(app.t("This is Linksy {version}.", { version: backend.version() }))
        }
    ]


    readonly property var paired: app.devices.filter(d => d.trusted)
    readonly property var nearby: app.devices.filter(d => !d.trusted && d.nearby)

    ColumnLayout {
        spacing: 0

        Kirigami.ListSectionHeader {
            Layout.fillWidth: true
            visible: page.paired.length > 0
            text: app.t("Your Devices")
        }
        Repeater {
            model: page.paired
            delegate: deviceRow
        }
        Kirigami.ListSectionHeader {
            Layout.fillWidth: true
            visible: page.nearby.length > 0
            text: app.t("Nearby, Not Paired")
        }
        Repeater {
            model: page.nearby
            delegate: deviceRow
        }

        Kirigami.PlaceholderMessage {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.gridUnit * 2
            visible: page.paired.length + page.nearby.length === 0
            icon.name: "network-wireless"
            text: app.t("Looking for devices…")
            explanation: app.t("Open Linksy on another device on this network, or add one.")
            helpfulAction: Kirigami.Action {
                icon.name: "list-add"
                text: app.t("Add Device…")
                onTriggered: app.openInvite()
            }
        }
    }

    Component {
        id: deviceRow
        QQC2.ItemDelegate {
            id: row
            required property var modelData
            readonly property var info: modelData.device
            Layout.fillWidth: true
            highlighted: info.id === app.selectedId
            onClicked: app.select(info.id)

            contentItem: RowLayout {
                spacing: Kirigami.Units.largeSpacing
                Components.Avatar {
                    Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                    name: row.info.name
                    source: app.pictures[row.info.id] ?? ""
                    Rectangle {
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        width: Kirigami.Units.smallSpacing * 2.5
                        height: width
                        radius: width / 2
                        visible: row.modelData.connected
                        color: Kirigami.Theme.positiveTextColor
                        border.color: Kirigami.Theme.backgroundColor
                        border.width: 2
                    }
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: row.info.name
                        elide: Text.ElideRight
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: app.statusLine(row.modelData)
                        font: Kirigami.Theme.smallFont
                        opacity: 0.7
                        elide: Text.ElideRight
                    }
                }
                QQC2.Button {
                    visible: !row.modelData.trusted
                    text: app.t("Pair")
                    onClicked: app.request("pair", { target: row.info.id }, (ok, err) => { if (!ok) app.toast(err) })
                }
            }
        }
    }

    footer: QQC2.ToolBar {
        contentItem: QQC2.Label {
            text: app.status ? app.t("This device: {name}", { name: app.status.device.name }) : ""
            opacity: 0.7
            font: Kirigami.Theme.smallFont
            elide: Text.ElideRight
            leftPadding: Kirigami.Units.largeSpacing
        }
    }
}
