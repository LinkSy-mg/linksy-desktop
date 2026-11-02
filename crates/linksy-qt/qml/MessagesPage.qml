// A phone's conversations, made from its message notifications: the list, and the open one
// with a field to answer; one at a time when the window is narrow.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigamiaddons.components as Components

Kirigami.Page {
    id: page
    required property var app
    required property string phone
    required property string name
    // Newest first
    property var conversations: []
    property var messages: []
    property string openId: ""
    property bool loaded: false
    property string error: ""
    readonly property var current: conversations.find(c => c.id === openId) ?? null
    readonly property bool narrow: width < Kirigami.Units.gridUnit * 30

    title: narrow && current ? current.title : app.t("Messages")
    padding: 0

    actions: [
        Kirigami.Action {
            visible: page.narrow && page.current !== null
            icon.name: "go-previous"
            text: app.t("Conversations")
            onTriggered: page.openId = ""
        }
    ]

    Component.onCompleted: {
        app.request("inbox", {}, (ok, list) => {
            page.loaded = true;
            if (!ok) { page.error = list; return }
            page.conversations = list.filter(c => c.phone === page.phone);
            // Development aid (scripts/qt-shot.sh): LINKSY_SCREENSHOT_THREAD=<title> opens one
            const title = backend.env("LINKSY_SCREENSHOT_THREAD");
            const c = title ? page.conversations.find(c => c.title === title) : null;
            if (c) page.openConversation(c.id);
        });
    }

    Connections {
        target: page.app
        function onConversationChanged(c) {
            if (c.phone !== page.phone) return;
            const list = page.conversations.filter(o => o.id !== c.id);
            const at = list.findIndex(o => o.at_ms <= c.at_ms);
            list.splice(at < 0 ? list.length : at, 0, c);
            page.conversations = list;
            if (c.id !== page.openId) return;
            if (page.app.active) page.markRead(c);
            page.loadThread();
        }
        // Back to the window with a conversation open: what came in meanwhile is read now
        function onActiveChanged() {
            if (page.app.active && page.current) page.markRead(page.current);
        }
    }

    function openConversation(id) {
        openId = id;
        messages = [];
        const c = current;
        if (!c) return;
        markRead(c);
        loadThread();
        if (c.can_reply) reply.forceActiveFocus();
    }

    function markRead(c) {
        if (c.unread > 0) app.request("inbox_read", { conversation: c.id });
    }

    function loadThread() {
        const id = openId;
        app.request("inbox_thread", { conversation: id }, (ok, list) => {
            if (id !== page.openId) return;
            if (!ok) return app.toast(list);
            page.messages = list;
            Qt.callLater(() => thread.positionViewAtEnd());
        });
    }

    function send() {
        const text = reply.text.trim();
        if (!text || !current) return;
        reply.enabled = false;
        app.request("inbox_reply", { conversation: openId, text: text }, (ok, err) => {
            reply.enabled = true;
            if (!ok) return app.toast(app.t("Not sent: {error}", { error: err.charAt(0).toUpperCase() + err.slice(1) }));
            reply.text = "";
            reply.forceActiveFocus();
        });
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

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // The conversations
        Item {
            visible: !page.narrow || page.current === null
            Layout.fillHeight: true
            Layout.fillWidth: page.narrow
            Layout.preferredWidth: Kirigami.Units.gridUnit * 18

            QQC2.ScrollView {
                anchors.fill: parent
                ListView {
                    id: list
                    model: page.conversations
                    delegate: QQC2.ItemDelegate {
                        id: row
                        required property var modelData
                        width: ListView.view.width
                        highlighted: modelData.id === page.openId
                        onClicked: page.openConversation(modelData.id)

                        contentItem: RowLayout {
                            spacing: Kirigami.Units.largeSpacing
                            Components.Avatar {
                                Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                                Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                                name: row.modelData.title
                                // A new name when it changes, so it isn't served from the cache
                                source: row.modelData.picture ? "file://" + row.modelData.picture + "?" + row.modelData.at_ms : ""
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                QQC2.Label {
                                    Layout.fillWidth: true
                                    text: row.modelData.title
                                    textFormat: Text.PlainText
                                    elide: Text.ElideRight
                                    font.bold: row.modelData.unread > 0
                                }
                                QQC2.Label {
                                    Layout.fillWidth: true
                                    text: page.preview(row.modelData)
                                    textFormat: Text.PlainText
                                    font: Kirigami.Theme.smallFont
                                    opacity: 0.7
                                    elide: Text.ElideRight
                                }
                            }
                            ColumnLayout {
                                spacing: Kirigami.Units.smallSpacing
                                QQC2.Label {
                                    Layout.alignment: Qt.AlignRight
                                    text: page.when(row.modelData.at_ms)
                                    font: Kirigami.Theme.smallFont
                                    color: row.modelData.unread > 0 ? Kirigami.Theme.highlightColor : Kirigami.Theme.disabledTextColor
                                }
                                Kirigami.Icon {
                                    Layout.alignment: Qt.AlignRight
                                    implicitWidth: Kirigami.Units.iconSizes.small
                                    implicitHeight: implicitWidth
                                    source: row.modelData.icon ? "file://" + row.modelData.icon : page.kindIcon(row.modelData.kind)
                                }
                            }
                        }
                    }
                }
            }

            Kirigami.PlaceholderMessage {
                anchors.centerIn: parent
                width: parent.width - Kirigami.Units.gridUnit * 2
                visible: page.conversations.length === 0
                icon.name: "dialog-messages"
                text: !page.loaded ? app.t("Loading…") : page.error ? app.t("Can't Reach Its Messages") : app.t("No Messages")
                explanation: page.error ? page.error
                           : page.loaded ? app.t("New messages from {device}'s chats, texts and mail show up here.", { device: page.name }) : ""
            }
        }

        Kirigami.Separator {
            visible: !page.narrow
            Layout.fillHeight: true
        }

        // The open conversation
        Item {
            visible: !page.narrow || page.current !== null
            Layout.fillWidth: true
            Layout.fillHeight: true

            Kirigami.PlaceholderMessage {
                anchors.centerIn: parent
                width: parent.width - Kirigami.Units.gridUnit * 4
                visible: page.current === null
                icon.name: "dialog-messages"
                text: app.t("No Conversation Selected")
            }

            ColumnLayout {
                anchors.fill: parent
                visible: page.current !== null
                spacing: 0

                // Whose, in which app (in the page's title when narrow)
                Kirigami.Heading {
                    visible: !page.narrow
                    Layout.fillWidth: true
                    Layout.margins: Kirigami.Units.largeSpacing
                    level: 4
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    text: page.current ? `${page.current.title} · ${page.current.app_name}` : ""
                }
                Kirigami.Separator {
                    visible: !page.narrow
                    Layout.fillWidth: true
                }

                QQC2.ScrollView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    ListView {
                        id: thread
                        model: page.messages
                        spacing: Kirigami.Units.largeSpacing
                        topMargin: Kirigami.Units.largeSpacing
                        bottomMargin: Kirigami.Units.largeSpacing
                        delegate: Item {
                            id: line
                            required property var modelData
                            readonly property bool mine: modelData.mine
                            width: ListView.view.width
                            implicitHeight: column.implicitHeight

                            Column {
                                id: column
                                x: line.mine ? line.width - width - Kirigami.Units.gridUnit : Kirigami.Units.gridUnit
                                spacing: 2
                                QQC2.Label {
                                    visible: (page.current?.group ?? false) && !line.mine && !!line.modelData.sender
                                    text: line.modelData.sender ?? ""
                                    textFormat: Text.PlainText
                                    font: Kirigami.Theme.smallFont
                                    opacity: 0.7
                                }
                                Rectangle {
                                    anchors.right: line.mine ? parent.right : undefined
                                    width: body.width + Kirigami.Units.largeSpacing * 2
                                    height: body.height + Kirigami.Units.smallSpacing * 3
                                    radius: Kirigami.Units.gridUnit
                                    color: line.mine ? Kirigami.Theme.highlightColor : Qt.alpha(Kirigami.Theme.textColor, 0.08)
                                    QQC2.Label {
                                        id: body
                                        anchors.centerIn: parent
                                        width: Math.min(implicitWidth, line.width * 0.7)
                                        text: line.modelData.text
                                        textFormat: Text.PlainText
                                        wrapMode: Text.Wrap
                                        color: line.mine ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                                    }
                                }
                                QQC2.Button {
                                    visible: !!line.modelData.code
                                    flat: true
                                    icon.name: "edit-copy"
                                    text: app.t("Copy {code}", { code: line.modelData.code ?? "" })
                                    onClicked: { backend.setClipboard(line.modelData.code); app.toast(app.t("Copied")) }
                                }
                                QQC2.Label {
                                    anchors.right: line.mine ? parent.right : undefined
                                    text: page.when(line.modelData.at_ms)
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
                    visible: page.current?.can_reply ?? false
                    Layout.fillWidth: true
                    Layout.margins: Kirigami.Units.largeSpacing
                    QQC2.TextField {
                        id: reply
                        Layout.fillWidth: true
                        placeholderText: app.t("Reply")
                        onAccepted: page.send()
                    }
                    QQC2.Button {
                        icon.name: "document-send"
                        text: app.t("Send")
                        display: QQC2.AbstractButton.IconOnly
                        enabled: reply.enabled && reply.text.trim() !== ""
                        QQC2.ToolTip.text: text
                        QQC2.ToolTip.visible: hovered
                        onClicked: page.send()
                    }
                }
                QQC2.Label {
                    visible: !(page.current?.can_reply ?? true)
                    Layout.fillWidth: true
                    Layout.margins: Kirigami.Units.largeSpacing
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    opacity: 0.7
                    text: app.t("This one can't be answered from here. Answer it on the phone.")
                }
            }
        }
    }
}
