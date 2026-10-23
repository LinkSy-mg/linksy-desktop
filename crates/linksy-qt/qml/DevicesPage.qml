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
            icon.name: "drive-removable-media-usb"
            text: app.t("Connect by USB Cable…")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            onTriggered: app.openUsb()
        },
        Kirigami.Action {
            icon.name: "folder-download"
            text: app.t("Open Received Files")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            onTriggered: if (app.settings) Qt.openUrlExternally("file://" + app.settings.download_dir)
        },
        Kirigami.Action {
            icon.name: "configure"
            text: app.t("Preferences")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            onTriggered: app.pageStack.pushDialogLayer(Qt.resolvedUrl("PrefsPage.qml"), { app: app })
        },
        Kirigami.Action {
            icon.name: "run-build-install"
            text: app.t("Finish Setup…")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            onTriggered: app.openSetup()
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
        contentItem: ColumnLayout {
            QQC2.Button {
                Layout.fillWidth: true
                // Screenshots (LINKSY_SCREENSHOT) show the app as on a finished setup
                readonly property int missing: backend.env("LINKSY_SCREENSHOT") ? 0 : (app.setup?.missing ?? 0)
                visible: missing > 0
                icon.name: "emblem-important"
                text: missing === 1 ? app.t("Finish Setup · 1 thing") : app.t("Finish Setup · {n} things", { n: missing })
                onClicked: app.openSetup()
            }
            QQC2.Button {
                Layout.fillWidth: true
                visible: app.update !== null
                flat: true
                icon.name: "update-none"
                text: app.update ? app.t("Linksy {version} Is Available", { version: app.update.version }) : ""
                QQC2.ToolTip.text: app.t("See what's new")
                QQC2.ToolTip.visible: hovered
                onClicked: if (app.update.notes) Qt.openUrlExternally(app.update.notes)
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: app.status ? app.t("This device: {name}", { name: app.status.device.name }) : ""
                opacity: 0.7
                font: Kirigami.Theme.smallFont
                elide: Text.ElideRight
                leftPadding: Kirigami.Units.largeSpacing
            }
        }
    }
}
