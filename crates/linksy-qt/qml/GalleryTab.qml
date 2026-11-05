// A phone's Gallery tab: its photos and videos in a grid under a heading for each day,
// newest first, the next page as it scrolls. Only the rows in view are built, and each
// square asks for its own thumbnail.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: tab
    required property var app
    required property var store

    spacing: Kirigami.Units.largeSpacing

    Component.onCompleted: albumPicker.currentIndex = albumIndex()

    Connections {
        target: tab.store
        function onAlbumsChanged() { Qt.callLater(() => albumPicker.currentIndex = tab.albumIndex()) }
        function onPageLanded() { Qt.callLater(tab.fill) }
    }

    function albumIndex() {
        return store.album ? store.albums.findIndex(a => a.id === store.album) + 1 : 0;
    }

    // Short pages leave nothing to scroll: the next one now
    function fill() {
        if (list.contentHeight < list.height + 400) store.page(false);
    }

    function click(index, modifiers) {
        if (modifiers & Qt.ControlModifier) store.toggle(index);
        else if (modifiers & Qt.ShiftModifier) store.selectRange(index);
        else store.selectOnly(index);
    }

    // "0:14", "12:03", "1:02:03"
    function clock(ms) {
        const s = Math.floor(ms / 1000), two = n => String(n).padStart(2, "0");
        return s >= 3600 ? `${Math.floor(s / 3600)}:${two(Math.floor(s / 60) % 60)}:${two(s % 60)}` : `${Math.floor(s / 60)}:${two(s % 60)}`;
    }

    RowLayout {
        Layout.fillWidth: true
        QQC2.ComboBox {
            id: albumPicker
            model: [tab.app.t("All Photos")].concat(tab.store.albums.map(a => `${a.name} (${a.count})`))
            onActivated: index => {
                tab.store.album = index > 0 ? tab.store.albums[index - 1].id : "";
                tab.store.page(true);
            }
        }
        Item { Layout.fillWidth: true }
        QQC2.Label {
            visible: tab.store.albums.length > 0
            text: tab.app.plural(tab.store.total, "item")
            opacity: 0.7
        }
    }

    QQC2.Label {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 2
        Layout.bottomMargin: Kirigami.Units.gridUnit * 2
        visible: tab.store.status !== ""
        text: tab.store.status
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        opacity: 0.7
    }

    QQC2.ScrollView {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 33
        visible: tab.store.status === ""
        ListView {
            id: list
            readonly property int columns: Math.max(3, Math.min(12, Math.floor(width / (Kirigami.Units.gridUnit * 7.5))))
            readonly property int square: Math.floor(width / columns)
            // Decoded no bigger than shown, nor than the service's thumbnails
            readonly property int decoded: Math.min(256, Math.ceil(square * Screen.devicePixelRatio))
            clip: true
            model: tab.store.rows
            onColumnsChanged: tab.store.setColumns(columns)
            Component.onCompleted: tab.store.setColumns(columns)
            onContentYChanged: if (contentY - originY + height > contentHeight - 400) tab.store.page(false)
            delegate: Item {
                id: line
                required property string kind
                required property string label
                required property int start
                required property int size
                width: ListView.view.width
                height: kind === "day" ? heading.implicitHeight + Kirigami.Units.gridUnit : list.square
                QQC2.Label {
                    id: heading
                    visible: line.kind === "day"
                    anchors.left: parent.left
                    anchors.bottom: parent.bottom
                    anchors.leftMargin: Kirigami.Units.smallSpacing
                    anchors.bottomMargin: Kirigami.Units.smallSpacing
                    text: line.label
                    font.weight: Font.DemiBold
                }
                Row {
                    visible: line.kind === "photos"
                    Repeater {
                        model: line.kind === "photos" ? line.size : 0
                        Cell { first: line.start }
                    }
                }
            }
        }
    }

    // What's selected, to save into the photos folder
    Rectangle {
        Layout.fillWidth: true
        visible: tab.store.selectedCount > 0
        implicitHeight: bar.implicitHeight + Kirigami.Units.smallSpacing * 2
        radius: height / 2
        color: Kirigami.Theme.backgroundColor
        border.width: 1
        border.color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
        RowLayout {
            id: bar
            anchors.fill: parent
            anchors.margins: Kirigami.Units.smallSpacing
            anchors.leftMargin: Kirigami.Units.largeSpacing * 2
            QQC2.Label {
                Layout.fillWidth: true
                text: tab.app.t("{what} selected", { what: tab.app.plural(tab.store.selectedCount, "item") })
                font.weight: Font.DemiBold
            }
            QQC2.Button {
                icon.name: "document-save"
                text: tab.app.t("Save to Pictures")
                onClicked: {
                    tab.store.save(tab.store.selectedIds(), false);
                    tab.store.clearSelection();
                }
            }
            QQC2.ToolButton {
                icon.name: "window-close"
                text: tab.app.t("Clear the Selection")
                display: QQC2.AbstractButton.IconOnly
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                onClicked: tab.store.clearSelection()
            }
        }
    }

    // One photo or video: its thumbnail once it's here, its length for a video
    component Cell: Item {
        id: cell
        // Its place in its row, and the row's first among the items
        required property int index
        required property int first
        readonly property int at: first + index
        readonly property var photo: tab.store.items.get(at)
        readonly property string itemId: photo.itemId
        readonly property string name: photo.name
        readonly property bool video: photo.video
        readonly property real taken_ms: photo.taken_ms
        readonly property real duration_ms: photo.duration_ms
        readonly property bool selected: tab.store.selectionRev >= 0 && !!tab.store.chosen[itemId]
        property string path: tab.store.thumb(itemId)
        implicitWidth: list.square
        implicitHeight: list.square

        Connections {
            target: tab.store
            function onThumbReady(id, path) { if (id === cell.itemId) cell.path = path }
        }

        Rectangle {
            anchors.fill: parent
            anchors.margins: 3
            radius: Kirigami.Units.cornerRadius * 2
            color: cell.selected ? Qt.alpha(Kirigami.Theme.highlightColor, 0.22) : "transparent"
            border.width: cell.selected ? 3 : 0
            border.color: Kirigami.Theme.highlightColor
            Rectangle {
                anchors.fill: parent
                anchors.margins: cell.selected ? 5 : 0
                radius: Kirigami.Units.cornerRadius
                color: Qt.alpha(Kirigami.Theme.textColor, 0.06)
                Image {
                    anchors.fill: parent
                    source: cell.path ? "file://" + cell.path.split("/").map(encodeURIComponent).join("/") : ""
                    sourceSize.width: list.decoded
                    sourceSize.height: list.decoded
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                }
                Rectangle {
                    visible: cell.video && cell.duration_ms >= 0
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: Kirigami.Units.smallSpacing
                    implicitWidth: length.implicitWidth + Kirigami.Units.smallSpacing * 2
                    implicitHeight: length.implicitHeight
                    radius: Kirigami.Units.cornerRadius
                    color: Qt.rgba(0, 0, 0, 0.62)
                    QQC2.Label {
                        id: length
                        anchors.centerIn: parent
                        text: tab.clock(cell.duration_ms)
                        color: "white"
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        font.weight: Font.DemiBold
                    }
                }
            }
        }

        MouseArea {
            id: pointer
            anchors.fill: parent
            hoverEnabled: true
            onClicked: mouse => tab.click(cell.at, mouse.modifiers)
            onDoubleClicked: tab.store.save([cell.itemId], true)
            onContainsMouseChanged: {
                if (containsMouse) {
                    tip.text = `${cell.name}\n${tab.app.dayLabel(cell.taken_ms)}`;
                    tip.cell = cell;
                } else if (tip.cell === cell) {
                    tip.cell = null;
                }
            }
        }
    }

    // One for the grid; names show as they are, never as markup
    QQC2.ToolTip {
        id: tip
        property Item cell: null
        parent: cell ?? tab
        visible: cell !== null
        delay: Kirigami.Units.toolTipDelay
        contentItem: QQC2.Label {
            text: tip.text
            textFormat: Text.PlainText
        }
    }
}
