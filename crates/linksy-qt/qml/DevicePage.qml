// One device: who it is and how it's connected, the everyday actions, then tabs: what it can be
// for this computer, its inbox, its files, everything shared with it, and its settings. A tab's
// content is built when it shows.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import QtQuick.Shapes
import org.kde.kirigami as Kirigami
import org.kde.kirigamiaddons.components as Components

Kirigami.Page {
    id: page
    required property var app
    readonly property var view: app.selected
    readonly property var info: view?.device ?? null
    readonly property string deviceId: info?.id ?? ""
    readonly property bool trusted: view?.trusted ?? false
    readonly property bool phone: info?.kind === "phone"
    readonly property bool handheld: phone || info?.kind === "tablet"
    readonly property bool canRing: phone && app.can(info, "ring")
    readonly property string name: info?.name ?? ""
    // The device asked again every few seconds while its page shows: how it's connected, how fast
    property var fresh: null
    readonly property var link: fresh ?? view
    readonly property bool onScreen: visible && Window.visibility !== Window.Minimized && Window.visibility !== Window.Hidden
    // Narrower, the inbox shows one side at a time
    readonly property bool narrow: width < Kirigami.Units.gridUnit * 38

    // The tabs this device has, and the one showing
    property string tab: "use"
    // What the History tab shows: all, files, clipboard or links
    property string historyFilter: "all"
    // Development aid (screenshots): LINKSY_SCREENSHOT_TAB=inbox opens that tab once it shows
    property string wanted: backend.env("LINKSY_SCREENSHOT_TAB")
    // A string first, so the list changes only when the tabs do, not with every device update
    readonly property string tabsKey: {
        if (!trusted || !info) return "";
        const shown = [];
        if (handheld || ["display", "camera", "microphone"].some(f => app.can(info, f))) shown.push("use");
        if (phone && (app.can(info, "notifications") || app.can(info, "sms"))) shown.push("inbox");
        if (handheld && app.can(info, "gallery")) shown.push("gallery");
        if (handheld && app.can(info, "fs")) shown.push("files");
        return shown.concat(["transfers", "history", "settings"]).join(" ");
    }
    readonly property var tabsShown: tabsKey ? tabsKey.split(" ") : []
    readonly property string inboxPhone: tabsShown.includes("inbox") ? deviceId : ""
    readonly property string filesPhone: tabsShown.includes("files") ? deviceId : ""
    readonly property string galleryPhone: tabsShown.includes("gallery") ? deviceId : ""
    // Where the tabs are in the page, to pin them at its top once it scrolls past them
    readonly property real tabsTop: column.y + tabBar.y

    // Its deck shows in place of the page, with Back
    property bool deckOpen: false

    title: deckOpen ? app.t("Deck") : name
    padding: 0
    flickable: deckOpen ? deckFlick : flick
    titleDelegate: RowLayout {
        Layout.fillWidth: true
        Layout.minimumWidth: 0
        spacing: Kirigami.Units.smallSpacing
        QQC2.ToolButton {
            visible: page.deckOpen
            icon.name: "go-previous"
            text: page.app.t("Back")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.text: text
            QQC2.ToolTip.visible: hovered
            onClicked: page.deckOpen = false
        }
        Kirigami.Heading {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: page.title
            maximumLineCount: 1
            elide: Text.ElideRight
            textFormat: Text.PlainText
        }
    }

    // Send Files is the header's own button, beside the device's name
    actions: [
        Kirigami.Action {
            icon.name: "edit-delete"
            text: app.t("Forget Device…")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            visible: page.trusted && !page.deckOpen
            onTriggered: forgetDialog.open()
        }
    ]

    onViewChanged: fresh = null
    onDeviceIdChanged: {
        wanted = "";
        deckOpen = false;
        Qt.callLater(page.rebuild);
    }
    onInboxPhoneChanged: inbox.reset(inboxPhone)
    onFilesPhoneChanged: if (tab === "files") Qt.callLater(page.showFiles)
    onGalleryPhoneChanged: if (tab === "gallery") Qt.callLater(page.showGallery)
    onTabsShownChanged: fixTab()
    onTabChanged: tabShown()
    Component.onCompleted: {
        inbox.reset(inboxPhone);
        fixTab();
        tabShown();
    }

    // Open a tab, now or once this device has it (openMessages)
    function showTab(which) {
        if (tabsShown.includes(which)) tab = which;
        else wanted = which;
    }

    // Another device: the tab built again, from its own state
    function rebuild() {
        content.active = false;
        content.active = true;
    }

    // The tab showing went away: the first one there is
    function fixTab() {
        if (wanted && tabsShown.includes(wanted)) {
            tab = wanted;
            wanted = "";
        } else if (tabsShown.length && !tabsShown.includes(tab)) {
            tab = tabsShown[0];
        }
    }

    // What a tab shows, fetched as it shows
    function tabShown() {
        if (tab === "inbox") inbox.load();
        else if (tab === "gallery") Qt.callLater(page.showGallery);
        else if (tab === "files") Qt.callLater(page.showFiles);
    }

    // Its albums and newest photos the first time
    function showGallery() {
        if (galleryPhone) gallery.show(galleryPhone);
    }

    // The phone's top folder the first time, else where it was
    function showFiles() {
        if (!filesPhone) return;
        if (files.phone !== filesPhone) files.reset(filesPhone);
        files.list();
    }

    // A tab chosen from the pinned ones: its content starts just under them
    function pick(which) {
        tab = which;
        pinning.restart();
        toTabs();
    }

    function toTabs() {
        flick.contentY = Math.max(0, Math.min(tabsTop, flick.contentHeight - flick.height));
    }

    function tabTitle(which) {
        return ({ use: app.t("Use As"), inbox: app.t("Inbox"), gallery: app.t("Gallery"), files: app.t("Files"), transfers: app.t("Transfers"), history: app.t("History"), settings: app.t("Settings") })[which] ?? "";
    }

    function tabIcon(which) {
        return ({ use: "video-display", inbox: "mail-unread", gallery: "image-x-generic", files: "folder", transfers: "exchange-positions", history: "document-open-recent", settings: "configure" })[which] ?? "";
    }

    function kindIcon(kind) {
        return ({ phone: "smartphone", tablet: "tablet", laptop: "computer-laptop" })[kind] ?? "computer";
    }

    // "Wi-Fi · 4 ms", or how it's reachable when it isn't connected
    function linkText(d) {
        if (!d) return "";
        if (!d.connected) return !d.trusted ? app.t("Nearby · Not paired") : d.nearby ? app.t("Nearby") : app.t("Not nearby");
        const what = d.away ? app.t("Away from home") : d.usb ? app.t("USB") : handheld ? app.t("Wi-Fi") : app.t("Local network");
        return typeof d.latency_ms === "number" ? app.t("{link} · {ms} ms", { link: what, ms: Math.max(1, d.latency_ms) }) : what;
    }

    Inbox {
        id: inbox
        app: page.app
        shown: page.tab === "inbox" && page.trusted
    }

    PhoneFiles {
        id: files
        app: page.app
    }

    PhoneGallery {
        id: gallery
        app: page.app
    }

    // The corner panel steps aside while a Transfers tab shows the same
    Binding {
        target: page.app
        property: "transfersOpen"
        value: page.visible && page.trusted && page.tab === "transfers"
    }

    Connections {
        target: page.app
        function onConversationChanged(c) { inbox.changed(c) }
        function onAlertsChanged(phone) { if (phone === inbox.phone) inbox.load() }
        function onFsProgress(progress) { files.progress(progress) }
        function onGalleryChanged(phone) {
            if (phone !== gallery.phone || page.tab !== "gallery") return;
            gallery.loadAlbums();
            gallery.newer();
        }
    }

    // Its link and round trip, while the page is on screen
    Timer {
        interval: 5000
        repeat: true
        running: page.onScreen && page.trusted && (page.view?.connected ?? false)
        onTriggered: {
            const id = page.deviceId;
            page.app.request("devices", {}, (ok, list) => {
                if (ok && id === page.deviceId) page.fresh = list.find(d => d.device.id === id) ?? null;
            });
        }
    }

    // While a tab chosen from the pinned ones fills in, it stays just under them
    Timer {
        id: pinning
        interval: 300
    }

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
        enabled: page.trusted
        onDropped: drop => { if (drop.hasUrls) app.sendFiles(page.info.id, drop.urls) }
    }

    // The deck in place of the page; the page stays where it was underneath
    QQC2.ScrollView {
        anchors.fill: parent
        visible: page.deckOpen
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        Flickable {
            id: deckFlick
            contentWidth: width
            contentHeight: deck.height + Kirigami.Units.gridUnit * 3
            clip: true
            flickDeceleration: 1500
            maximumFlickVelocity: 5000
            Loader {
                id: deck
                x: column.x
                y: Kirigami.Units.largeSpacing * 2
                width: column.width
                active: page.deckOpen
                sourceComponent: DeckView {
                    app: page.app
                    phone: page.deviceId
                    name: page.name
                }
            }
        }
    }

    QQC2.ScrollView {
        anchors.fill: parent
        visible: !page.deckOpen
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        Flickable {
            id: flick
            contentWidth: width
            contentHeight: column.y + column.implicitHeight + Kirigami.Units.gridUnit * 2
            clip: true
            // As Kirigami's own pages flick
            flickDeceleration: 1500
            maximumFlickVelocity: 5000
            onContentHeightChanged: if (pinning.running) page.toTabs()

            ColumnLayout {
                id: column
                x: (flick.width - width) / 2
                y: Kirigami.Units.largeSpacing * 2
                width: Math.min(flick.width - Kirigami.Units.gridUnit * 2, Kirigami.Units.gridUnit * 48)
                spacing: Kirigami.Units.largeSpacing * 2
                visible: page.info !== null

                // Who it is, how it's connected and how fast, its battery
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.largeSpacing * 2

                    Item {
                        Layout.alignment: Qt.AlignTop
                        implicitWidth: Kirigami.Units.gridUnit * 4.5
                        implicitHeight: implicitWidth
                        Components.Avatar {
                            anchors.fill: parent
                            name: page.name
                            source: page.info ? (app.pictures[page.info.id] ?? "") : ""
                        }
                        Rectangle {
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            width: Kirigami.Units.iconSizes.small + Kirigami.Units.smallSpacing * 2.5
                            height: width
                            radius: width / 2
                            color: Kirigami.Theme.backgroundColor
                            border.width: 1
                            border.color: Qt.alpha(Kirigami.Theme.textColor, 0.15)
                            Kirigami.Icon {
                                anchors.centerIn: parent
                                width: Kirigami.Units.iconSizes.small * 0.875
                                height: width
                                source: app.sym(page.kindIcon(page.info?.kind ?? ""))
                                fallback: page.kindIcon(page.info?.kind ?? "")
                            }
                            HoverHandler { id: kindHover }
                            QQC2.ToolTip.visible: kindHover.hovered
                            QQC2.ToolTip.text: page.info ? app.kindName(page.info) : ""
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                        spacing: Kirigami.Units.smallSpacing * 2
                        Kirigami.Heading {
                            Layout.fillWidth: true
                            level: 1
                            font.weight: Font.DemiBold
                            text: page.name
                            textFormat: Text.PlainText
                            wrapMode: Text.Wrap
                        }
                        // How it's connected and how fast, its battery, that it's encrypted: plain words
                        Flow {
                            Layout.fillWidth: true
                            spacing: Kirigami.Units.largeSpacing * 1.5

                            RowLayout {
                                id: linkInfo
                                readonly property var d: page.link
                                readonly property string look: !d || !d.connected ? "offline" : d.away ? "away" : d.usb ? "usb" : "on"
                                readonly property string iconName: ({ on: page.handheld ? "network-wireless" : "network-wired", away: "network-wireless-hotspot",
                                                                      usb: "drive-removable-media-usb", offline: "network-disconnect" })[look]
                                readonly property color ink: look === "on" ? Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.positiveTextColor, Kirigami.Theme.textColor, 0.3)
                                                           : look === "offline" ? Qt.alpha(Kirigami.Theme.textColor, 0.7) : Kirigami.Theme.linkColor
                                visible: d !== null
                                spacing: Kirigami.Units.smallSpacing
                                Kirigami.Icon {
                                    implicitWidth: Kirigami.Units.iconSizes.small
                                    implicitHeight: implicitWidth
                                    source: app.sym(linkInfo.iconName)
                                    fallback: linkInfo.iconName
                                    color: linkInfo.ink
                                }
                                QQC2.Label {
                                    text: page.linkText(linkInfo.d)
                                    color: linkInfo.ink
                                }
                            }

                            Item {
                                id: batteryChip
                                readonly property var battery: page.link && (page.link.connected || page.link.nearby) ? (page.link.battery ?? null) : null
                                readonly property real level: Math.max(0, Math.min(1, (battery?.level ?? 0) / 100))
                                readonly property bool charging: battery?.charging ?? false
                                visible: battery !== null
                                implicitWidth: batteryRow.implicitWidth
                                implicitHeight: batteryRow.implicitHeight
                                opacity: 0.75
                                HoverHandler { id: batteryHover }
                                QQC2.ToolTip.visible: batteryHover.hovered
                                QQC2.ToolTip.text: batteryChip.charging ? app.t("Battery {level}%, charging", { level: batteryChip.battery?.level ?? 0 })
                                                                        : app.t("Battery {level}%", { level: batteryChip.battery?.level ?? 0 })
                                RowLayout {
                                    id: batteryRow
                                    anchors.centerIn: parent
                                    spacing: Kirigami.Units.smallSpacing
                                    // The battery, filled to its level; red when low and not charging
                                    Item {
                                        implicitWidth: 22
                                        implicitHeight: 12
                                        Rectangle {
                                            width: 19
                                            height: parent.height
                                            radius: 3
                                            color: "transparent"
                                            border.width: 1.4
                                            border.color: Kirigami.Theme.textColor
                                            Rectangle {
                                                x: 2.3
                                                y: 2.3
                                                width: (parent.width - 4.6) * batteryChip.level
                                                height: parent.height - 4.6
                                                radius: 1.4
                                                color: batteryChip.level <= 0.15 && !batteryChip.charging ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
                                            }
                                        }
                                        Rectangle {
                                            x: 19.6
                                            y: 3.8
                                            width: 1.8
                                            height: 4.4
                                            radius: 0.9
                                            color: Kirigami.Theme.textColor
                                        }
                                    }
                                    QQC2.Label {
                                        text: `${batteryChip.battery?.level ?? 0}%`
                                    }
                                    // Charging: a bolt
                                    Shape {
                                        visible: batteryChip.charging
                                        implicitWidth: 8
                                        implicitHeight: 12
                                        preferredRendererType: Shape.CurveRenderer
                                        ShapePath {
                                            strokeWidth: -1
                                            fillColor: Kirigami.Theme.textColor
                                            startX: 5
                                            startY: 0
                                            PathLine { x: 0; y: 7 }
                                            PathLine { x: 3.5; y: 7 }
                                            PathLine { x: 2.5; y: 12 }
                                            PathLine { x: 8; y: 4.5 }
                                            PathLine { x: 4.5; y: 4.5 }
                                            PathLine { x: 5; y: 0 }
                                        }
                                    }
                                }
                            }

                            RowLayout {
                                visible: page.trusted
                                spacing: Kirigami.Units.smallSpacing
                                opacity: 0.75
                                Kirigami.Icon {
                                    implicitWidth: Kirigami.Units.iconSizes.small * 0.875
                                    implicitHeight: implicitWidth
                                    source: app.sym("object-locked")
                                    fallback: "object-locked"
                                }
                                QQC2.Label {
                                    text: app.t("Encrypted end to end")
                                }
                            }
                        }
                    }

                    QQC2.Button {
                        Layout.alignment: Qt.AlignVCenter
                        visible: page.trusted
                        highlighted: true
                        icon.name: app.sym("document-send")
                        text: app.t("Send Files")
                        onClicked: filePicker.open()
                        QQC2.ToolTip.visible: hovered
                        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                        QQC2.ToolTip.text: app.t("Click to choose, or drop them anywhere on this page")
                    }

                    // How long it can still be reached away from home
                    ColumnLayout {
                        Layout.alignment: Qt.AlignVCenter
                        visible: page.trusted && typeof page.view?.away_until === "number"
                        spacing: 2
                        QQC2.Label {
                            Layout.alignment: Qt.AlignRight
                            text: app.t("Reachable from away until")
                            font: Kirigami.Theme.smallFont
                            opacity: 0.7
                        }
                        QQC2.Label {
                            Layout.alignment: Qt.AlignRight
                            text: typeof page.view?.away_until === "number" ? new Date(page.view.away_until * 1000).toLocaleDateString(Qt.locale(), "ddd d MMM") : ""
                            font.weight: Font.DemiBold
                        }
                    }
                }

                // Not paired yet
                Kirigami.InlineMessage {
                    Layout.fillWidth: true
                    visible: page.view !== null && !page.trusted
                    type: Kirigami.MessageType.Information
                    text: app.t("Pair once to share with this device. Both screens will show a code; check they match.")
                    actions: Kirigami.Action {
                        text: app.t("Pair")
                        icon.name: "network-connect"
                        onTriggered: app.request("pair", { target: page.info.id }, (ok, err) => { if (!ok) app.toast(err) })
                    }
                }

                // The everyday actions, one bar of tool buttons
                Rectangle {
                    Layout.fillWidth: true
                    visible: page.trusted
                    implicitHeight: actionBar.implicitHeight + Kirigami.Units.smallSpacing * 2
                    radius: Kirigami.Units.cornerRadius * 2
                    color: Qt.alpha(Kirigami.Theme.textColor, 0.04)

                    RowLayout {
                        id: actionBar
                        anchors.fill: parent
                        anchors.margins: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.smallSpacing

                        BarButton {
                            iconName: "edit-paste"
                            text: app.t("Clipboard")
                            hint: app.t("Send what you copied")
                            onClicked: {
                                const text = backend.clipboard();
                                if (!text) return app.toast(app.t("There's no text on the clipboard"));
                                app.request("clipboard", { to: page.info.id, text: text }, (ok, err) =>
                                    app.toast(ok ? app.t("Clipboard sent to {name}", { name: page.name }) : err));
                            }
                        }
                        BarButton {
                            iconName: "video-display"
                            text: app.t("View Screen")
                            hint: app.t("Watch or take control")
                            onClicked: app.viewScreen(page.info.id)
                        }
                        BarButton {
                            visible: page.canRing
                            iconName: "audio-volume-high"
                            tint: Kirigami.Theme.neutralTextColor
                            text: app.t("Ring")
                            hint: app.t("Loud, even on silent")
                            onClicked: app.request("ring", { to: page.info.id, on: true }, (ok, err) =>
                                app.toast(ok ? app.t("Ringing {name}…", { name: page.name }) : err))
                        }
                        BarButton {
                            visible: !page.canRing
                            iconName: "preferences-desktop-notification-bell"
                            tint: Kirigami.Theme.neutralTextColor
                            text: app.t("Ping")
                            hint: app.t("Show a notification")
                            onClicked: app.request("notify", { to: page.info.id, title: "Ping", body: "" }, (ok, err) =>
                                app.toast(ok ? app.t("Pinged {name}", { name: page.name }) : err))
                        }
                        BarButton {
                            visible: page.phone && page.info && app.can(page.info, "locate")
                            iconName: "find-location"
                            tint: Kirigami.Theme.neutralTextColor
                            text: app.t("Locate")
                            hint: app.t("Asked once, never tracked")
                            onClicked: {
                                app.toast(app.t("Asking {name} where it is…", { name: page.name }));
                                app.request("locate", { device: page.info.id }, (ok, where) => {
                                    if (!ok) return app.toast(where);
                                    Qt.openUrlExternally(`https://www.openstreetmap.org/?mlat=${where.lat.toFixed(5)}&mlon=${where.lon.toFixed(5)}#map=17/${where.lat.toFixed(5)}/${where.lon.toFixed(5)}`);
                                });
                            }
                        }
                    }
                }

                TabRow {
                    id: tabBar
                    Layout.fillWidth: true
                    Layout.topMargin: Kirigami.Units.largeSpacing
                    visible: page.trusted
                    onChosen: which => page.tab = which
                    onScrollChanged: pinned.scroll = scroll
                }

                Loader {
                    id: content
                    Layout.fillWidth: true
                    visible: page.trusted
                    sourceComponent: !page.trusted ? null
                        : ({ use: useTab, inbox: inboxTab, gallery: galleryTab, files: filesTab, transfers: transfersTab, history: historyTab, settings: settingsTab })[page.tab] ?? null
                }
            }
        }
    }

    // The tabs again, pinned at the top of the page once it scrolls past them
    Rectangle {
        visible: !page.deckOpen && tabBar.visible && flick.contentY > page.tabsTop
        width: flick.width
        height: pinned.implicitHeight
        color: Kirigami.Theme.backgroundColor
        TabRow {
            id: pinned
            x: column.x
            width: column.width
            onChosen: which => page.pick(which)
            onScrollChanged: tabBar.scroll = scroll
        }
    }

    // ---- the tabs ----

    // What it can be for this computer
    Component {
        id: useTab
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing * 2

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
            // Four a line on a wide page (760sp), two on a medium one (540sp), else one; a hidden
            // card leaves no gap
            GridLayout {
                id: useAs
                Layout.fillWidth: true
                columns: page.width > Kirigami.Units.gridUnit * 42 ? 4 : page.width >= Kirigami.Units.gridUnit * 30 ? 2 : 1
                columnSpacing: Kirigami.Units.largeSpacing
                rowSpacing: Kirigami.Units.largeSpacing
                // Not the cards' own visible: it goes false while a layer covers the page, and stays false
                visible: page.info !== null && (page.handheld || ["display", "camera", "microphone"].some(f => app.can(page.info, f)))

                UseCard {
                    visible: page.info !== null && app.can(page.info, "display")
                    iconName: "video-display"
                    title: app.t("Second Display")
                    subtitle: app.t("Extend this computer's desktop onto it")
                    button: app.t("Extend Desktop")
                    kind: "display"
                }
                UseCard {
                    visible: page.info !== null && app.can(page.info, "camera")
                    iconName: "camera-web"
                    title: app.t("Webcam")
                    subtitle: app.t("“{camera}” in your apps", { camera: backend.cameraName(page.name) })
                    button: app.t("Start Webcam")
                    kind: "camera"
                }
                UseCard {
                    visible: page.info !== null && app.can(page.info, "microphone")
                    iconName: "audio-input-microphone"
                    title: app.t("Microphone")
                    subtitle: app.t("“{microphone}” in calls and recorders", { microphone: `Linksy Microphone (${page.name})` })
                    button: app.t("Start Microphone")
                    kind: "microphone"
                }
                // Whatever the phone says it can do: the deck is this computer's side
                Kirigami.Card {
                    id: deckCard
                    padding: Kirigami.Units.largeSpacing * 2
                    visible: page.handheld
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                    footer: CardActions { owner: deckCard }
                    contentItem: Item {
                        implicitWidth: deckHead.implicitWidth
                        implicitHeight: deckHead.implicitHeight
                        CardHead {
                            id: deckHead
                            width: parent.width
                            title: app.t("Deck")
                            iconName: "view-grid"
                            description: app.t("Buttons for this computer on {device}", { device: page.name })
                        }
                    }
                    actions: Kirigami.Action {
                        text: app.t("Choose Actions…")
                        icon.name: app.sym("configure")
                        icon.color: Kirigami.Theme.linkColor
                        onTriggered: page.deckOpen = true
                    }
                }
            }

            // Its terminal here: allowed with the code, or locked out after wrong ones
            Kirigami.Card {
                id: terminalCard
                readonly property bool locked: page.info !== null && app.terminal.locked.includes(page.info.id)
                readonly property bool allowed: page.info !== null && app.terminal.allowed.includes(page.info.id)
                readonly property var running: app.terminal.running.filter(t => t.device === page.info?.id)
                Layout.fillWidth: true
                padding: Kirigami.Units.largeSpacing * 2
                visible: page.phone
                footer: CardActions { owner: terminalCard }
                contentItem: Item {
                    implicitWidth: terminalColumn.implicitWidth
                    implicitHeight: terminalColumn.implicitHeight
                    ColumnLayout {
                        id: terminalColumn
                        width: parent.width
                        spacing: Kirigami.Units.largeSpacing
                        CardHead {
                            Layout.fillWidth: true
                            iconName: "utilities-terminal"
                            title: app.t("Your computer's terminal on {name}", { name: page.name })
                            description: terminalCard.locked ? app.t("Locked: 5 wrong codes in a row were typed on {name}. Unlock it if that was you.", { name: page.name })
                                : app.terminal.code_set ? app.t("Let {name} open a shell on this computer after typing the terminal code. It runs as you.", { name: page.name })
                                : app.t("Not set: no phone can open a terminal")
                        }
                        // The ones running now, each with End
                        Repeater {
                            model: terminalCard.running
                            delegate: RowLayout {
                                id: shell
                                required property var modelData
                                Layout.fillWidth: true
                                spacing: Kirigami.Units.largeSpacing
                                Kirigami.Icon {
                                    implicitWidth: Kirigami.Units.iconSizes.smallMedium
                                    implicitHeight: implicitWidth
                                    source: app.sym("utilities-terminal")
                                    fallback: "utilities-terminal"
                                    color: shell.modelData.attached ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.disabledTextColor
                                }
                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 0
                                    QQC2.Label {
                                        Layout.fillWidth: true
                                        text: shell.modelData.title
                                        textFormat: Text.PlainText
                                        elide: Text.ElideRight
                                    }
                                    QQC2.Label {
                                        Layout.fillWidth: true
                                        text: shell.modelData.attached ? app.t("Terminal open on the phone now") : app.t("Terminal still running · not open on the phone")
                                        wrapMode: Text.Wrap
                                        font: Kirigami.Theme.smallFont
                                        opacity: 0.7
                                    }
                                }
                                QQC2.Button {
                                    icon.name: "media-playback-stop"
                                    text: app.t("End")
                                    onClicked: app.request("end_terminal", { terminal: shell.modelData.id }, (ok, err) => {
                                        if (!ok) app.toast(err);
                                        app.request("terminal_status", {}, (ok2, s) => { if (ok2) app.terminal = s });
                                    })
                                }
                            }
                        }
                    }
                }
                actions: [
                    Kirigami.Action {
                        visible: terminalCard.locked
                        text: app.t("Unlock")
                        icon.name: app.sym("object-unlocked")
                        onTriggered: app.request("unlock_terminal", { target: page.info.id }, (ok, err) => {
                            if (!ok) app.toast(err);
                            app.request("terminal_status", {}, (ok2, s) => { if (ok2) app.terminal = s });
                        })
                    },
                    Kirigami.Action {
                        visible: !terminalCard.locked
                        enabled: app.terminal.code_set
                        text: terminalCard.allowed ? app.t("Turn Off") : app.t("Allow on {name}", { name: page.name })
                        icon.name: app.sym(terminalCard.allowed ? "media-playback-stop" : "dialog-ok")
                        icon.color: terminalCard.allowed ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.linkColor
                        onTriggered: app.request("allow_terminal", { target: page.info.id, on: !terminalCard.allowed }, () =>
                            app.request("terminal_status", {}, (ok, s) => { if (ok) app.terminal = s }))
                    }
                ]
            }
        }
    }

    Component {
        id: inboxTab
        InboxTab {
            app: page.app
            store: inbox
            narrow: page.narrow
        }
    }

    Component {
        id: transfersTab
        TransfersTab {
            app: page.app
            peer: page.deviceId
        }
    }

    Component {
        id: galleryTab
        GalleryTab {
            app: page.app
            store: gallery
        }
    }

    Component {
        id: filesTab
        FilesTab {
            app: page.app
            store: files
            deviceName: page.name
            narrow: page.narrow
            view: flick
            covered: tabBar.height
        }
    }

    // Everything shared with it, newest first; only the rows in view are built
    Component {
        id: historyTab
        ColumnLayout {
            id: historyView
            spacing: 0

            // The newest 150, of the kind chosen; older ones are still in `linksy history`
            function show() {
                const kind = ({ files: "files", clipboard: "clipboard", links: "link" })[page.historyFilter];
                const shown = app.history.slice(0, 150).filter(e => !kind || e.what.kind === kind);
                entries.sync(shown, e => e.id, e => ({ day: app.dayLabel(e.at) }));
            }

            Component.onCompleted: show()

            KeyedList { id: entries }

            Connections {
                target: page.app
                function onHistoryChanged() { historyView.show() }
            }
            Connections {
                target: page
                function onHistoryFilterChanged() { historyView.show() }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.bottomMargin: Kirigami.Units.largeSpacing
                Flow {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing
                    Repeater {
                        model: [["all", app.t("All")], ["files", app.t("Files")], ["clipboard", app.t("Clipboard")], ["links", app.t("Links")]]
                        delegate: FilterPill {
                            required property var modelData
                            text: modelData[1]
                            chosen: page.historyFilter === modelData[0]
                            onClicked: page.historyFilter = modelData[0]
                        }
                    }
                }
                QQC2.Button {
                    Layout.alignment: Qt.AlignTop
                    flat: true
                    text: app.t("Clear")
                    onClicked: app.request("clear_history", { with: page.info.id }, ok => { if (ok) app.history = [] })
                }
            }

            Kirigami.PlaceholderMessage {
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.gridUnit
                visible: entries.model.count === 0
                icon.name: "view-history"
                text: app.t("Nothing shared yet")
                explanation: app.t("Files, clipboard text and pings you exchange show up here.")
            }
            QQC2.ScrollView {
                Layout.fillWidth: true
                // As tall as the page under the pinned tabs, then it scrolls by itself. (A view
                // with no height builds no rows to measure: the most at first.)
                readonly property real most: Math.max(Kirigami.Units.gridUnit * 8, flick.height - tabBar.height)
                Layout.preferredHeight: historyList.contentHeight > 0 ? Math.min(historyList.contentHeight, most) : most
                visible: entries.model.count > 0
                ListView {
                    id: historyList
                    clip: true
                    model: entries.model
                    section.property: "day"
                    section.delegate: QQC2.Label {
                        required property string section
                        width: historyList.width
                        topPadding: Kirigami.Units.largeSpacing
                        bottomPadding: Kirigami.Units.smallSpacing
                        text: section
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        font.weight: Font.DemiBold
                        opacity: 0.7
                    }
                    delegate: Item {
                        id: entry
                        required property string key
                        width: historyList.width
                        implicitHeight: activity.implicitHeight
                        ActivityRow {
                            id: activity
                            width: parent.width
                            app: page.app
                            modelData: entries.rows[entry.key]
                        }
                    }
                }
            }
        }
    }

    // Its settings, as plain rows
    Component {
        id: settingsTab
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing * 2

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                SwitchRow {
                    visible: page.phone && app.can(page.info, "heartbeat")
                    iconName: "system-lock-screen"
                    title: app.t("Lock When Away")
                    subtitle: app.t("Lock this computer when {device} leaves. It never unlocks it.", { device: page.name })
                    checked: page.info !== null && app.lockPrefs.devices.includes(page.info.id)
                    onSwitched: now => {
                        const devices = app.lockPrefs.devices.filter(d => d !== page.info.id).concat(now ? [page.info.id] : []);
                        const prefs = Object.assign({}, app.lockPrefs, { devices: devices });
                        app.request("set_lock_prefs", { prefs: prefs }, ok => { if (ok) app.lockPrefs = prefs });
                    }
                }
                SwitchRow {
                    iconName: "folder-download"
                    title: app.t("Always Accept Files")
                    subtitle: app.t("Files from {device} are saved without asking.", { device: page.name })
                    checked: page.view?.auto_accept ?? false
                    onSwitched: now => app.request("set_auto_accept", { device: page.info.id, on: now }, () => app.refreshDevices())
                }
                SwitchRow {
                    iconName: "video-display"
                    title: app.t("Always Allow Screen and Control")
                    subtitle: app.t("{device} can see and control this screen without asking, on the same network or by cable. Linksy still tells you when it starts.", { device: page.name })
                    checked: page.view?.allow_screen ?? false
                    onSwitched: now => app.request("set_allow_screen", { device: page.info.id, on: now }, () => app.refreshDevices())
                }
                SwitchRow {
                    visible: page.view !== null && page.view.away_until !== undefined
                    iconName: "network-wireless-hotspot"
                    title: app.t("Allow Everything From Away")
                    subtitle: app.t("Away from home, {device} can always send the clipboard, links and files and ring. On: screens, control, its files, camera and microphone too.", { device: page.name })
                    checked: page.view?.away_all ?? false
                    onSwitched: now => app.request("set_away_all", { device: page.info.id, on: now }, () => app.refreshDevices())
                }
            }

            QQC2.Button {
                icon.name: "edit-delete"
                text: app.t("Forget Device…")
                onClicked: forgetDialog.open()
            }
        }
    }

    // ---- small pieces ----

    // Use As, Inbox, Files, History, Settings: a line under them all, the one showing
    // underlined in the accent
    component TabRow: Item {
        id: bar
        property alias scroll: tabsFlick.contentX
        signal chosen(string which)
        implicitHeight: tabsRow.implicitHeight

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
        }
        // Too narrow for them all, they scroll sideways
        Flickable {
            id: tabsFlick
            anchors.fill: parent
            contentWidth: tabsRow.implicitWidth
            contentHeight: height
            flickableDirection: Flickable.HorizontalFlick
            boundsBehavior: Flickable.StopAtBounds
            interactive: contentWidth > width
            clip: true

            Row {
                id: tabsRow
                spacing: Kirigami.Units.gridUnit * 1.3
                Repeater {
                    model: page.tabsShown
                    delegate: QQC2.AbstractButton {
                        id: tabButton
                        required property string modelData
                        readonly property bool current: page.tab === modelData
                        implicitWidth: implicitContentWidth + leftPadding + rightPadding
                        implicitHeight: implicitContentHeight + topPadding + bottomPadding
                        topPadding: Kirigami.Units.largeSpacing
                        bottomPadding: Kirigami.Units.largeSpacing
                        leftPadding: 2
                        rightPadding: 2
                        hoverEnabled: true
                        text: page.tabTitle(modelData)
                        onClicked: bar.chosen(modelData)

                        contentItem: RowLayout {
                            spacing: Kirigami.Units.smallSpacing * 1.5
                            Kirigami.Icon {
                                implicitWidth: Kirigami.Units.iconSizes.small
                                implicitHeight: Kirigami.Units.iconSizes.small
                                source: app.sym(page.tabIcon(tabButton.modelData))
                                fallback: page.tabIcon(tabButton.modelData)
                                color: tabButton.current ? Kirigami.Theme.linkColor : tabLabel.color
                            }
                            QQC2.Label {
                                id: tabLabel
                                text: tabButton.text
                                font.weight: tabButton.current ? Font.DemiBold : Font.Medium
                                color: tabButton.current || tabButton.hovered ? Kirigami.Theme.textColor : Qt.alpha(Kirigami.Theme.textColor, 0.72)
                            }
                            // Unread conversations and other notifications
                            Rectangle {
                                visible: tabButton.modelData === "inbox" && inbox.count > 0
                                implicitWidth: Math.max(implicitHeight, countLabel.implicitWidth + Kirigami.Units.smallSpacing * 2)
                                implicitHeight: countLabel.implicitHeight + 2
                                radius: height / 2
                                color: Kirigami.Theme.linkColor
                                QQC2.Label {
                                    id: countLabel
                                    anchors.centerIn: parent
                                    text: String(inbox.count)
                                    font.pointSize: Kirigami.Theme.defaultFont.pointSize * 0.8
                                    font.weight: Font.DemiBold
                                    color: Kirigami.Theme.highlightedTextColor
                                }
                            }
                        }
                        background: Item {
                            Rectangle {
                                anchors.fill: parent
                                visible: tabButton.visualFocus
                                color: "transparent"
                                radius: Kirigami.Units.cornerRadius
                                border.width: 1
                                border.color: Kirigami.Theme.focusColor
                            }
                            Rectangle {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                height: 2
                                visible: tabButton.current
                                color: Kirigami.Theme.highlightColor
                            }
                        }
                    }
                }
            }
        }
    }

    // One of the History tab's kinds, filled in when chosen
    component FilterPill: QQC2.AbstractButton {
        id: pill
        property bool chosen: false
        hoverEnabled: true
        implicitWidth: implicitContentWidth + leftPadding + rightPadding
        implicitHeight: implicitContentHeight + topPadding + bottomPadding
        topPadding: Kirigami.Units.smallSpacing
        bottomPadding: Kirigami.Units.smallSpacing
        leftPadding: Kirigami.Units.largeSpacing * 1.5
        rightPadding: Kirigami.Units.largeSpacing * 1.5
        Accessible.checkable: true
        Accessible.checked: pill.chosen
        contentItem: QQC2.Label {
            text: pill.text
            font.weight: Font.DemiBold
            color: pill.chosen ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
        }
        background: Rectangle {
            radius: height / 2
            color: pill.chosen ? Kirigami.Theme.highlightColor : Qt.alpha(Kirigami.Theme.textColor, pill.down ? 0.16 : pill.hovered ? 0.11 : 0.07)
            border.width: pill.visualFocus ? 1 : 0
            border.color: Kirigami.Theme.focusColor
        }
    }

    // A setting: what it is, what it does, and its switch
    component SwitchRow: ColumnLayout {
        id: switchRow
        property string iconName
        property string title
        property string subtitle
        property bool checked
        signal switched(bool now)
        Layout.fillWidth: true
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.largeSpacing
            Layout.bottomMargin: Kirigami.Units.largeSpacing
            Layout.leftMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.largeSpacing * 1.5
            Kirigami.Icon {
                Layout.alignment: Qt.AlignTop
                implicitWidth: Kirigami.Units.iconSizes.smallMedium
                implicitHeight: implicitWidth
                source: app.sym(switchRow.iconName)
                fallback: switchRow.iconName
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                QQC2.Label {
                    Layout.fillWidth: true
                    text: switchRow.title
                    textFormat: Text.PlainText
                    wrapMode: Text.Wrap
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    text: switchRow.subtitle
                    textFormat: Text.PlainText
                    wrapMode: Text.Wrap
                    font: Kirigami.Theme.smallFont
                    opacity: 0.7
                }
            }
            QQC2.Switch {
                Layout.alignment: Qt.AlignVCenter
                checked: switchRow.checked
                Accessible.name: switchRow.title
                onToggled: switchRow.switched(checked)
            }
        }
        Kirigami.Separator {
            Layout.fillWidth: true
        }
    }

    // One of the everyday actions: its icon in its colour, its name, what it does on hover
    component BarButton: QQC2.ToolButton {
        property string iconName
        property color tint: Kirigami.Theme.linkColor
        property string hint
        Layout.fillWidth: true
        display: QQC2.AbstractButton.TextBesideIcon
        icon.name: app.sym(iconName)
        icon.color: tint
        QQC2.ToolTip.visible: hovered && hint !== ""
        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
        QQC2.ToolTip.text: hint
    }

    // A card's actions as a section of its own, under a line across the card
    component CardActions: ColumnLayout {
        id: cardActions
        required property var owner
        spacing: 0
        Kirigami.Separator {
            Layout.fillWidth: true
            Layout.leftMargin: -cardActions.owner.leftPadding
            Layout.rightMargin: -cardActions.owner.rightPadding
            opacity: 0.6
        }
        Kirigami.ActionToolBar {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.smallSpacing
            actions: cardActions.owner.actions
        }
    }

    // One "Use as" card: start it, or stop it while it runs
    component UseCard: Kirigami.Card {
        id: card
        padding: Kirigami.Units.largeSpacing * 2
        property string iconName
        property string title
        property string subtitle
        property string button
        property string kind
        readonly property var running: app.sessions.find(s => s.kind === card.kind && s.peer.id === page.info?.id) ?? null
        // Equal columns, and as tall as the other card in their row
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.preferredWidth: Kirigami.Units.gridUnit * 14
        // Kirigami stretches a card's contentItem to the card's full height when the card grows (as
        // its row does for the other card), and a stretched ColumnLayout centres its rows onto the
        // actions: the column inside keeps its own height
        contentItem: Item {
            implicitWidth: column.implicitWidth
            implicitHeight: column.implicitHeight
            ColumnLayout {
                id: column
                width: parent.width
                spacing: Kirigami.Units.largeSpacing
                CardHead {
                    Layout.fillWidth: true
                    title: card.title
                    iconName: card.iconName
                    description: card.subtitle
                    live: card.running !== null
                }
                // A microphone in use here: what it hears
                MicWave {
                    Layout.fillWidth: true
                    visible: card.running !== null && card.kind === "microphone"
                }
            }
        }
        footer: CardActions { owner: card }
        actions: [
            Kirigami.Action {
                // A camera in use here: see what apps get, with the phone's controls
                visible: card.running !== null && card.kind === "camera" && !card.running.sharing
                text: app.t("Preview")
                icon.name: app.sym("view-preview")
                onTriggered: app.previewCamera(card.running.session, page.name)
            },
            Kirigami.Action {
                text: card.running ? app.t("Stop") : card.button
                icon.name: app.sym(card.running ? "media-playback-stop" : "media-playback-start")
                icon.color: card.running ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.linkColor
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

    // A card's head: its icon in a round tint (and Live while it runs), its title, what it does
    component CardHead: ColumnLayout {
        id: head
        property string title
        property string iconName
        property string description
        property bool live: false
        spacing: Kirigami.Units.smallSpacing
        RowLayout {
            Layout.fillWidth: true
            Layout.bottomMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.largeSpacing
            Rectangle {
                implicitWidth: Math.round(Kirigami.Units.gridUnit * 2.4)
                implicitHeight: implicitWidth
                radius: width / 2
                color: Qt.alpha(Kirigami.Theme.linkColor, 0.14)
                Kirigami.Icon {
                    anchors.centerIn: parent
                    width: Kirigami.Units.iconSizes.smallMedium
                    height: width
                    source: app.sym(head.iconName)
                    fallback: head.iconName
                    color: Kirigami.Theme.linkColor
                }
            }
            Item { Layout.fillWidth: true }
            Rectangle {
                Layout.alignment: Qt.AlignTop
                visible: head.live
                implicitWidth: liveLabel.implicitWidth + Kirigami.Units.largeSpacing * 2
                implicitHeight: liveLabel.implicitHeight + 4
                radius: height / 2
                color: Qt.alpha(Kirigami.Theme.positiveTextColor, 0.16)
                QQC2.Label {
                    id: liveLabel
                    anchors.centerIn: parent
                    text: app.t("Live")
                    font.pointSize: Kirigami.Theme.defaultFont.pointSize * 0.85
                    font.weight: Font.DemiBold
                    color: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.positiveTextColor, Kirigami.Theme.textColor, 0.3)
                }
            }
        }
        Kirigami.Heading {
            Layout.fillWidth: true
            level: 4
            font.weight: Font.DemiBold
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            text: head.title
        }
        QQC2.Label {
            Layout.fillWidth: true
            text: head.description
            wrapMode: Text.Wrap
            opacity: 0.75
        }
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
