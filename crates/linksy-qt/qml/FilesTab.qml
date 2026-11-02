// A phone's Files tab: a file manager for the folder it shares, one folder at a time. Only the
// rows in view are built (a camera folder holds thousands of photos). A click picks a row
// (Ctrl and Shift add to it, as in Files); a double click or Enter opens it.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: tab
    required property var app
    required property var store
    required property string deviceName
    // Icons only on a narrow page
    required property bool narrow
    // The page's own scrolling, which the rows grow into, and how much the pinned tabs cover
    required property Flickable view
    required property real covered
    // The folder the rows are from: another one starts at its top
    property string listed: ""
    // What the folder holds in the order and search chosen; the first `limit` are rows
    property var sorted: []
    property int limit: 200
    // Search: this folder, by name, as it's typed
    property bool searching: false
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
    onSearchingChanged: {
        if (searching) return searchField.forceActiveFocus();
        searchField.text = "";
        query = "";
    }

    KeyedList { id: shown }

    Connections {
        target: tab.store
        function onListed(moved) { tab.landed(moved) }
        function onSortByChanged() { tab.show(true) }
        function onDescendingChanged() { tab.show(true) }
    }
    // 200 more rows each time the page reaches its end
    Connections {
        target: tab.view
        function onContentYChanged() { if (tab.view.contentY + tab.view.height > tab.view.contentHeight - 600) tab.more() }
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

    // The rows: in place as the folder changes, or all anew (the first 200) for another order,
    // search or folder
    function show(fresh) {
        const list = store.entries.filter(e => !query || e.name.toLowerCase().includes(query));
        list.sort((a, b) => tab.compare(a, b));
        sorted = list;
        if (fresh) limit = 200;
        refill(fresh);
        if (listed !== store.path) {
            listed = store.path;
            reveal(tab, 0);
        }
    }

    function refill(fresh) {
        if (fresh) shown.replace(sorted.slice(0, limit), e => tab.keyOf(e));
        else shown.sync(sorted.slice(0, limit), e => tab.keyOf(e));
    }

    function more() {
        if (limit >= sorted.length) return;
        limit += 200;
        refill(false);
    }

    // Rows down to this one, at least
    function showUpTo(index) {
        if (index < limit) return;
        limit = Math.ceil((index + 1) / 200) * 200;
        refill(false);
    }

    // The page scrolled so `height` of `item` shows, under the pinned tabs
    function reveal(item, height) {
        rowsColumn.forceLayout();
        const top = item.mapToItem(view.contentItem, 0, 0).y - covered;
        if (top < view.contentY) view.contentY = Math.max(0, top);
        else if (top + covered + height > view.contentY + view.height) view.contentY = Math.min(top + covered + height - view.height, view.contentHeight - view.height);
    }

    function revealRow(index) {
        const row = rowRepeater.itemAt(index);
        if (row) reveal(row, row.height);
    }

    // The folder was read: nothing picked but what was asked for
    function landed(moved) {
        clearPicks();
        show(moved);
        if (store.pick) pickName(store.pick);
    }

    function entryAt(index) {
        return sorted[index];
    }

    function setPicks(next, index) {
        picked = next;
        pickedCount = Object.keys(next).length;
        folderView.currentIndex = index;
    }

    function pickOnly(index) {
        anchor = index;
        setPicks({ [keyOf(sorted[index])]: true }, index);
    }

    function togglePick(index) {
        const key = keyOf(sorted[index]), next = Object.assign({}, picked);
        if (next[key]) delete next[key];
        else next[key] = true;
        anchor = index;
        setPicks(next, index);
    }

    // From where the last pick was to here, with what's picked already or instead of it
    function pickRange(index, add) {
        const next = add ? Object.assign({}, picked) : {}, from = anchor < 0 ? index : anchor;
        for (let i = Math.min(from, index); i <= Math.max(from, index); i++) next[keyOf(sorted[i])] = true;
        setPicks(next, index);
    }

    // All of the folder's, rows yet to show too
    function pickAll() {
        const next = {};
        for (const e of sorted) next[keyOf(e)] = true;
        setPicks(next, folderView.currentIndex);
    }

    function clearPicks() {
        picked = {};
        pickedCount = 0;
        anchor = -1;
    }

    // A new or renamed one, shown even past the rows there were
    function pickName(name) {
        const at = sorted.findIndex(e => e.name === name);
        if (at < 0) return;
        showUpTo(at);
        pickOnly(at);
        revealRow(at);
    }

    // What's picked, in the order shown
    function pickedEntries() {
        return sorted.filter(e => picked[tab.keyOf(e)]);
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

    // Go to a folder: a fresh look, nothing picked, no search
    function openFolder(path) {
        searching = false;
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

    function localPath(url) {
        return decodeURIComponent(url.toString().replace(/^file:\/\//, ""));
    }

    // As in Files: Delete, F2 to rename, Escape to let go, Ctrl+F to search, Ctrl+A, the
    // arrows, Enter to open
    function key(event) {
        const ctrl = event.modifiers & Qt.ControlModifier, count = sorted.length;
        if (event.key === Qt.Key_F && ctrl) searching = true;
        else if (event.key === Qt.Key_Delete && pickedCount > 0) askDelete();
        else if (event.key === Qt.Key_F2 && pickedCount === 1) askRename();
        else if (event.key === Qt.Key_Escape && pickedCount > 0) clearPicks();
        else if (event.key === Qt.Key_A && ctrl) pickAll();
        else if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && folderView.currentIndex >= 0) activate(entryAt(folderView.currentIndex));
        else if ((event.key === Qt.Key_Down || event.key === Qt.Key_Up) && count > 0) {
            const next = Math.max(0, Math.min(count - 1, folderView.currentIndex + (event.key === Qt.Key_Down ? 1 : -1)));
            showUpTo(next);
            if (event.modifiers & Qt.ShiftModifier) pickRange(next, false);
            else pickOnly(next);
            revealRow(next);
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

        ToolIcon {
            icon.name: "system-search"
            text: tab.app.t("Search")
            checkable: true
            checked: tab.searching
            onToggled: {
                tab.searching = checked;
                checked = Qt.binding(() => tab.searching);
            }
        }
        ToolIcon {
            id: sortButton
            icon.name: "view-sort-descending"
            text: tab.app.t("Sort")
            down: sortMenu.visible
            onClicked: sortMenu.popup(sortButton, 0, sortButton.height)
        }
        ToolIcon {
            icon.name: "folder-new"
            text: tab.app.t("New Folder…")
            onClicked: nameDialog.ask(false, "", 0)
        }
        ToolIcon {
            id: uploadButton
            icon.name: "document-send"
            text: tab.app.t("Upload")
            down: uploadMenu.visible
            onClicked: uploadMenu.popup(uploadButton, 0, uploadButton.height)
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

    Kirigami.SearchField {
        id: searchField
        Layout.fillWidth: true
        visible: tab.searching
        placeholderText: tab.app.t("Search This Folder")
        onTextChanged: tab.query = text.trim().toLowerCase()
        Keys.onEscapePressed: {
            tab.searching = false;
            folderView.forceActiveFocus();
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

    // The list, or why there's nothing in it. Its rows grow the page, which scrolls them.
    Item {
        Layout.fillWidth: true
        implicitHeight: tab.status === "" ? folderView.height : statusBox.implicitHeight + Kirigami.Units.gridUnit * 3

        Item {
            id: folderView
            // The row the keys move from
            property int currentIndex: -1
            width: parent.width
            height: rowsColumn.implicitHeight
            visible: tab.status === ""
            activeFocusOnTab: true
            Keys.onPressed: event => tab.key(event)

            Column {
                id: rowsColumn
                width: parent.width
                Repeater {
                    id: rowRepeater
                    model: shown.model
                    delegate: EntryRow {}
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

        // Files dropped here go into the folder shown
        DropArea {
            id: drop
            anchors.fill: parent
            onDropped: event => {
                if (!event.hasUrls) return;
                tab.store.upload(event.urls.map(u => tab.localPath(u)));
                event.acceptProposedAction();
            }
        }
        Rectangle {
            anchors.fill: parent
            visible: drop.containsDrag
            radius: Kirigami.Units.cornerRadius * 2
            color: Qt.alpha(Kirigami.Theme.highlightColor, 0.08)
            border.width: 2
            border.color: Kirigami.Theme.highlightColor
        }
    }

    // Copies to and from this phone going on, each with Cancel
    Repeater {
        model: tab.store.ops
        delegate: RowLayout {
            id: copying
            required property string op
            required property string phone
            required property string name
            required property real done
            required property real total
            required property bool upload
            property bool stopping: false
            Layout.fillWidth: true
            visible: phone === tab.store.phone
            spacing: Kirigami.Units.largeSpacing
            Kirigami.Icon {
                Layout.alignment: Qt.AlignTop
                implicitWidth: Kirigami.Units.iconSizes.smallMedium
                implicitHeight: implicitWidth
                source: copying.upload ? "document-send" : "folder-download"
                opacity: 0.7
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                QQC2.Label {
                    Layout.fillWidth: true
                    text: copying.upload ? tab.app.t("Uploading {name}", { name: copying.name }) : tab.app.t("Copying {name}", { name: copying.name })
                    textFormat: Text.PlainText
                    elide: Text.ElideMiddle
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    text: tab.app.t("{done} of {total}", { done: tab.app.size(copying.done), total: tab.app.size(copying.total) })
                          + (copying.total > 0 ? ` · ${Math.floor(copying.done / copying.total * 100)}%` : "")
                    font: Kirigami.Theme.smallFont
                    opacity: 0.7
                }
                QQC2.ProgressBar {
                    Layout.fillWidth: true
                    from: 0
                    to: Math.max(1, copying.total)
                    value: copying.done
                    indeterminate: copying.total === 0
                }
            }
            QQC2.Button {
                flat: true
                enabled: !copying.stopping
                text: tab.app.t("Cancel")
                onClicked: {
                    copying.stopping = true;
                    tab.store.cancel(copying.op);
                }
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

    // Name, date or size, either way; folders first whichever
    QQC2.ButtonGroup { id: sortGroup }
    QQC2.ButtonGroup { id: orderGroup }
    QQC2.Menu {
        id: sortMenu
        MenuHeading { text: tab.app.t("Sort") }
        SortItem { value: "name"; text: tab.app.t("Name") }
        SortItem { value: "date"; text: tab.app.t("Date") }
        SortItem { value: "size"; text: tab.app.t("Size") }
        QQC2.MenuSeparator {}
        OrderItem { descending: false; text: tab.app.t("Ascending") }
        OrderItem { descending: true; text: tab.app.t("Descending") }
    }

    QQC2.Menu {
        id: uploadMenu
        QQC2.MenuItem {
            text: tab.app.t("Upload Files…")
            onTriggered: uploadFiles.open()
        }
        QQC2.MenuItem {
            text: tab.app.t("Upload Folders…")
            onTriggered: uploadFolder.open()
        }
    }

    FileDialog {
        id: uploadFiles
        title: tab.app.t("Upload to {name}", { name: tab.deviceName })
        acceptLabel: tab.app.t("Upload")
        fileMode: FileDialog.OpenFiles
        onAccepted: tab.store.upload(selectedFiles.map(u => tab.localPath(u)))
    }

    FolderDialog {
        id: uploadFolder
        title: tab.app.t("Upload to {name}", { name: tab.deviceName })
        acceptLabel: tab.app.t("Upload")
        onAccepted: tab.store.upload([tab.localPath(selectedFolder)])
    }

    component ToolIcon: QQC2.ToolButton {
        id: toolIcon
        display: QQC2.AbstractButton.IconOnly
        QQC2.ToolTip.text: toolIcon.text
        QQC2.ToolTip.visible: toolIcon.hovered
    }

    component MenuHeading: QQC2.Label {
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.largeSpacing
        topPadding: Kirigami.Units.smallSpacing
        bottomPadding: Kirigami.Units.smallSpacing
        font.pointSize: Kirigami.Theme.smallFont.pointSize
        font.bold: true
        opacity: 0.7
    }

    component SortItem: QQC2.MenuItem {
        id: sortItem
        required property string value
        checkable: true
        checked: tab.store.sortBy === sortItem.value
        QQC2.ButtonGroup.group: sortGroup
        onTriggered: tab.store.sortBy = sortItem.value
    }

    component OrderItem: QQC2.MenuItem {
        id: orderItem
        required property bool descending
        checkable: true
        checked: tab.store.descending === orderItem.descending
        QQC2.ButtonGroup.group: orderGroup
        onTriggered: tab.store.descending = orderItem.descending
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
            visible: entry.index === folderView.currentIndex && folderView.activeFocus
            color: "transparent"
            border.width: 1
            border.color: Kirigami.Theme.focusColor
        }
        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            visible: entry.index < shown.model.count - 1
            color: Qt.alpha(Kirigami.Theme.textColor, 0.07)
        }
    }
}
