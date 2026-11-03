// A phone's Inbox tab: its conversations, made from its message notifications, and its other
// notifications on the left; the open conversation on the right with a field to answer. One
// side at a time when the page is narrow.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigamiaddons.components as Components

Item {
    id: tab
    required property var app
    // The page's Inbox: what's in it, what's open, how it's filtered and sorted
    required property var store
    required property bool narrow
    readonly property var current: store.current
    readonly property var currentAlert: store.currentAlert
    // A conversation or another notification shows on the right
    readonly property bool opened: current !== null || currentAlert !== null
    readonly property var conversations: {
        const list = store.conversations.filter(c => tab.shows(c.kind) && (tab.matches(c.title) || tab.matches(tab.preview(c))));
        return store.oldestFirst ? list.reverse() : list;
    }
    readonly property var alerts: {
        if (store.show !== "all" && store.show !== "other") return [];
        const list = store.alerts.filter(a => tab.matches(a.title) || tab.matches(a.body) || tab.matches(a.app_name));
        return store.oldestFirst ? list.reverse() : list;
    }

    implicitHeight: Kirigami.Units.gridUnit * 31

    onConversationsChanged: shown.sync(conversations, c => c.id)

    Component.onCompleted: {
        shown.sync(conversations, c => c.id);
        showThread();
        search.text = store.search;
        if (current) {
            store.markRead(current);
            store.loadThread();
        }
    }

    Connections {
        target: tab.store
        function onThreadChanged() { tab.showThread() }
        function onSent() {
            reply.text = "";
            reply.forceActiveFocus();
        }
    }
    Connections {
        target: tab.app
        // Back to the window with a conversation open: what came in meanwhile is read now
        function onActiveChanged() { if (tab.app.active && tab.current) tab.store.markRead(tab.current) }
    }

    // The open conversation's messages: at its end, or with earlier ones just loaded, the
    // message at the top stays where it is
    function showThread() {
        let anchor = "", offset = 0;
        if (store.keepPlace) {
            store.keepPlace = false;
            for (let y = 0; y < thread.height && !anchor; y += 8) {
                const i = thread.indexAt(thread.width / 2, thread.contentY + y);
                if (i < 0) continue;
                anchor = messages.model.get(i).key;
                offset = (thread.itemAtIndex(i)?.y ?? thread.contentY) - thread.contentY;
            }
        }
        // Messages have no id: when, whose and what, and how many alike came before
        const keys = [], seen = {};
        for (const m of store.thread) {
            const key = `${m.at_ms}|${m.mine ? 1 : 0}|${m.text}`;
            seen[key] = (seen[key] ?? 0) + 1;
            keys.push(`${key}|${seen[key]}`);
        }
        messages.sync(store.thread, (m, i) => keys[i], m => ({ day: app.dayLabel(m.at_ms) }));
        if (!anchor) {
            Qt.callLater(() => thread.positionViewAtEnd());
            return;
        }
        for (let i = 0; i < messages.model.count; i++) {
            if (messages.model.get(i).key !== anchor) continue;
            thread.positionViewAtIndex(i, ListView.Beginning);
            thread.contentY -= offset;
            break;
        }
    }

    function shows(kind) {
        switch (store.show) {
        case "chats": return kind === "chat" || kind === "social";
        case "texts": return kind === "sms";
        case "mail": return kind === "mail";
        case "other": return false;
        }
        return true;
    }

    function matches(text) {
        const needle = store.search.toLowerCase();
        return !needle || (text ?? "").toLowerCase().includes(needle);
    }

    // The newest line, with who said it when that isn't plain
    function preview(c) {
        const m = c.last;
        if (!m) return "";
        const text = m.text.split("\n")[0];
        if (m.mine) return app.t("You: {text}", { text: text });
        if (c.group && m.sender) return `${m.sender}: ${text}`;
        return text;
    }

    function kindIcon(kind) {
        return { sms: "smartphone", mail: "mail-message", social: "group" }[kind] ?? "dialog-messages";
    }

    // "14:05" today, "Mon" this week, "12 Oct" before
    function when(ms) {
        const d = new Date(ms), now = new Date();
        if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString(Qt.locale(), "HH:mm");
        if (now - d < 6 * 86400000) return d.toLocaleDateString(Qt.locale(), "ddd");
        return d.toLocaleDateString(Qt.locale(), "d MMM");
    }

    function open(id) {
        store.openConversation(id);
        if (store.current?.can_reply) reply.forceActiveFocus();
    }

    function send() {
        store.reply(reply.text.trim());
    }

    KeyedList { id: shown }
    KeyedList { id: messages }

    QQC2.ButtonGroup { id: showGroup }
    QQC2.ButtonGroup { id: sortGroup }

    QQC2.Menu {
        id: menu
        MenuHeading { text: tab.app.t("Show") }
        ShowItem { value: "all"; text: tab.app.t("All") }
        ShowItem { value: "chats"; text: tab.app.t("Chats") }
        ShowItem { value: "texts"; text: tab.app.t("Texts") }
        ShowItem { value: "mail"; text: tab.app.t("Mail") }
        ShowItem { value: "other"; text: tab.app.t("Other Notifications") }
        QQC2.MenuSeparator {}
        MenuHeading { text: tab.app.t("Sort") }
        SortItem { oldest: false; text: tab.app.t("Newest First") }
        SortItem { oldest: true; text: tab.app.t("Oldest First") }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // The conversations, then the other notifications
        ColumnLayout {
            visible: !tab.narrow || !tab.opened
            Layout.fillHeight: true
            Layout.fillWidth: tab.narrow
            Layout.preferredWidth: Kirigami.Units.gridUnit * 17
            Layout.rightMargin: tab.narrow ? 0 : Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.smallSpacing

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing
                Kirigami.SearchField {
                    id: search
                    Layout.fillWidth: true
                    placeholderText: tab.app.t("Search")
                    onTextChanged: tab.store.search = text
                }
                QQC2.ToolButton {
                    id: sortButton
                    icon.name: "view-sort-descending"
                    // Filtered: the button says so
                    icon.color: tab.store.show !== "all" ? Kirigami.Theme.highlightColor : Kirigami.Theme.textColor
                    text: tab.app.t("Show and Sort")
                    display: QQC2.AbstractButton.IconOnly
                    down: menu.visible
                    QQC2.ToolTip.text: text
                    QQC2.ToolTip.visible: hovered && !menu.visible
                    onClicked: menu.popup(sortButton, 0, sortButton.height)
                }
            }

            QQC2.ScrollView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                ListView {
                    id: list
                    clip: true
                    model: shown.model
                    delegate: QQC2.ItemDelegate {
                        id: row
                        required property string key
                        readonly property var modelData: shown.rows[key]
                        readonly property bool unread: modelData.unread > 0
                        width: list.width
                        highlighted: modelData.id === tab.store.open
                        text: modelData.title
                        onClicked: tab.open(modelData.id)

                        contentItem: RowLayout {
                            spacing: Kirigami.Units.largeSpacing
                            Face {
                                Layout.alignment: Qt.AlignTop
                                conversation: row.modelData
                                badge: true
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2
                                RowLayout {
                                    Layout.fillWidth: true
                                    QQC2.Label {
                                        Layout.fillWidth: true
                                        text: row.modelData.title
                                        textFormat: Text.PlainText
                                        elide: Text.ElideRight
                                        font.weight: row.unread ? Font.DemiBold : Font.Normal
                                    }
                                    QQC2.Label {
                                        text: tab.when(row.modelData.at_ms)
                                        font: Kirigami.Theme.smallFont
                                        color: row.unread ? Kirigami.Theme.linkColor : Kirigami.Theme.disabledTextColor
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    QQC2.Label {
                                        Layout.fillWidth: true
                                        text: tab.preview(row.modelData)
                                        textFormat: Text.PlainText
                                        elide: Text.ElideRight
                                        opacity: 0.7
                                    }
                                    CountPill {
                                        visible: row.unread
                                        count: row.modelData.unread
                                    }
                                }
                                CodeChip {
                                    Layout.topMargin: Kirigami.Units.smallSpacing
                                    code: row.modelData.last?.code ?? ""
                                }
                            }
                        }
                    }

                    footer: ColumnLayout {
                        width: list.width
                        spacing: 0
                        RowLayout {
                            Layout.fillWidth: true
                            Layout.topMargin: Kirigami.Units.largeSpacing
                            Layout.leftMargin: Kirigami.Units.largeSpacing
                            visible: tab.alerts.length > 0
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: tab.app.t("Other Notifications")
                                font.pointSize: Kirigami.Theme.smallFont.pointSize
                                font.weight: Font.DemiBold
                                opacity: 0.7
                            }
                            QQC2.Button {
                                visible: tab.store.search === ""
                                flat: true
                                text: tab.app.t("Clear All")
                                onClicked: tab.store.dismiss()
                            }
                        }
                        Repeater {
                            model: tab.alerts
                            delegate: AlertRow {}
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            Layout.margins: Kirigami.Units.largeSpacing
                            Layout.topMargin: Kirigami.Units.gridUnit
                            visible: tab.conversations.length === 0 && tab.alerts.length === 0
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.Wrap
                            opacity: 0.7
                            text: !tab.store.loaded ? tab.app.t("Loading…")
                                : tab.store.error ? tab.store.error
                                : tab.store.conversations.length + tab.store.alerts.length === 0 ? tab.app.t("New messages and notifications from the phone show up here.")
                                : tab.app.t("Nothing matches.")
                        }
                    }
                }
            }
        }

        Kirigami.Separator {
            visible: !tab.narrow
            Layout.fillHeight: true
        }

        // The open conversation, or another notification in full
        Item {
            visible: !tab.narrow || tab.opened
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.leftMargin: tab.narrow ? 0 : Kirigami.Units.gridUnit

            Kirigami.PlaceholderMessage {
                anchors.centerIn: parent
                width: parent.width - Kirigami.Units.gridUnit * 4
                visible: !tab.opened
                icon.name: "dialog-messages"
                text: tab.app.t("No Conversation Selected")
            }

            // Another notification, all of it
            ColumnLayout {
                anchors.fill: parent
                visible: tab.current === null && tab.currentAlert !== null
                spacing: Kirigami.Units.largeSpacing

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.largeSpacing
                    QQC2.ToolButton {
                        visible: tab.narrow
                        icon.name: "go-previous"
                        text: tab.app.t("Back")
                        display: QQC2.AbstractButton.IconOnly
                        QQC2.ToolTip.text: text
                        QQC2.ToolTip.visible: hovered
                        onClicked: tab.store.close()
                    }
                    AlertIcon {
                        alert: tab.currentAlert
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Kirigami.Heading {
                            Layout.fillWidth: true
                            level: 4
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            text: tab.currentAlert?.title ?? ""
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            font: Kirigami.Theme.smallFont
                            opacity: 0.7
                            text: tab.currentAlert ? `${tab.currentAlert.app_name} · ${tab.when(tab.currentAlert.at_ms)}` : ""
                        }
                    }
                    QQC2.Button {
                        icon.name: "window-close"
                        text: tab.app.t("Dismiss")
                        onClicked: tab.store.dismiss(tab.currentAlert.key)
                    }
                }
                Kirigami.Separator {
                    Layout.fillWidth: true
                }
                QQC2.ScrollView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    QQC2.TextArea {
                        readOnly: true
                        selectByMouse: true
                        wrapMode: Text.Wrap
                        textFormat: TextEdit.PlainText
                        background: null
                        text: tab.currentAlert?.body ?? ""
                    }
                }
                CodeChip {
                    Layout.bottomMargin: Kirigami.Units.largeSpacing
                    code: tab.currentAlert?.code ?? ""
                }
            }

            ColumnLayout {
                anchors.fill: parent
                visible: tab.current !== null
                spacing: 0

                RowLayout {
                    Layout.fillWidth: true
                    Layout.bottomMargin: Kirigami.Units.largeSpacing
                    spacing: Kirigami.Units.largeSpacing
                    QQC2.ToolButton {
                        visible: tab.narrow
                        icon.name: "go-previous"
                        text: tab.app.t("Back")
                        display: QQC2.AbstractButton.IconOnly
                        QQC2.ToolTip.text: text
                        QQC2.ToolTip.visible: hovered
                        onClicked: tab.store.close()
                    }
                    Face {
                        conversation: tab.current
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Kirigami.Heading {
                            Layout.fillWidth: true
                            level: 4
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            text: tab.current?.title ?? ""
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            font: Kirigami.Theme.smallFont
                            opacity: 0.7
                            text: tab.current?.app_name ?? ""
                        }
                    }
                }
                Kirigami.Separator {
                    Layout.fillWidth: true
                }

                QQC2.ScrollView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    ListView {
                        id: thread
                        clip: true
                        model: messages.model
                        spacing: Kirigami.Units.smallSpacing
                        topMargin: Kirigami.Units.largeSpacing
                        bottomMargin: Kirigami.Units.largeSpacing
                        // The phone may have earlier ones to give
                        header: Item {
                            readonly property bool older: tab.current?.older ?? false
                            width: thread.width
                            height: older ? olderButton.implicitHeight + Kirigami.Units.smallSpacing : 0
                            visible: older
                            QQC2.Button {
                                id: olderButton
                                anchors.horizontalCenter: parent.horizontalCenter
                                flat: true
                                enabled: !tab.store.fetchingOlder
                                text: tab.app.t("Load Earlier Messages")
                                onClicked: tab.store.loadOlder()
                            }
                        }
                        section.property: "day"
                        section.delegate: QQC2.Label {
                            required property string section
                            width: thread.width
                            horizontalAlignment: Text.AlignHCenter
                            topPadding: Kirigami.Units.largeSpacing
                            bottomPadding: Kirigami.Units.smallSpacing
                            text: section
                            font: Kirigami.Theme.smallFont
                            opacity: 0.7
                        }
                        delegate: Item {
                            id: line
                            required property string key
                            readonly property var modelData: messages.rows[key]
                            readonly property bool mine: modelData.mine
                            // Lines are the bubble's side of the view, mine on the right
                            readonly property real side: Kirigami.Units.gridUnit
                            width: thread.width
                            implicitHeight: column.implicitHeight

                            Column {
                                id: column
                                width: parent.width
                                spacing: 2
                                QQC2.Label {
                                    visible: (tab.current?.group ?? false) && !line.mine && !!line.modelData.sender
                                    x: line.side + Kirigami.Units.largeSpacing
                                    text: line.modelData.sender ?? ""
                                    textFormat: Text.PlainText
                                    font: Kirigami.Theme.smallFont
                                    opacity: 0.7
                                }
                                Rectangle {
                                    x: line.mine ? line.width - width - line.side : line.side
                                    width: body.width + Kirigami.Units.largeSpacing * 2
                                    height: body.height + Kirigami.Units.smallSpacing * 3
                                    radius: Kirigami.Units.gridUnit
                                    // Yours in a soft tint of the accent, as in KDE's own chat apps
                                    color: line.mine ? Qt.alpha(Kirigami.Theme.highlightColor, 0.2) : Qt.alpha(Kirigami.Theme.textColor, 0.07)
                                    border.width: 1
                                    border.color: line.mine ? Qt.alpha(Kirigami.Theme.highlightColor, 0.45) : Qt.alpha(Kirigami.Theme.textColor, 0.1)
                                    QQC2.Label {
                                        id: body
                                        anchors.centerIn: parent
                                        width: Math.min(implicitWidth, line.width * 0.7)
                                        text: line.modelData.text
                                        textFormat: Text.PlainText
                                        wrapMode: Text.Wrap
                                    }
                                }
                                CodeChip {
                                    x: line.mine ? line.width - width - line.side : line.side
                                    code: line.modelData.code ?? ""
                                }
                                QQC2.Label {
                                    x: line.mine ? line.width - width - line.side - Kirigami.Units.largeSpacing : line.side + Kirigami.Units.largeSpacing
                                    text: new Date(line.modelData.at_ms).toLocaleTimeString(Qt.locale(), "HH:mm")
                                    font: Kirigami.Theme.smallFont
                                    opacity: 0.6
                                }
                            }
                        }
                    }
                }

                Kirigami.Separator {
                    Layout.fillWidth: true
                }
                RowLayout {
                    visible: tab.current?.can_reply ?? false
                    Layout.fillWidth: true
                    Layout.topMargin: Kirigami.Units.largeSpacing
                    QQC2.TextField {
                        id: reply
                        Layout.fillWidth: true
                        enabled: !tab.store.sending
                        placeholderText: tab.app.t("Reply on {app}", { app: tab.current?.app_name ?? "" })
                        onAccepted: tab.send()
                    }
                    QQC2.Button {
                        icon.name: "document-send"
                        text: tab.app.t("Send")
                        display: QQC2.AbstractButton.IconOnly
                        enabled: !tab.store.sending && reply.text.trim() !== ""
                        QQC2.ToolTip.text: text
                        QQC2.ToolTip.visible: hovered
                        onClicked: tab.send()
                    }
                }
                QQC2.Label {
                    visible: !(tab.current?.can_reply ?? true)
                    Layout.fillWidth: true
                    Layout.topMargin: Kirigami.Units.largeSpacing
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    opacity: 0.7
                    text: tab.app.t("This one can't be answered from here. Answer it on the phone.")
                }
            }
        }
    }

    // ---- small pieces ----

    // A conversation's picture, initials or group icon, with its app in the corner
    component Face: Item {
        id: face
        property var conversation: null
        property bool badge: false
        implicitWidth: Kirigami.Units.gridUnit * 2.4
        implicitHeight: implicitWidth
        Components.Avatar {
            anchors.fill: parent
            name: face.conversation?.title ?? ""
            // A new name when it changes, so it isn't served from the cache
            source: face.conversation?.picture ? "file://" + face.conversation.picture + "?" + face.conversation.at_ms : ""
            initialsMode: (face.conversation?.group ?? false) ? Components.Avatar.InitialsMode.UseIcon : Components.Avatar.InitialsMode.UseInitials
            iconSource: "group"
        }
        Rectangle {
            visible: face.badge
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.rightMargin: -2
            anchors.bottomMargin: -2
            width: Kirigami.Units.iconSizes.small + 4
            height: width
            radius: Kirigami.Units.cornerRadius
            color: Kirigami.Theme.backgroundColor
            border.width: 1
            border.color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
            Kirigami.Icon {
                anchors.centerIn: parent
                width: Kirigami.Units.iconSizes.small - 2
                height: width
                source: face.conversation?.icon ? "file://" + face.conversation.icon : tab.kindIcon(face.conversation?.kind ?? "")
            }
            HoverHandler { id: appHover }
            QQC2.ToolTip.visible: appHover.hovered
            QQC2.ToolTip.text: face.conversation?.app_name ?? ""
        }
    }

    // Its app's icon in a tile
    component AlertIcon: Rectangle {
        id: tile
        property var alert: null
        Layout.alignment: Qt.AlignTop
        implicitWidth: Kirigami.Units.gridUnit * 2.2
        implicitHeight: implicitWidth
        radius: Kirigami.Units.cornerRadius * 2
        color: Qt.alpha(Kirigami.Theme.textColor, 0.06)
        Kirigami.Icon {
            anchors.centerIn: parent
            width: Kirigami.Units.iconSizes.smallMedium
            height: width
            source: tile.alert?.icon ? "file://" + tile.alert.icon : app.sym("preferences-desktop-notification-bell")
            fallback: "preferences-desktop-notification-bell"
        }
    }

    // One of the phone's other notifications: opens in full, copies its code, goes away
    component AlertRow: QQC2.ItemDelegate {
        id: alert
        required property var modelData
        Layout.fillWidth: true
        highlighted: modelData.key === tab.store.alert
        text: modelData.title
        onClicked: tab.store.openAlert(modelData.key)

        contentItem: RowLayout {
            spacing: Kirigami.Units.largeSpacing
            AlertIcon {
                alert: alert.modelData
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                QQC2.Label {
                    Layout.fillWidth: true
                    text: alert.modelData.title
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    font.weight: Font.DemiBold
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    visible: text !== ""
                    text: (alert.modelData.body ?? "").split("\n")[0]
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    opacity: 0.7
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    text: `${alert.modelData.app_name} · ${tab.when(alert.modelData.at_ms)}`
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    font: Kirigami.Theme.smallFont
                    opacity: 0.6
                }
                CodeChip {
                    Layout.topMargin: Kirigami.Units.smallSpacing
                    code: alert.modelData.code ?? ""
                }
            }
            QQC2.ToolButton {
                Layout.alignment: Qt.AlignVCenter
                icon.name: "window-close"
                text: tab.app.t("Dismiss")
                display: QQC2.AbstractButton.IconOnly
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                onClicked: tab.store.dismiss(alert.modelData.key)
            }
        }
    }

    // "Copy 482913", as a chip
    component CodeChip: QQC2.AbstractButton {
        id: chip
        property string code
        visible: code !== ""
        hoverEnabled: true
        implicitWidth: implicitContentWidth + leftPadding + rightPadding
        implicitHeight: implicitContentHeight + topPadding + bottomPadding
        padding: Kirigami.Units.smallSpacing
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.largeSpacing
        text: tab.app.t("Copy {code}", { code: chip.code })
        onClicked: {
            backend.setClipboard(chip.code);
            tab.app.toast(tab.app.t("{code} copied", { code: chip.code }));
        }
        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing
            Kirigami.Icon {
                implicitWidth: Kirigami.Units.iconSizes.small
                implicitHeight: Kirigami.Units.iconSizes.small
                source: app.sym("edit-copy")
                fallback: "edit-copy"
                color: Kirigami.Theme.linkColor
            }
            QQC2.Label {
                text: chip.text
                font.weight: Font.DemiBold
                color: Kirigami.Theme.linkColor
            }
        }
        background: Rectangle {
            radius: Kirigami.Units.cornerRadius
            color: Qt.alpha(Kirigami.Theme.linkColor, chip.down ? 0.22 : chip.hovered ? 0.17 : 0.12)
            border.width: chip.visualFocus ? 1 : 0
            border.color: Kirigami.Theme.focusColor
        }
    }

    // A soft pill of the accent with its number in a deeper shade of it
    component CountPill: Rectangle {
        id: pill
        property int count
        implicitWidth: Math.max(implicitHeight, countLabel.implicitWidth + Kirigami.Units.smallSpacing * 3)
        implicitHeight: countLabel.implicitHeight + 4
        radius: height / 2
        color: Qt.alpha(Kirigami.Theme.highlightColor, 0.2)
        QQC2.Label {
            id: countLabel
            anchors.centerIn: parent
            text: String(pill.count)
            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 0.8
            font.weight: Font.DemiBold
            color: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.linkColor, Kirigami.Theme.textColor, 0.35)
        }
    }

    component MenuHeading: QQC2.Label {
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.largeSpacing
        topPadding: Kirigami.Units.smallSpacing
        bottomPadding: Kirigami.Units.smallSpacing
        font.pointSize: Kirigami.Theme.smallFont.pointSize
        font.weight: Font.DemiBold
        opacity: 0.7
    }

    component ShowItem: QQC2.MenuItem {
        id: showItem
        required property string value
        checkable: true
        checked: tab.store.show === showItem.value
        QQC2.ButtonGroup.group: showGroup
        onTriggered: tab.store.show = showItem.value
    }

    component SortItem: QQC2.MenuItem {
        id: sortItem
        required property bool oldest
        checkable: true
        checked: tab.store.oldestFirst === sortItem.oldest
        QQC2.ButtonGroup.group: sortGroup
        onTriggered: tab.store.oldestFirst = sortItem.oldest
    }
}
