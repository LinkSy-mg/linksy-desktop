// One device: what you can do with it, what it can be for this computer, its settings,
// and everything you shared with it
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigamiaddons.components as Components

Kirigami.ScrollablePage {
    id: page
    required property var app
    readonly property var view: app.selected
    readonly property var info: view?.device ?? null
    readonly property bool phone: info?.kind === "phone"
    readonly property bool canRing: phone && app.can(info, "ring")
    readonly property string name: info?.name ?? ""

    title: name

    actions: [
        Kirigami.Action {
            icon.name: "document-send"
            text: app.t("Send Files")
            visible: page.view?.trusted ?? false
            onTriggered: filePicker.open()
        },
        Kirigami.Action {
            icon.name: "edit-clear-history"
            text: app.t("Clear")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            visible: page.view?.trusted ?? false
            onTriggered: app.request("clear_history", { with: page.info.id }, ok => { if (ok) app.history = [] })
        },
        Kirigami.Action {
            icon.name: "edit-delete"
            text: app.t("Forget Device…")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            visible: page.view?.trusted ?? false
            onTriggered: forgetDialog.open()
        }
    ]

    FileDialog {
        id: filePicker
        title: app.t("Send to {name}", { name: page.name })
        fileMode: FileDialog.OpenFiles
        onAccepted: app.sendFiles(page.info.id, selectedFiles)
    }

    Kirigami.PromptDialog {
        id: forgetDialog
        title: app.t("Forget {name}?", { name: page.name })
        subtitle: app.t("You'll need to pair again before sharing with this device.")
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: app.t("Forget")
                icon.name: "edit-delete"
                onTriggered: {
                    app.request("forget", { target: page.info.id }, () => app.refreshDevices());
                    app.selectedId = "";
                    forgetDialog.close();
                }
            },
            Kirigami.Action {
                text: app.t("Cancel")
                onTriggered: forgetDialog.close()
            }
        ]
    }

    // Files dropped anywhere on the page go to this device
    DropArea {
        anchors.fill: parent
        enabled: page.view?.trusted ?? false
        onDropped: drop => { if (drop.hasUrls) app.sendFiles(page.info.id, drop.urls) }
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing * 2
        visible: page.info !== null

        // Who it is and how it's reachable
        RowLayout {
            spacing: Kirigami.Units.largeSpacing * 2
            Layout.topMargin: Kirigami.Units.largeSpacing
            Components.Avatar {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 4
                Layout.preferredHeight: Kirigami.Units.gridUnit * 4
                name: page.name
                source: page.info ? (app.pictures[page.info.id] ?? "") : ""
            }
            ColumnLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Heading {
                    level: 1
                    text: page.name
                    textFormat: Text.PlainText
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    opacity: 0.75
                    text: {
                        if (!page.view) return "";
                        const parts = [app.kindName(page.info), app.statusLine(page.view)];
                        if (page.view.trusted) parts.push(app.t("Encrypted end to end"));
                        return parts.join(" · ");
                    }
                }
            }
        }

        // Not paired yet
        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.view !== null && !page.view.trusted
            type: Kirigami.MessageType.Information
            text: app.t("Pair once to share with this device. Both screens will show a code; check they match.")
            actions: Kirigami.Action {
                text: app.t("Pair")
                icon.name: "network-connect"
                onTriggered: app.request("pair", { target: page.info.id }, (ok, err) => { if (!ok) app.toast(err) })
            }
        }

        // The everyday actions
        Flow {
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing
            visible: page.view?.trusted ?? false

            Tile {
                iconName: "document-send"
                title: app.t("Send Files")
                subtitle: app.t("Click to choose, or drop them anywhere on this page")
                onClicked: filePicker.open()
            }
            Tile {
                iconName: "video-display"
                title: app.t("View Screen")
                subtitle: app.t("Watch or take control")
                onClicked: app.viewScreen(page.info.id)
            }
            Tile {
                iconName: "edit-paste"
                title: app.t("Clipboard")
                subtitle: app.t("Send what you copied")
                onClicked: {
                    const text = backend.clipboard();
                    if (!text) return app.toast(app.t("There's no text on the clipboard"));
                    app.request("clipboard", { to: page.info.id, text: text }, (ok, err) =>
                        app.toast(ok ? app.t("Clipboard sent to {name}", { name: page.name }) : err));
                }
            }
            Tile {
                visible: page.canRing
                iconName: "audio-volume-high"
                title: app.t("Ring")
                subtitle: app.t("Loud, even on silent")
                onClicked: app.request("ring", { to: page.info.id, on: true }, (ok, err) =>
                    app.toast(ok ? app.t("Ringing {name}…", { name: page.name }) : err))
            }
            Tile {
                visible: !page.canRing
                iconName: "preferences-desktop-notification-bell"
                title: app.t("Ping")
                subtitle: app.t("Show a notification")
                onClicked: app.request("notify", { to: page.info.id, title: "Ping", body: "" }, (ok, err) =>
                    app.toast(ok ? app.t("Pinged {name}", { name: page.name }) : err))
            }
            Tile {
                visible: page.phone && page.info && app.can(page.info, "locate")
                iconName: "find-location"
                title: app.t("Locate")
                subtitle: app.t("Asked once, never tracked")
                onClicked: {
                    app.toast(app.t("Asking {name} where it is…", { name: page.name }));
                    app.request("locate", { device: page.info.id }, (ok, where) => {
                        if (!ok) return app.toast(where);
                        Qt.openUrlExternally(`https://www.openstreetmap.org/?mlat=${where.lat.toFixed(5)}&mlon=${where.lon.toFixed(5)}#map=17/${where.lat.toFixed(5)}/${where.lon.toFixed(5)}`);
                    });
                }
            }
        }

        // What it can be for this computer
        RowLayout {
            visible: useAs.visible
            Kirigami.Heading {
                level: 3
                textFormat: Text.PlainText
                text: app.t("Use {device} as", { device: page.name })
            }
            // Faster by cable, for a phone not on one yet
            QQC2.ToolButton {
                visible: page.phone && !(page.view?.usb ?? false)
                icon.name: "drive-removable-media-usb"
                text: app.t("Connect by USB Cable")
                onClicked: app.openUsb()
            }
        }
        Flow {
            id: useAs
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing
            // Not the cards' own visible: it goes false while a layer covers the page, and stays false
            visible: (page.view?.trusted ?? false) && page.info !== null && ["display", "camera", "microphone"].some(f => app.can(page.info, f))

            UseCard {
                id: display
                visible: page.info !== null && app.can(page.info, "display")
                iconName: "video-display"
                title: app.t("Second Display")
                subtitle: app.t("Extend this computer's desktop onto it")
                button: app.t("Extend Desktop")
                kind: "display"
            }
            UseCard {
                id: webcam
                visible: page.info !== null && app.can(page.info, "camera")
                iconName: "camera-web"
                title: app.t("Webcam")
                subtitle: app.t("“{camera}” in your apps", { camera: backend.cameraName(page.name) })
                button: app.t("Start Webcam")
                kind: "camera"
            }
            UseCard {
                id: mic
                visible: page.info !== null && app.can(page.info, "microphone")
                iconName: "audio-input-microphone"
                title: app.t("Microphone")
                subtitle: app.t("“{microphone}” in calls and recorders", { microphone: `Linksy Microphone (${page.name})` })
                button: app.t("Start Microphone")
                kind: "microphone"
            }
        }

        // Its settings
        Kirigami.FormLayout {
            Layout.fillWidth: true
            visible: page.view?.trusted ?? false

            QQC2.Switch {
                Kirigami.FormData.label: app.t("Lock When Away") + ":"
                visible: page.phone && app.can(page.info, "heartbeat")
                checked: page.info !== null && app.lockPrefs.devices.includes(page.info.id)
                onToggled: {
                    const devices = app.lockPrefs.devices.filter(d => d !== page.info.id).concat(checked ? [page.info.id] : []);
                    const prefs = Object.assign({}, app.lockPrefs, { devices: devices });
                    app.request("set_lock_prefs", { prefs: prefs }, ok => { if (ok) app.lockPrefs = prefs });
                }
            }
            QQC2.Label {
                visible: page.phone && app.can(page.info, "heartbeat")
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                wrapMode: Text.Wrap
                opacity: 0.7
                font: Kirigami.Theme.smallFont
                textFormat: Text.PlainText
                text: app.t("Lock this computer when {device} leaves. It never unlocks it.", { device: page.name })
            }
            QQC2.Switch {
                Kirigami.FormData.label: app.t("Always Accept Files") + ":"
                visible: page.view?.trusted ?? false
                checked: page.view?.auto_accept ?? false
                onToggled: app.request("set_auto_accept", { device: page.info.id, on: checked }, () => app.refreshDevices())
            }
            QQC2.Switch {
                id: screenSwitch
                Kirigami.FormData.label: app.t("Always Allow Screen and Control") + ":"
                visible: page.view?.trusted ?? false
                checked: page.view?.allow_screen ?? false
                onToggled: app.request("set_allow_screen", { device: page.info.id, on: checked }, () => app.refreshDevices())
            }
            QQC2.Label {
                visible: screenSwitch.visible
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                wrapMode: Text.Wrap
                opacity: 0.7
                font: Kirigami.Theme.smallFont
                textFormat: Text.PlainText
                text: app.t("{device} can see and control this screen without asking, on the same network or by cable. Linksy still tells you when it starts.", { device: page.name })
            }
            QQC2.Switch {
                Kirigami.FormData.label: app.t("Allow Everything From Away") + ":"
                visible: page.view !== null && page.view.away_until !== undefined
                checked: page.view?.away_all ?? false
                onToggled: app.request("set_away_all", { device: page.info.id, on: checked }, () => app.refreshDevices())
            }
            QQC2.Switch {
                Kirigami.FormData.label: app.t("Terminal") + ":"
                visible: page.phone
                enabled: app.terminal.code_set
                checked: page.info !== null && app.terminal.allowed.includes(page.info.id)
                onToggled: app.request("allow_terminal", { target: page.info.id, on: checked }, () =>
                    app.request("terminal_status", {}, (ok, s) => { if (ok) app.terminal = s }))
            }
            QQC2.Label {
                visible: page.phone
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                wrapMode: Text.Wrap
                opacity: 0.7
                font: Kirigami.Theme.smallFont
                textFormat: Text.PlainText
                text: app.terminal.code_set ? app.t("Let {name} open a shell on this computer after typing the terminal code. It runs as you.", { name: page.name })
                                            : app.t("Not set: no phone can open a terminal")
            }
            QQC2.Button {
                visible: page.phone && page.info && app.can(page.info, "fs")
                Kirigami.FormData.label: app.t("Browse Files") + ":"
                icon.name: "folder-open"
                text: app.t("Browse Files")
                onClicked: app.request("browse_files", { phone: page.info.id }, (ok, where) => {
                    if (!ok) return app.toast(where);
                    Qt.openUrlExternally(where.startsWith("/") ? "file://" + where : where);
                })
            }
        }

        // Everything shared with it, newest first
        Kirigami.Heading {
            level: 3
            visible: page.view?.trusted ?? false
            text: app.t("Activity")
        }
        Kirigami.PlaceholderMessage {
            Layout.fillWidth: true
            visible: (page.view?.trusted ?? false) && app.history.length === 0
            icon.name: "view-history"
            text: app.t("Nothing shared yet")
            explanation: app.t("Files, clipboard text and pings you exchange show up here.")
        }
        Repeater {
            model: (page.view?.trusted ?? false) ? app.history : []
            delegate: ActivityRow {
                Layout.fillWidth: true
                app: page.app
            }
        }
    }

    // ---- small pieces ----

    component Tile: QQC2.AbstractButton {
        id: tile
        property string iconName
        property string title
        property string subtitle
        width: Kirigami.Units.gridUnit * 11
        height: Kirigami.Units.gridUnit * 4
        padding: Kirigami.Units.largeSpacing
        background: Rectangle {
            radius: Kirigami.Units.cornerRadius
            color: tile.down ? Qt.darker(Kirigami.Theme.alternateBackgroundColor, 1.08)
                 : tile.hovered ? Kirigami.Theme.alternateBackgroundColor
                 : Kirigami.Theme.backgroundColor
            border.color: tile.visualFocus ? Kirigami.Theme.focusColor : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.15)
            border.width: 1
        }
        contentItem: RowLayout {
            spacing: Kirigami.Units.largeSpacing
            Kirigami.Icon {
                source: tile.iconName
                Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                color: Kirigami.Theme.highlightColor
            }
            ColumnLayout {
                spacing: 0
                Layout.fillWidth: true
                QQC2.Label {
                    text: tile.title
                    font.bold: true
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                }
                QQC2.Label {
                    text: tile.subtitle
                    opacity: 0.7
                    font: Kirigami.Theme.smallFont
                    wrapMode: Text.Wrap
                    maximumLineCount: 2
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
            }
        }
    }

    // One "Use as" card: start it, or stop it while it runs
    component UseCard: Kirigami.Card {
        id: card
        property string iconName
        property string title
        property string subtitle
        property string button
        property string kind
        readonly property var running: app.sessions.find(s => s.kind === card.kind && s.peer.id === page.info?.id) ?? null
        width: Kirigami.Units.gridUnit * 14
        banner.title: card.title
        banner.titleIcon: card.iconName
        contentItem: ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            QQC2.Label {
                Layout.fillWidth: true
                text: card.running ? app.t("Live now") : card.subtitle
                wrapMode: Text.Wrap
                opacity: 0.75
            }
            // A microphone in use here: what it hears
            MicWave {
                Layout.fillWidth: true
                visible: card.running !== null && card.kind === "microphone"
            }
        }
        actions: [
            Kirigami.Action {
                // A camera in use here: see what apps get, with the phone's controls
                visible: card.running !== null && card.kind === "camera" && !card.running.sharing
                text: app.t("Preview")
                icon.name: "view-preview"
                onTriggered: app.previewCamera(card.running.session, page.name)
            },
            Kirigami.Action {
                text: card.running ? app.t("Stop") : card.button
                icon.name: card.running ? "media-playback-stop" : "media-playback-start"
                onTriggered: {
                    if (card.running) return app.request("end_session", { session: card.running.session });
                    app.request("start_session", { with: page.info.id, kind: card.kind }, (ok, err) => {
                        if (ok) return;
                        if (err.includes("can't do this yet")) app.toast(app.t({
                            display: "{name} can't be a second display yet",
                            camera: "{name} can't share its camera yet",
                            microphone: "{name} can't share its microphone yet"
                        }[card.kind], { name: page.name }));
                        else if (err.includes("declined")) app.toast(app.t("{name} didn't allow it", { name: page.name }));
                        else app.toast(err);
                    });
                }
            }
        ]
    }

    // The microphone's sound as it arrives: its latest levels as bars, newest on the right
    component MicWave: Rectangle {
        id: wave
        property var levels: []
        property bool metering: false
        readonly property int bars: Math.max(0, Math.floor((width - 2 * Kirigami.Units.largeSpacing) / 6))
        implicitHeight: Kirigami.Units.gridUnit * 2
        radius: Kirigami.Units.cornerRadius
        color: Qt.alpha(Kirigami.Theme.highlightColor, 0.12)

        function meter(on) {
            if (on === metering) return;
            metering = on;
            levels = [];
            backend.meterMicrophone(on);
        }
        onVisibleChanged: meter(visible)
        Component.onCompleted: meter(visible)
        Component.onDestruction: meter(false)

        Timer {
            interval: 60
            repeat: true
            running: wave.metering
            onTriggered: {
                const level = backend.microphoneLevel();
                if (level >= 0) wave.levels = wave.levels.concat([level]).slice(-160);
            }
        }
        Row {
            anchors.right: parent.right
            anchors.rightMargin: Kirigami.Units.largeSpacing
            anchors.verticalCenter: parent.verticalCenter
            spacing: 3
            Repeater {
                model: wave.bars
                Rectangle {
                    required property int index
                    readonly property real level: wave.levels[wave.levels.length - wave.bars + index] ?? 0
                    anchors.verticalCenter: parent.verticalCenter
                    width: 3
                    height: 3 + level * (wave.height - 15)
                    radius: 1.5
                    color: Kirigami.Theme.highlightColor
                    opacity: 0.35 + 0.65 * (index + 1) / wave.bars
                }
            }
        }
    }
}
