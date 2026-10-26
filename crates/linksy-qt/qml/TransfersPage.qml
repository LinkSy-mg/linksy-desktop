// Transfers, a page of its own as on the phone: speed, files on their way, and recent ones
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    required property var app
    objectName: "transfers"
    title: app.t("Transfers")

    readonly property var moving: app.moving
    readonly property var first: moving.length > 0 ? moving[0] : null
    readonly property var device: first ? app.devices.find(d => d.device.id === first.peer) ?? null : null
    readonly property var route: {
        if (!first) return "wifi";
        if (first.peer in app.direct) return app.direct[first.peer] ? "direct" : "setup";
        if (device?.usb) return "usb";
        if (device?.away) return "away";
        return "wifi";
    }
    readonly property real rate: {
        let r = 0;
        for (const m of moving) {
            const p = app.progress[m.id];
            if (!p || p.samples.length < 2) continue;
            const [a, b] = [p.samples[0], p.samples[p.samples.length - 1]];
            if (b[0] > a[0]) r += (b[1] - a[1]) / ((b[0] - a[0]) / 1000);
        }
        return r;
    }
    readonly property real fraction: {
        let bytes = 0, total = 0;
        for (const m of moving) {
            const p = app.progress[m.id];
            bytes += p ? p.bytes : 0;
            total += p ? Math.max(p.total, m.total) : m.total;
        }
        return total > 0 ? bytes / total : 0;
    }
    readonly property bool started: moving.some(m => app.progress[m.id] !== undefined)
    readonly property color cyan: "#22C3E6"

    function plural(n, word) {
        if (word === "photo") return n === 1 ? app.t("1 photo") : app.t("{n} photos", { n: n });
        return n === 1 ? app.t("1 item") : app.t("{n} items", { n: n });
    }

    function fileIcon(name) {
        const ext = name.split(".").pop().toLowerCase();
        if (["jpg", "jpeg", "png", "gif", "webp", "heic", "avif", "bmp", "svg"].includes(ext)) return "image-x-generic";
        if (["mp4", "mkv", "mov", "webm", "avi", "m4v"].includes(ext)) return "video-x-generic";
        if (["mp3", "flac", "ogg", "opus", "wav", "m4a", "aac"].includes(ext)) return "audio-x-generic";
        if (ext === "pdf") return "application-pdf";
        if (["zip", "tar", "gz", "xz", "7z", "rar", "zst", "apk"].includes(ext)) return "package-x-generic";
        if (["odt", "doc", "docx", "ods", "xls", "xlsx", "odp", "ppt", "pptx"].includes(ext)) return "x-office-document";
        return "text-x-generic";
    }

    function doneFiles(m) {
        const p = app.progress[m.id];
        if (!p || m.files.some(f => f.size === null)) return -1;
        let sum = 0;
        for (let i = 0; i < m.files.length; i++) {
            sum += m.files[i].size;
            if (sum > p.bytes) return i;
        }
        return m.files.length;
    }

    function fileFraction(m, i) {
        const p = app.progress[m.id];
        if (!p || m.files[i].size === null || m.files[i].size === 0) return -1;
        let before = 0;
        for (let k = 0; k < i; k++) before += m.files[k].size;
        return Math.max(0, Math.min(1, (p.bytes - before) / m.files[i].size));
    }

    function feels() {
        const mb = rate / 1e6;
        if (mb < 15) return "";
        const lines = [app.t("A 1 GB movie every {seconds} s", { seconds: Math.round(1000 / mb) })];
        if (mb / 3 >= 2) lines.push(app.t("About {count} photos a second", { count: Math.round(mb / 3) }));
        if (mb / 5 >= 2) lines.push(app.t("{count} songs a second", { count: Math.round(mb / 5) }));
        return lines[Math.floor(Date.now() / 4000) % lines.length];
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        // Nothing on its way
        Rectangle {
            Layout.fillWidth: true
            visible: page.moving.length === 0
            implicitHeight: nothingRow.implicitHeight + Kirigami.Units.gridUnit * 2
            radius: Kirigami.Units.cornerRadius * 2
            color: Kirigami.Theme.backgroundColor
            border.color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
            RowLayout {
                id: nothingRow
                anchors.fill: parent
                anchors.margins: Kirigami.Units.gridUnit
                spacing: Kirigami.Units.largeSpacing
                Rectangle {
                    implicitWidth: Kirigami.Units.gridUnit * 3
                    implicitHeight: implicitWidth
                    radius: width / 2
                    color: Qt.alpha(Kirigami.Theme.highlightColor, 0.12)
                    Kirigami.Icon {
                        anchors.centerIn: parent
                        width: Kirigami.Units.iconSizes.smallMedium
                        height: width
                        source: "network-transmit-receive"
                        color: Kirigami.Theme.highlightColor
                    }
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    Kirigami.Heading {
                        level: 3
                        text: page.app.t("Nothing on its way")
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        opacity: 0.7
                        text: page.app.t("Files you send or receive show up here, with their speed.")
                    }
                }
            }
        }

        // The speed, and the lane files run along
        Rectangle {
            Layout.fillWidth: true
            visible: page.first !== null
            implicitHeight: speedColumn.implicitHeight + Kirigami.Units.gridUnit * 1.6
            radius: Kirigami.Units.cornerRadius * 2
            color: Kirigami.Theme.backgroundColor
            border.color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
            ColumnLayout {
                id: speedColumn
                anchors.fill: parent
                anchors.margins: Kirigami.Units.gridUnit * 0.8
                spacing: Kirigami.Units.largeSpacing
                RowLayout {
                    Layout.fillWidth: true
                    QQC2.Label {
                        visible: page.rate > 0
                        text: page.app.size(page.rate).split(" ")[0]
                        font.pixelSize: 28
                        font.weight: Font.Black
                    }
                    QQC2.Label {
                        visible: page.rate > 0
                        Layout.alignment: Qt.AlignBaseline
                        text: page.app.size(page.rate).split(" ").slice(1).join(" ") + "/s"
                        opacity: 0.6
                    }
                    QQC2.Label {
                        visible: page.rate <= 0
                        text: page.route === "setup" ? page.app.t("Setting up Wi-Fi Direct") : page.app.t("Waiting to start")
                        font.pixelSize: 20
                        font.bold: true
                    }
                    Item { Layout.fillWidth: true }
                    QQC2.Label {
                        visible: page.started
                        text: Math.round(page.fraction * 100) + "%"
                        color: Kirigami.Theme.highlightColor
                        font.bold: true
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.largeSpacing
                    LaneEnd {
                        icon: page.first && page.first.incoming ? page.kindIcon : "computer"
                        tint: page.first && page.first.incoming ? page.cyan : Kirigami.Theme.highlightColor
                    }
                    Item {
                        id: lane
                        Layout.fillWidth: true
                        implicitHeight: 32
                        property real phase: 0
                        property real pace: 0
                        Rectangle {
                            id: track
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width
                            height: 10
                            radius: 5
                            color: Qt.alpha(Kirigami.Theme.highlightColor, 0.14)
                            clip: true
                            Repeater {
                                model: 6
                                Rectangle {
                                    required property int index
                                    readonly property real t: (lane.phase + index / 6) % 1
                                    visible: lane.pace > 0.001
                                    width: 14
                                    height: 6
                                    radius: 3
                                    y: 2
                                    x: -width + t * (track.width + width)
                                    color: Kirigami.Theme.highlightColor
                                    opacity: Math.min(1, t * 8) * Math.min(1, (1 - t) * 8)
                                }
                            }
                        }
                        FrameAnimation {
                            running: page.visible && (page.rate > 0 || lane.pace > 0.001)
                            onTriggered: {
                                const mb = page.rate / 1e6;
                                const goal = mb <= 0 ? 0 : Math.max(0.12, Math.min(1.6, 0.12 + mb / 80));
                                lane.pace += (goal - lane.pace) * Math.min(1, frameTime * 3);
                                lane.phase = (lane.phase + lane.pace * frameTime) % 1;
                            }
                        }
                        Rectangle {
                            anchors.centerIn: parent
                            implicitWidth: pill.implicitWidth + 18
                            implicitHeight: pill.implicitHeight + 6
                            radius: height / 2
                            readonly property bool blue: page.route === "direct" || page.route === "setup"
                            color: Kirigami.Theme.backgroundColor
                            border.color: blue ? Qt.alpha(Kirigami.Theme.highlightColor, 0.3) : Qt.alpha(Kirigami.Theme.textColor, 0.15)
                            RowLayout {
                                id: pill
                                anchors.centerIn: parent
                                spacing: 4
                                Kirigami.Icon {
                                    implicitWidth: 12
                                    implicitHeight: 12
                                    source: ({ direct: "network-wireless-hotspot", setup: "network-wireless-acquiring", usb: "drive-removable-media-usb", away: "internet-web-browser" })[page.route] ?? "network-wireless"
                                    color: parent.parent.blue ? Kirigami.Theme.highlightColor : Kirigami.Theme.textColor
                                }
                                QQC2.Label {
                                    text: ({ direct: page.app.t("Wi-Fi Direct"), setup: page.app.t("Setting up Wi-Fi Direct"), usb: page.app.t("USB cable"), away: page.app.t("Away from home") })[page.route] ?? page.app.t("Same Wi-Fi")
                                    font.pixelSize: 11
                                    font.bold: true
                                    color: parent.parent.blue ? Kirigami.Theme.highlightColor : Kirigami.Theme.disabledTextColor
                                }
                            }
                        }
                    }
                    LaneEnd {
                        icon: page.first && page.first.incoming ? "computer" : page.kindIcon
                        tint: page.first && page.first.incoming ? Kirigami.Theme.highlightColor : page.cyan
                    }
                }
            }
        }

        QQC2.Label {
            id: feelsLabel
            Layout.leftMargin: Kirigami.Units.smallSpacing
            visible: text !== ""
            color: Kirigami.Theme.highlightColor
            font.bold: true
            Timer {
                interval: 1000
                running: page.visible && page.moving.length > 0
                repeat: true
                triggeredOnStart: true
                onTriggered: feelsLabel.text = page.feels()
            }
        }

        // Each transfer's files
        Repeater {
            model: page.moving
            ColumnLayout {
                id: group
                required property var modelData
                readonly property var m: modelData
                readonly property int done: page.doneFiles(m)
                readonly property int firstShown: Math.max(0, Math.min(Math.max(0, done) - 2, m.files.length - 30))
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.largeSpacing
                spacing: 0
                RowLayout {
                    Layout.fillWidth: true
                    QQC2.Label {
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                        opacity: 0.6
                        font.bold: true
                        font.letterSpacing: 1.3
                        font.pixelSize: 11
                        text: [group.m.incoming ? page.app.t("From {name}", { name: group.m.peer_name }) : page.app.t("To {name}", { name: group.m.peer_name }),
                               group.done >= 0 ? page.app.t("{done} of {total} files", { done: group.done, total: group.m.files.length })
                                               : page.plural(group.m.files.length, group.m.imported ? "photo" : "item")].join(" · ").toUpperCase()
                    }
                    QQC2.ToolButton {
                        text: page.app.t("Cancel")
                        onClicked: page.app.request("cancel", { transfer: group.m.id })
                    }
                }
                Repeater {
                    model: group.m.files.slice(group.firstShown, group.firstShown + 30)
                    RowLayout {
                        id: fileRow
                        required property var modelData
                        required property int index
                        readonly property int i: group.firstShown + index
                        readonly property bool going: group.done === i
                        readonly property bool finished: group.done > i
                        readonly property real progress: page.fileFraction(group.m, i)
                        Layout.fillWidth: true
                        Layout.margins: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.largeSpacing
                        FileRing {
                            icon: page.fileIcon(fileRow.modelData.name)
                            going: fileRow.going
                            progress: fileRow.progress
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: fileRow.modelData.name.split("/").pop()
                                font.bold: fileRow.going
                                elide: Text.ElideMiddle
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                visible: fileRow.modelData.size !== null
                                opacity: 0.6
                                font: Kirigami.Theme.smallFont
                                textFormat: Text.StyledText
                                text: fileRow.going && fileRow.progress >= 0
                                    ? page.app.t("{done} of {total}", { done: page.app.size(fileRow.modelData.size * fileRow.progress), total: page.app.size(fileRow.modelData.size) }) + " · <b>" + Math.round(fileRow.progress * 100) + "%</b>"
                                    : page.app.size(fileRow.modelData.size ?? 0)
                            }
                        }
                        Kirigami.Icon {
                            visible: fileRow.finished
                            implicitWidth: Kirigami.Units.iconSizes.small
                            implicitHeight: implicitWidth
                            source: "checkmark"
                            opacity: 0.5
                        }
                    }
                }
                QQC2.Label {
                    readonly property int more: group.m.files.length - group.firstShown - Math.min(30, group.m.files.length - group.firstShown)
                    visible: more > 0
                    Layout.leftMargin: Kirigami.Units.gridUnit * 3
                    text: page.app.t("and {count} more", { count: more })
                    opacity: 0.6
                    font: Kirigami.Theme.smallFont
                }
            }
        }

        // What ended lately, with every device
        QQC2.Label {
            visible: page.app.recent.length > 0
            Layout.topMargin: Kirigami.Units.gridUnit
            opacity: 0.6
            font.bold: true
            font.letterSpacing: 1.3
            font.pixelSize: 11
            text: page.app.t("Recent").toUpperCase()
        }
        Repeater {
            model: page.app.recent.slice(0, 30)
            ColumnLayout {
                id: recentItem
                required property var modelData
                readonly property var e: modelData
                readonly property bool incoming: e.direction === "incoming"
                readonly property int count: e.what.files.length + (e.what.more ?? 0)
                readonly property bool group: count > 1
                readonly property bool ended: e.status.state === "done"
                readonly property var saved: e.what.saved ?? []
                property bool open: false
                Layout.fillWidth: true
                spacing: 0
                QQC2.ItemDelegate {
                    Layout.fillWidth: true
                    onClicked: {
                        if (recentItem.group && recentItem.ended) recentItem.open = !recentItem.open;
                        else if (recentItem.incoming && recentItem.ended && recentItem.saved.length > 0) Qt.openUrlExternally("file://" + recentItem.saved[0]);
                    }
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.largeSpacing
                        FileRing {
                            icon: recentItem.group ? (recentItem.incoming ? "folder-download" : "document-send") : page.fileIcon(recentItem.e.what.files[0])
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            QQC2.Label {
                                Layout.fillWidth: true
                                elide: Text.ElideMiddle
                                text: recentItem.e.what["import"] === true ? page.plural(recentItem.count, "photo")
                                    : recentItem.group ? page.plural(recentItem.count, "item") : recentItem.e.what.files[0].split("/").pop()
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                                opacity: 0.6
                                font: Kirigami.Theme.smallFont
                                text: {
                                    const when = new Date(recentItem.e.at);
                                    const today = when.toDateString() === new Date().toDateString();
                                    return [recentItem.incoming ? page.app.t("From {name}", { name: recentItem.e.peer_name }) : page.app.t("To {name}", { name: recentItem.e.peer_name }),
                                            page.app.size(recentItem.e.what.total),
                                            today ? when.toLocaleTimeString(Qt.locale(), "HH:mm") : when.toLocaleDateString(Qt.locale(), "dddd d MMMM")].join(" · ");
                                }
                            }
                        }
                        QQC2.Label {
                            visible: !recentItem.ended
                            color: Kirigami.Theme.negativeTextColor
                            font: Kirigami.Theme.smallFont
                            text: ({ declined: page.app.t("Declined"), cancelled: page.app.t("Cancelled") })[recentItem.e.status.state] ?? page.app.t("Failed")
                        }
                        Kirigami.Icon {
                            visible: recentItem.group && recentItem.ended
                            implicitWidth: Kirigami.Units.iconSizes.small
                            implicitHeight: implicitWidth
                            source: recentItem.open ? "arrow-up" : "arrow-down"
                            opacity: 0.6
                        }
                    }
                }
                Repeater {
                    model: recentItem.open ? recentItem.e.what.files : []
                    QQC2.ItemDelegate {
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.gridUnit * 3
                        enabled: index < recentItem.saved.length
                        icon.name: page.fileIcon(modelData)
                        text: modelData.split("/").pop()
                        onClicked: Qt.openUrlExternally("file://" + recentItem.saved[index])
                    }
                }
                QQC2.Label {
                    visible: recentItem.open && (recentItem.e.what.more ?? 0) > 0
                    Layout.leftMargin: Kirigami.Units.gridUnit * 3
                    text: page.app.t("and {count} more", { count: recentItem.e.what.more ?? 0 })
                    opacity: 0.6
                    font: Kirigami.Theme.smallFont
                }
            }
        }
    }

    readonly property string kindIcon: ({ phone: "smartphone", tablet: "tablet", laptop: "computer-laptop", desktop: "computer" })[device?.device.kind ?? "phone"] ?? "smartphone"

    component LaneEnd: Rectangle {
        property string icon
        property color tint
        implicitWidth: 32
        implicitHeight: 32
        radius: 10
        color: Qt.alpha(tint, 0.15)
        Kirigami.Icon {
            anchors.centerIn: parent
            width: 18
            height: 18
            source: parent.icon
            color: parent.tint
        }
    }

    component FileRing: Item {
        property string icon
        property bool going: false
        property real progress: -1
        implicitWidth: 40
        implicitHeight: 40
        Rectangle {
            anchors.fill: parent
            radius: width / 2
            color: parent.going ? Qt.alpha(Kirigami.Theme.highlightColor, 0.1) : Qt.alpha(Kirigami.Theme.textColor, 0.06)
        }
        Canvas {
            id: ring
            anchors.fill: parent
            visible: parent.going
            readonly property real value: parent.progress
            onValueChanged: requestPaint()
            onVisibleChanged: requestPaint()
            onPaint: {
                const ctx = getContext("2d");
                ctx.reset();
                const r = width / 2 - 1.5;
                ctx.lineWidth = 2.5;
                ctx.strokeStyle = Qt.alpha(Kirigami.Theme.textColor, 0.12);
                ctx.beginPath();
                ctx.arc(width / 2, height / 2, r, 0, Math.PI * 2);
                ctx.stroke();
                if (value < 0) return;
                ctx.lineCap = "round";
                ctx.strokeStyle = Kirigami.Theme.highlightColor;
                ctx.beginPath();
                ctx.arc(width / 2, height / 2, r, -Math.PI / 2, -Math.PI / 2 + value * Math.PI * 2);
                ctx.stroke();
            }
        }
        Kirigami.Icon {
            anchors.centerIn: parent
            width: 18
            height: 18
            source: parent.icon
            color: parent.going ? Kirigami.Theme.highlightColor : Kirigami.Theme.disabledTextColor
        }
    }
}
