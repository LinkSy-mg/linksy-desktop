// A phone's Files tab: where in the folder it shares, then what's in it, folders first. Only the
// rows in view are built (a camera folder holds thousands of photos).
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: tab
    required property var app
    required property var store
    required property string deviceName
    // The phone's name, then each folder down to the one shown
    readonly property var crumbs: {
        const out = [{ name: deviceName, path: "/" }];
        let at = "/";
        for (const part of store.path.split("/").filter(p => p)) {
            at += part + "/";
            out.push({ name: part, path: at });
        }
        return out;
    }

    // The folder the rows are from: another one starts at its top
    property string listed: ""

    spacing: Kirigami.Units.largeSpacing

    Component.onCompleted: show()

    KeyedList { id: shown }

    Connections {
        target: tab.store
        function onEntriesChanged() { tab.show() }
    }

    function show() {
        shown.sync(store.entries, e => (e.dir ? "d/" : "f/") + e.name);
        if (listed !== store.path) {
            listed = store.path;
            list.positionViewAtBeginning();
        }
    }

    function icon(e) {
        if (e.dir) return "folder";
        const ext = e.name.includes(".") ? e.name.split(".").pop().toLowerCase() : "";
        if (["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "dng"].includes(ext)) return "image-x-generic";
        if (["mp4", "mkv", "webm", "mov", "3gp"].includes(ext)) return "video-x-generic";
        if (["mp3", "m4a", "ogg", "opus", "flac", "wav", "aac"].includes(ext)) return "audio-x-generic";
        return "text-x-generic";
    }

    // "3.8 MB · Yesterday"
    function detail(e) {
        return [e.dir ? "" : app.size(e.size), e.modified_ms > 0 ? app.dayLabel(e.modified_ms) : ""].filter(x => x).join(" · ");
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: Kirigami.Units.largeSpacing

        Flickable {
            Layout.fillWidth: true
            implicitHeight: crumbRow.implicitHeight
            contentWidth: crumbRow.implicitWidth
            contentHeight: height
            flickableDirection: Flickable.HorizontalFlick
            boundsBehavior: Flickable.StopAtBounds
            interactive: contentWidth > width
            clip: true

            Row {
                id: crumbRow
                Repeater {
                    model: tab.crumbs
                    delegate: Row {
                        id: crumb
                        required property var modelData
                        required property int index
                        readonly property bool last: index === tab.crumbs.length - 1
                        Kirigami.Icon {
                            visible: crumb.index > 0
                            anchors.verticalCenter: parent.verticalCenter
                            width: Kirigami.Units.iconSizes.small * 0.75
                            height: width
                            source: "go-next"
                            opacity: 0.6
                        }
                        QQC2.ToolButton {
                            id: crumbButton
                            text: crumb.modelData.name
                            onClicked: tab.store.list(crumb.modelData.path)
                            contentItem: QQC2.Label {
                                text: crumbButton.text
                                textFormat: Text.PlainText
                                font.weight: Font.DemiBold
                                color: crumb.last ? Kirigami.Theme.textColor : Kirigami.Theme.highlightColor
                            }
                        }
                    }
                }
            }
        }

        QQC2.Button {
            icon.name: "system-file-manager"
            text: tab.app.t("Open in Files")
            onClicked: tab.store.openInFiles()
        }
    }

    Rectangle {
        // A view with no height builds no rows to measure: the most at first
        readonly property real most: Kirigami.Units.gridUnit * 29
        Layout.fillWidth: true
        Layout.preferredHeight: (list.contentHeight > 0 ? Math.min(list.contentHeight, most) : most) + 2
        visible: tab.store.status === ""
        radius: Kirigami.Units.cornerRadius * 2
        color: "transparent"
        border.width: 1
        border.color: Qt.alpha(Kirigami.Theme.textColor, 0.1)

        QQC2.ScrollView {
            anchors.fill: parent
            anchors.margins: 1
            ListView {
                id: list
                clip: true
                model: shown.model
                delegate: QQC2.ItemDelegate {
                    id: entry
                    required property string key
                    required property int index
                    readonly property var modelData: shown.rows[key]
                    width: list.width
                    text: modelData.name
                    onClicked: {
                        const at = tab.store.path + modelData.name;
                        if (modelData.dir) tab.store.list(at + "/");
                        else tab.store.launch(at);
                    }

                    contentItem: RowLayout {
                        spacing: Kirigami.Units.largeSpacing * 1.5
                        Kirigami.Icon {
                            implicitWidth: Kirigami.Units.iconSizes.smallMedium
                            implicitHeight: Kirigami.Units.iconSizes.smallMedium
                            source: tab.icon(entry.modelData)
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: entry.modelData.name
                                textFormat: Text.PlainText
                                elide: Text.ElideMiddle
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                visible: text !== ""
                                text: tab.detail(entry.modelData)
                                font: Kirigami.Theme.smallFont
                                opacity: 0.7
                            }
                        }
                        Kirigami.Icon {
                            visible: entry.modelData.dir
                            implicitWidth: Kirigami.Units.iconSizes.small
                            implicitHeight: Kirigami.Units.iconSizes.small
                            source: "go-next"
                            opacity: 0.6
                        }
                    }

                    Rectangle {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        height: 1
                        visible: entry.index < list.count - 1
                        color: Qt.alpha(Kirigami.Theme.textColor, 0.07)
                    }
                }
            }
        }
    }

    QQC2.Label {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 1.5
        Layout.bottomMargin: Kirigami.Units.gridUnit * 1.5
        visible: tab.store.status !== ""
        text: tab.store.status
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        opacity: 0.7
    }
}
