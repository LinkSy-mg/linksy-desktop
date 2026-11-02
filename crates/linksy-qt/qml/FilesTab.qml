// A phone's Files tab: a file manager for the folder it shares, one folder at a time. Only the
// rows in view are built (a camera folder holds thousands of photos). A click picks a row
// (Ctrl and Shift add to it, as in Files); a double click or Enter opens it.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: tab
    required property var app
    required property var store
    required property string deviceName
    // Icons only on a narrow page
    required property bool narrow
    // The folder the rows are from: another one starts at its top
    property string listed: ""
    property string query: ""
    // Picked rows by key, how many, and where a Shift range starts
    property var picked: ({})
    property int pickedCount: 0
    property int anchor: -1
    // What shows instead of the list, "" for the list
    readonly property string status: store.waiting ? app.t("Loading…") : store.error ? store.error
        : store.entries.length === 0 && !store.loading ? app.t("This folder is empty.")
        : shown.model.count === 0 && query ? app.t("Nothing here matches “{query}”.", { query: query }) : ""
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

    spacing: Kirigami.Units.largeSpacing

    Component.onCompleted: show(true)
    onQueryChanged: {
        clearPicks();
        show(true);
    }

    KeyedList { id: shown }

    Connections {
        target: tab.store
        function onListed(moved) { tab.landed(moved) }
        function onSortByChanged() { tab.show(true) }
        function onDescendingChanged() { tab.show(true) }
    }

    function keyOf(e) {
        return (e.dir ? "d/" : "f/") + e.name;
    }

    // Folders first, whichever the order; then by name, date or size
    function compare(a, b) {
        if (a.dir !== b.dir) return a.dir ? -1 : 1;
        let order = store.sortBy === "date" ? a.modified_ms - b.modified_ms : store.sortBy === "size" ? a.size - b.size : 0;
        if (order === 0) order = a.name.localeCompare(b.name);
        return store.descending ? -order : order;
    }

    // The rows: in place as the folder changes, or all anew for another order or search
    function show(fresh) {
        const list = store.entries.filter(e => !query || e.name.toLowerCase().includes(query));
        list.sort((a, b) => tab.compare(a, b));
        if (fresh) shown.replace(list, e => tab.keyOf(e));
        else shown.sync(list, e => tab.keyOf(e));
        if (listed !== store.path) {
            listed = store.path;
            folderView.positionViewAtBeginning();
        }
    }

    // The folder was read: nothing picked but what was asked for
    function landed(moved) {
        clearPicks();
        show(moved);
        if (store.pick) pickName(store.pick);
    }

    function entryAt(index) {
        return shown.rows[shown.model.get(index).key];
    }

    function setPicks(next, index) {
        picked = next;
        pickedCount = Object.keys(next).length;
        folderView.currentIndex = index;
    }

    function pickOnly(index) {
        anchor = index;
        setPicks({ [shown.model.get(index).key]: true }, index);
    }

    function togglePick(index) {
        const key = shown.model.get(index).key, next = Object.assign({}, picked);
        if (next[key]) delete next[key];
        else next[key] = true;
        anchor = index;
        setPicks(next, index);
    }

    // From where the last pick was to here, with what's picked already or instead of it
    function pickRange(index, add) {
        const next = add ? Object.assign({}, picked) : {}, from = anchor < 0 ? index : anchor;
        for (let i = Math.min(from, index); i <= Math.max(from, index); i++) next[shown.model.get(i).key] = true;
        setPicks(next, index);
    }

    function pickAll() {
        const next = {};
        for (let i = 0; i < shown.model.count; i++) next[shown.model.get(i).key] = true;
        setPicks(next, folderView.currentIndex);
    }

    function clearPicks() {
        picked = {};
        pickedCount = 0;
        anchor = -1;
    }

    function pickName(name) {
        for (let i = 0; i < shown.model.count; i++) {
            if (entryAt(i).name !== name) continue;
            pickOnly(i);
            folderView.positionViewAtIndex(i, ListView.Contain);
            return;
        }
    }

    // What's picked, in the order shown
    function pickedEntries() {
        const out = [];
        for (let i = 0; i < shown.model.count; i++) {
            if (picked[shown.model.get(i).key]) out.push(entryAt(i));
        }
        return out;
    }

    function click(index, modifiers) {
        folderView.forceActiveFocus();
        if (modifiers & Qt.ShiftModifier) pickRange(index, modifiers & Qt.ControlModifier);
        else if (modifiers & Qt.ControlModifier) togglePick(index);
        else pickOnly(index);
    }

    // A folder opens here, a file through the phone's bridge
    function activate(e) {
        if (e.dir) openFolder(store.path + e.name + "/");
        else store.launch(store.path + e.name);
    }

    // Go to a folder: a fresh look, nothing picked
    function openFolder(path) {
        clearPicks();
        store.list(path);
    }

    function taken(name) {
        const lower = name.toLowerCase();
        return store.entries.some(e => e.name.toLowerCase() === lower);
    }

    function askRename() {
        const one = pickedEntries();
        if (one.length !== 1) return;
        const dot = one[0].name.lastIndexOf(".");
        nameDialog.ask(true, one[0].name, !one[0].dir && dot > 0 ? dot : one[0].name.length);
    }

    function askDelete() {
        const names = pickedEntries().map(e => e.name);
        if (names.length) deleteDialog.ask(names);
    }

    function copyPicked() {
        store.copy(pickedEntries().map(e => e.name));
    }

    // As in Files: Delete, F2 to rename, Escape to let go, Ctrl+A, the arrows, Enter to open
    function key(event) {
        const ctrl = event.modifiers & Qt.ControlModifier, count = shown.model.count;
        if (event.key === Qt.Key_Delete && pickedCount > 0) askDelete();
        else if (event.key === Qt.Key_F2 && pickedCount === 1) askRename();
        else if (event.key === Qt.Key_Escape && pickedCount > 0) clearPicks();
        else if (event.key === Qt.Key_A && ctrl) pickAll();
        else if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && folderView.currentIndex >= 0) activate(entryAt(folderView.currentIndex));
        else if ((event.key === Qt.Key_Down || event.key === Qt.Key_Up) && count > 0) {
            const next = Math.max(0, Math.min(count - 1, folderView.currentIndex + (event.key === Qt.Key_Down ? 1 : -1)));
            if (event.modifiers & Qt.ShiftModifier) pickRange(next, false);
            else pickOnly(next);
            folderView.positionViewAtIndex(next, ListView.Contain);
        } else {
            event.accepted = false;
            return;
        }
        event.accepted = true;
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
        spacing: 2

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
                            onClicked: tab.openFolder(crumb.modelData.path)
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
            Layout.leftMargin: Kirigami.Units.smallSpacing
            icon.name: "folder-open"
            text: tab.app.t("Open in Files")
            display: tab.narrow ? QQC2.AbstractButton.IconOnly : QQC2.AbstractButton.TextOnly
            QQC2.ToolTip.text: text
            QQC2.ToolTip.visible: hovered && tab.narrow
            onClicked: tab.store.openInFiles()
        }
    }

    // What's picked, and what can be done with it
    Rectangle {
        Layout.fillWidth: true
        visible: tab.pickedCount > 0
        implicitHeight: pickedRow.implicitHeight + Kirigami.Units.smallSpacing * 2
        radius: height / 2
        color: Qt.alpha(Kirigami.Theme.highlightColor, 0.1)
        RowLayout {
            id: pickedRow
            anchors.fill: parent
            anchors.margins: Kirigami.Units.smallSpacing
            anchors.rightMargin: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.smallSpacing
            QQC2.ToolButton {
                icon.name: "window-close"
                text: tab.app.t("Clear Selection")
                display: QQC2.AbstractButton.IconOnly
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                onClicked: tab.clearPicks()
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: tab.app.t("{n} selected", { n: tab.pickedCount })
                font.bold: true
                elide: Text.ElideRight
            }
            PickedAction {
                icon.name: "folder-download"
                text: tab.app.t("Copy to Computer")
                onClicked: tab.copyPicked()
            }
            PickedAction {
                icon.name: "document-edit"
                text: tab.app.t("Rename…")
                enabled: tab.pickedCount === 1
                onClicked: tab.askRename()
            }
            PickedAction {
                icon.name: "edit-delete"
                text: tab.app.t("Delete…")
                onClicked: tab.askDelete()
            }
        }
    }

    // The list, or why there's nothing in it
    Item {
        Layout.fillWidth: true
        implicitHeight: tab.status === "" ? frame.height : statusBox.implicitHeight + Kirigami.Units.gridUnit * 3

        Rectangle {
            id: frame
            // A view with no height builds no rows to measure: the most at first
            readonly property real most: Kirigami.Units.gridUnit * 29
            width: parent.width
            height: (folderView.contentHeight > 0 ? Math.min(folderView.contentHeight, most) : most) + 2
            visible: tab.status === ""
            radius: Kirigami.Units.cornerRadius * 2
            color: "transparent"
            border.width: 1
            border.color: Qt.alpha(Kirigami.Theme.textColor, 0.1)

            QQC2.ScrollView {
                anchors.fill: parent
                anchors.margins: 1
                ListView {
                    id: folderView
                    clip: true
                    model: shown.model
                    keyNavigationEnabled: false
                    delegate: EntryRow {}
                    Keys.onPressed: event => tab.key(event)
                }
            }
        }

        ColumnLayout {
            id: statusBox
            visible: tab.status !== ""
            width: parent.width
            y: Kirigami.Units.gridUnit * 1.5
            QQC2.BusyIndicator {
                Layout.alignment: Qt.AlignHCenter
                visible: tab.store.waiting
                running: visible
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: tab.status
                textFormat: Text.PlainText
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                opacity: 0.7
            }
        }
    }

    // A name for something in the folder shown: none taken there, no "/"
    Kirigami.PromptDialog {
        id: nameDialog
        property bool renaming: false
        property string initial: ""
        property int stem: 0
        readonly property string typed: nameField.text
        readonly property string problem: typed.includes("/") ? tab.app.t("A name can't have “/” in it.")
            : typed.toLowerCase() !== initial.toLowerCase() && tab.taken(typed) ? tab.app.t("Something with this name is already here.") : ""
        readonly property bool ready: problem === "" && typed.trim() !== "" && typed !== "." && typed !== ".." && typed !== initial
        standardButtons: Kirigami.Dialog.NoButton

        function ask(rename, name, selected) {
            renaming = rename;
            initial = name;
            stem = selected;
            title = rename ? tab.app.t("Rename “{name}”", { name: name }) : tab.app.t("New Folder");
            nameField.text = name;
            open();
        }

        function confirm() {
            if (!ready) return;
            if (renaming) tab.store.rename(initial, typed);
            else tab.store.mkdir(typed);
            close();
        }

        // A file's name is picked up to its extension, ready to type over
        onOpened: {
            nameField.forceActiveFocus();
            nameField.select(0, stem);
        }
        customFooterActions: [
            Kirigami.Action {
                text: nameDialog.renaming ? tab.app.t("Rename") : tab.app.t("Create")
                icon.name: "dialog-ok"
                enabled: nameDialog.ready
                onTriggered: nameDialog.confirm()
            },
            Kirigami.Action {
                text: tab.app.t("Cancel")
                icon.name: "dialog-cancel"
                onTriggered: nameDialog.close()
            }
        ]
        ColumnLayout {
            QQC2.TextField {
                id: nameField
                Layout.fillWidth: true
                Layout.minimumWidth: Kirigami.Units.gridUnit * 18
                onAccepted: nameDialog.confirm()
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: nameDialog.problem !== ""
                text: nameDialog.problem
                wrapMode: Text.Wrap
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.negativeTextColor
            }
        }
    }

    // Deleted on the phone for good: asked first
    Kirigami.PromptDialog {
        id: deleteDialog
        property var names: []
        title: names.length === 1 ? tab.app.t("Delete “{name}”?", { name: names[0] }) : tab.app.t("Delete {n} Items?", { n: names.length })
        subtitle: names.length === 1 ? tab.app.t("It's deleted from {phone} for good.", { phone: tab.deviceName })
                                     : tab.app.t("They're deleted from {phone} for good.", { phone: tab.deviceName })
        standardButtons: Kirigami.Dialog.NoButton

        function ask(list) {
            names = list;
            open();
        }

        customFooterActions: [
            Kirigami.Action {
                text: tab.app.t("Delete")
                icon.name: "edit-delete"
                onTriggered: {
                    tab.store.remove(deleteDialog.names);
                    deleteDialog.close();
                }
            },
            Kirigami.Action {
                text: tab.app.t("Cancel")
                icon.name: "dialog-cancel"
                onTriggered: deleteDialog.close()
            }
        ]
    }

    // On a narrow page only its icon
    component PickedAction: QQC2.Button {
        id: action
        flat: true
        display: tab.narrow ? QQC2.AbstractButton.IconOnly : QQC2.AbstractButton.TextOnly
        QQC2.ToolTip.text: action.text
        QQC2.ToolTip.visible: action.hovered
    }

    component EntryRow: QQC2.ItemDelegate {
        id: entry
        required property string key
        required property int index
        readonly property var modelData: shown.rows[key]
        implicitWidth: folderView.width
        highlighted: tab.picked[key] === true
        text: modelData.name

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

        MouseArea {
            anchors.fill: parent
            onClicked: mouse => tab.click(entry.index, mouse.modifiers)
            onDoubleClicked: tab.activate(entry.modelData)
        }
        // The row the keys move from
        Rectangle {
            anchors.fill: parent
            visible: entry.ListView.isCurrentItem && folderView.activeFocus
            color: "transparent"
            border.width: 1
            border.color: Kirigami.Theme.focusColor
        }
        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            visible: entry.index < folderView.count - 1
            color: Qt.alpha(Kirigami.Theme.textColor, 0.07)
        }
    }
}
