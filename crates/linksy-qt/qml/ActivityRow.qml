// One thing shared with a device, as the GTK app's timeline says it: when, what, and how
// it went; a transfer in progress has its bar and Cancel
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

RowLayout {
    id: row
    required property var app
    required property var modelData
    readonly property var e: modelData
    readonly property var what: e.what
    readonly property bool incoming: e.direction === "incoming"
    readonly property bool running: e.status.state === "in_progress"
    readonly property bool live: ["screen", "camera", "microphone", "remote", "terminal"].includes(what.kind)
    readonly property var progress: app.progress[e.id] ?? null

    spacing: Kirigami.Units.largeSpacing

    function plural(n, word) {
        if (word === "photo") return n === 1 ? app.t("1 photo") : app.t("{n} photos", { n: n });
        return n === 1 ? app.t("1 item") : app.t("{n} items", { n: n });
    }

    function oneLine(text, max) {
        const flat = text.replace(/\s+/g, " ").trim();
        return flat.length > max ? flat.slice(0, max - 1) + "…" : flat;
    }

    function duration(s) {
        if (s < 60) return app.t("{seconds} s", { seconds: s });
        if (s < 3600) return app.t("{minutes} min", { minutes: Math.floor(s / 60) });
        return app.t("{hours} h {minutes} min", { hours: Math.floor(s / 3600), minutes: Math.floor(s % 3600 / 60) });
    }

    readonly property var look: {
        switch (what.kind) {
        case "files": {
            // `import` reads as QML's import statement here: by name instead
            const count = what.files.length + (what.more ?? 0);
            if (what["import"]) return ["image-x-generic", plural(count, "photo")];
            return [incoming ? "folder-download" : "document-send",
                    count === 1 ? what.files[0].split("/").pop() : plural(count, "item")];
        }
        case "clipboard": return ["edit-paste", what.text ? `“${oneLine(what.text, 80)}”` : app.t("Clipboard text")];
        case "ping": return ["preferences-desktop-notification-bell", what.body ? `${what.title}: ${what.body}` : what.title];
        case "link": return ["internet-web-browser", oneLine(what.url, 120)];
        case "locked": return ["system-lock-screen", app.t("Locked this computer")];
        case "remote": return ["input-touchpad", app.t("Touchpad and keyboard")];
        case "camera": return ["camera-web", incoming ? app.t("Their camera") : app.t("Your camera")];
        case "microphone": return ["audio-input-microphone", incoming ? app.t("Their microphone") : app.t("Your microphone")];
        case "terminal": return ["utilities-terminal", app.t("Terminal")];
        case "screen":
            if (what.display) return ["video-display", incoming ? app.t("Their second display") : app.t("Second display")];
            return ["video-display", incoming ? app.t("Their screen") : app.t("Your screen")];
        }
        return ["dialog-information", ""];
    }

    readonly property string detail: {
        const verb = {
            files: what.import ? "Imported" : incoming ? "Received" : "Sent",
            camera: incoming ? "Used" : "Shared",
            microphone: incoming ? "Used" : "Shared",
            screen: incoming ? "Watched" : "Shared",
            link: incoming ? "Opened here" : "Opened there",
            locked: "When it left",
            terminal: "Opened from the phone",
            remote: "Used"
        }[what.kind] ?? (incoming ? "From them" : "From you");
        const amount = what.kind === "files" ? app.size(what.total) : (what.seconds > 0 ? duration(what.seconds) : "");
        switch (e.status.state) {
        case "done": return [app.t(verb), amount].filter(x => x).join(" · ");
        case "declined": return app.t("Declined");
        case "cancelled": return app.t("Cancelled");
        case "failed": return app.t("Failed: {reason}", { reason: e.status.reason });
        }
        if (live) return app.t("Live now");
        if (progress && progress.total > 0) return app.progressText(progress);
        return incoming ? app.t("Starting…") : app.t("Waiting for them to accept…");
    }

    QQC2.Label {
        Layout.alignment: Qt.AlignTop
        Layout.preferredWidth: Kirigami.Units.gridUnit * 2.5
        horizontalAlignment: Text.AlignRight
        text: new Date(row.e.at).toLocaleTimeString(Qt.locale(), "HH:mm")
        opacity: 0.6
        font: Kirigami.Theme.smallFont
        topPadding: 2
    }
    Kirigami.Icon {
        Layout.alignment: Qt.AlignTop
        Layout.preferredWidth: Kirigami.Units.iconSizes.small
        Layout.preferredHeight: Kirigami.Units.iconSizes.small
        source: row.look[0]
        color: row.running ? Kirigami.Theme.highlightColor : Kirigami.Theme.textColor
        opacity: ["failed", "declined", "cancelled"].includes(row.e.status.state) ? 0.4 : 0.8
    }
    ColumnLayout {
        Layout.fillWidth: true
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        spacing: 2
        RowLayout {
            Layout.fillWidth: true
            QQC2.Label {
                Layout.fillWidth: true
                text: row.look[1]
                font.bold: true
                elide: Text.ElideMiddle
            }
            QQC2.ToolButton {
                visible: row.running && !row.live
                text: row.app.t("Cancel")
                onClicked: row.app.request("cancel", { transfer: row.e.id })
            }
            QQC2.ToolButton {
                visible: row.e.status.state === "done" && row.what.kind === "files" && row.incoming && (row.what.saved?.length ?? 0) > 0
                text: row.app.t("Show in Folder")
                onClicked: {
                    const first = row.what.saved[0];
                    Qt.openUrlExternally("file://" + first.slice(0, first.lastIndexOf("/")));
                }
            }
            QQC2.ToolButton {
                visible: row.what.kind === "link"
                text: row.app.t("Open Link")
                onClicked: Qt.openUrlExternally(row.what.url)
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
            visible: row.running && !row.live
            from: 0
            to: Math.max(1, row.progress?.total ?? 1)
            value: row.progress?.bytes ?? 0
            indeterminate: !row.progress || row.progress.total === 0
        }
    }
}
