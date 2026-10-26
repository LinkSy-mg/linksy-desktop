// Transfers in a corner, as the GTK app has them: while files go either way, a panel at
// the bottom right of the window shows each one (what, with whom, progress, Cancel),
// whichever page is open. It folds to one bar, and goes away a few seconds after the last
// one ends, with Open and Show in Folder until then (app.corner, kept by Main.qml).
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

QQC2.Pane {
    id: panel
    required property var app
    readonly property var entries: app.corner
    readonly property var running: entries.filter(e => e.status.state === "in_progress")
    property bool folded: false

    visible: entries.length > 0 && !app.transfersOpen
    width: Math.min(Kirigami.Units.gridUnit * 20, parent.width - Kirigami.Units.largeSpacing * 2)
    anchors.right: parent.right
    anchors.bottom: parent.bottom
    anchors.margins: Kirigami.Units.largeSpacing
    padding: Kirigami.Units.largeSpacing
    Accessible.name: app.t("Transfers")

    background: Rectangle {
        color: Kirigami.Theme.backgroundColor
        radius: Kirigami.Units.cornerRadius
        border.width: 1
        border.color: Qt.alpha(Kirigami.Theme.textColor, 0.2)
    }

    function plural(n, word) {
        if (word === "photo") return n === 1 ? app.t("1 photo") : app.t("{n} photos", { n: n });
        return n === 1 ? app.t("1 item") : app.t("{n} items", { n: n });
    }

    // Folded: one bar for everything running
    readonly property real overall: {
        let bytes = 0, total = 0;
        for (const e of running) {
            const p = app.progress[e.id];
            if (p) { bytes += p.bytes; total += p.total }
        }
        return total > 0 ? bytes / total : 0;
    }

    contentItem: ColumnLayout {
        spacing: Kirigami.Units.smallSpacing
        RowLayout {
            Layout.fillWidth: true
            QQC2.ToolButton {
                text: panel.app.t("Transfers")
                font.bold: true
                onClicked: panel.app.openTransfers()
            }
            QQC2.ProgressBar {
                Layout.fillWidth: true
                visible: panel.folded && panel.running.length > 0
                from: 0
                to: 1
                value: panel.overall
            }
            Item { Layout.fillWidth: !(panel.folded && panel.running.length > 0) }
            QQC2.ToolButton {
                icon.name: panel.folded ? "arrow-up" : "arrow-down"
                display: QQC2.AbstractButton.IconOnly
                text: panel.folded ? panel.app.t("Show More") : panel.app.t("Show Less")
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                onClicked: panel.folded = !panel.folded
            }
            QQC2.ToolButton {
                visible: panel.running.length === 0
                icon.name: "window-close"
                display: QQC2.AbstractButton.IconOnly
                text: panel.app.t("Close")
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                onClicked: panel.app.dismissCorner()
            }
        }
        QQC2.ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(rows.implicitHeight, Kirigami.Units.gridUnit * 16)
            visible: !panel.folded
            QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff
            ColumnLayout {
                id: rows
                width: panel.availableWidth
                spacing: Kirigami.Units.largeSpacing
                Repeater {
                    model: panel.entries
                    RowLayout {
                        id: row
                        required property var modelData
                        readonly property var e: modelData
                        readonly property var what: e.what
                        readonly property bool incoming: e.direction === "incoming"
                        readonly property string status: e.status.state
                        readonly property var progress: panel.app.progress[e.id] ?? null
                        readonly property var saved: what.saved ?? []
                        // `import` is a keyword QML's parser trips on in a one-line binding
                        readonly property bool imported: what["import"] === true
                        Layout.fillWidth: true
                        spacing: Kirigami.Units.smallSpacing

                        readonly property string detail: {
                            const amount = panel.app.size(what.total);
                            switch (status) {
                            case "done": return [e.peer_name, panel.app.t(imported ? "Imported" : incoming ? "Received" : "Sent"), amount].join(" · ");
                            case "declined": return [e.peer_name, panel.app.t("Declined")].join(" · ");
                            case "cancelled": return [e.peer_name, panel.app.t("Cancelled")].join(" · ");
                            case "failed": return [e.peer_name, panel.app.t("Failed: {reason}", { reason: e.status.reason })].join(" · ");
                            }
                            if (progress && progress.total > 0) return `${e.peer_name} · ${panel.app.progressText(progress)}`;
                            return `${e.peer_name} · ${incoming ? panel.app.t("Starting…") : panel.app.t("Waiting for them to accept…")}`;
                        }

                        Kirigami.Icon {
                            Layout.alignment: Qt.AlignTop
                            Layout.preferredWidth: Kirigami.Units.iconSizes.small
                            Layout.preferredHeight: Kirigami.Units.iconSizes.small
                            source: row.imported ? "image-x-generic" : row.incoming ? "folder-download" : "document-send"
                            color: row.status === "in_progress" ? Kirigami.Theme.highlightColor : Kirigami.Theme.textColor
                            opacity: ["failed", "declined", "cancelled"].includes(row.status) ? 0.4 : 0.8
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2
                            RowLayout {
                                Layout.fillWidth: true
                                QQC2.Label {
                                    Layout.fillWidth: true
                                    text: row.imported ? panel.plural(row.what.files.length, "photo")
                                        : row.what.files.length === 1 ? row.what.files[0].split("/").pop() : panel.plural(row.what.files.length, "item")
                                    font.bold: true
                                    elide: Text.ElideMiddle
                                }
                                QQC2.ToolButton {
                                    visible: row.status === "in_progress"
                                    icon.name: "dialog-cancel"
                                    display: QQC2.AbstractButton.IconOnly
                                    text: panel.app.t("Cancel")
                                    QQC2.ToolTip.text: text
                                    QQC2.ToolTip.visible: hovered
                                    onClicked: panel.app.request("cancel", { transfer: row.e.id })
                                }
                                QQC2.ToolButton {
                                    visible: row.status === "done" && row.incoming && row.saved.length > 0
                                    text: panel.app.t("Open")
                                    // One file opens; several, their folder
                                    onClicked: Qt.openUrlExternally("file://" + (row.saved.length === 1 ? row.saved[0] : row.saved[0].slice(0, row.saved[0].lastIndexOf("/"))))
                                }
                                QQC2.ToolButton {
                                    visible: row.status === "done" && row.incoming && row.saved.length > 0
                                    icon.name: "folder-open"
                                    display: QQC2.AbstractButton.IconOnly
                                    text: panel.app.t("Show in Folder")
                                    QQC2.ToolTip.text: text
                                    QQC2.ToolTip.visible: hovered
                                    onClicked: Qt.openUrlExternally("file://" + row.saved[0].slice(0, row.saved[0].lastIndexOf("/")))
                                }
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: row.detail
                                opacity: 0.7
                                font: Kirigami.Theme.smallFont
                                elide: Text.ElideRight
                            }
                            QQC2.ProgressBar {
                                Layout.fillWidth: true
                                visible: row.status === "in_progress"
                                from: 0
                                to: Math.max(1, row.progress?.total ?? 1)
                                value: row.progress?.bytes ?? 0
                            }
                        }
                    }
                }
            }
        }
    }
}
