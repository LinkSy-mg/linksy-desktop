// A phone's deck, from this computer's side, shown in place of its page: what the phone may do
// here (a switch each, by group), what its saved deck asked for, and your own commands. The
// phone lays the buttons out and edits them (docs/protocol.md §4.26).
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: page
    required property var app
    required property string phone
    required property string name
    // The actions the computer offers, each with whether it's on for this phone
    property var options: []
    property var commands: []
    // Ids of what the phone's saved tiles run that is off here
    property var asked: []
    // What the computer keeps of the phone's layouts, if anything
    property var layout: null
    // The phone's icon names a command may have
    property var icons: []
    property string error: ""
    property string query: ""
    readonly property string needle: query.trim().toLowerCase()
    // The groups, in the order the service gives them
    readonly property var groups: options.reduce((out, a) => out.includes(a.group) ? out : out.concat([a.group]), [])
    readonly property var askedOptions: asked.map(id => options.find(a => a.id === id)).filter(a => a !== undefined)

    Component.onCompleted: load()

    function capitalize(text) {
        return text.charAt(0).toUpperCase() + text.slice(1);
    }

    function load() {
        app.request("deck_prefs", { phone: phone }, (ok, prefs) => {
            if (!ok) {
                page.error = page.capitalize(prefs);
                return;
            }
            page.options = prefs.actions;
            page.commands = prefs.commands;
            page.asked = prefs.asked ?? [];
            page.layout = prefs.layout ?? null;
            page.icons = prefs.icons ?? [];
        });
    }

    // The service names its groups in English; each shows in the app's language. One of the
    // user's own shows as they typed it.
    function groupTitle(group) {
        return ({ Media: app.t("Media"), Sound: app.t("Sound"), Time: app.t("Time"), System: app.t("System"), Network: app.t("Network"),
                  Power: app.t("Power"), Commands: app.t("Commands") })[group] ?? group;
    }

    // The phone's icon names, as words
    function iconTitle(key) {
        return ({ play_pause: app.t("Play or Pause"), play: app.t("Play"), pause: app.t("Pause"), previous: app.t("Previous"), next: app.t("Next"),
                  speaker: app.t("Speaker"), speaker_off: app.t("Speaker Off"), microphone: app.t("Microphone"), microphone_off: app.t("Microphone Off"),
                  lock: app.t("Lock"), terminal: app.t("Terminal"), lightning: app.t("Lightning"), star: app.t("Star"), moon: app.t("Moon"),
                  sun: app.t("Sun"), house: app.t("House"), bell: app.t("Bell"), code: app.t("Code"), power: app.t("Power"), rocket: app.t("Rocket"),
                  game: app.t("Game"), camera: app.t("Camera"), heart: app.t("Heart"), coffee: app.t("Coffee"), music: app.t("Music"),
                  monitor: app.t("Monitor"), broadcast: app.t("Broadcast"), flag: app.t("Flag"), folder: app.t("Folder"), globe: app.t("Globe"),
                  cloud: app.t("Cloud"), timer: app.t("Timer") })[key] ?? key.replace(/_/g, " ");
    }

    function matches(action) {
        return needle === "" || action.name.toLowerCase().includes(needle) || groupTitle(action.group).toLowerCase().includes(needle);
    }

    // The ones that ask first keep their own switch: a button for all of them could do harm
    function plain(group) {
        return options.filter(a => a.group === group && matches(a) && !a.confirm);
    }

    function shownIn(group) {
        return options.filter(a => a.group === group && matches(a));
    }

    function whenSaved(ms) {
        return new Date(ms).toLocaleString(Qt.locale(), Locale.ShortFormat);
    }

    // All of them go back (a new one without an id); then what's offered, with their actions
    function saveCommands(list, done) {
        error = "";
        app.request("set_deck_commands", { commands: list }, (ok, err) => {
            if (ok) page.load();
            else if (!done) page.error = page.capitalize(err);
            if (done) done(ok, ok ? "" : page.capitalize(err));
        });
    }

    // Saved, or the switch goes back with why
    function setAction(row, action, on) {
        error = "";
        app.request("set_deck_action", { phone: phone, action: action, on: on }, (ok, err) => {
            if (!ok) {
                page.error = page.capitalize(err);
                row.checked = !on;
            } else if (page.asked.includes(action)) {
                page.load();
            }
        });
    }

    // Several at once (a group's buttons, or all it asked for); then the list as it is
    function setMany(ids, on) {
        error = "";
        let left = ids.length;
        for (const id of ids) {
            app.request("set_deck_action", { phone: phone, action: id, on: on }, (ok, err) => {
                if (!ok) page.error = page.capitalize(err);
                if (--left === 0) page.load();
            });
        }
    }

    // Tiles in a row: as many as fit, never fewer than two or more than three
    function columnsFor(width) {
        return Math.max(2, Math.min(3, Math.floor(width / (Kirigami.Units.gridUnit * 11))));
    }

    // The theme icon for a tile: by action, or for one of your own commands by the phone icon it has
    function tileIcon(action) {
        const byAction = {
            "media.play_pause": "media-playback-start", "media.next": "media-skip-forward", "media.previous": "media-skip-backward",
            "sound.volume": "audio-volume-high", "sound.mute": "audio-volume-muted", "sound.mic_mute": "microphone-sensitivity-muted",
            "sound.output": "audio-speakers", "system.lock": "system-lock-screen", "system.dnd": "notifications-disabled",
            "system.restart": "system-reboot", "system.shutdown": "system-shutdown", "network.wifi": "network-wireless",
            "network.bluetooth": "bluetooth-active", "network.airplane": "airplane-mode", "power.saver": "power-profile-power-saver",
            "power.balanced": "power-profile-balanced", "power.performance": "power-profile-performance", "power.auto": "battery-level-20"
        };
        const byKey = {
            play_pause: "media-playback-start", play: "media-playback-start", pause: "media-playback-pause", previous: "media-skip-backward",
            next: "media-skip-forward", speaker: "audio-volume-high", speaker_off: "audio-volume-muted", microphone: "audio-input-microphone",
            microphone_off: "microphone-sensitivity-muted", lock: "system-lock-screen", lightning: "weather-storm", star: "starred", heart: "starred",
            moon: "night-light", sun: "weather-clear", house: "go-home", bell: "preferences-system-notifications", code: "application-x-executable",
            power: "system-shutdown", rocket: "system-run", game: "input-gaming", camera: "camera-photo", coffee: "emoji-food",
            music: "audio-headphones", monitor: "video-display", broadcast: "network-wireless-hotspot", flag: "emblem-important",
            folder: "folder", globe: "network-workgroup", cloud: "weather-overcast", timer: "alarm"
        };
        return byAction[action.id] ?? byKey[action.icon ?? ""] ?? "utilities-terminal";
    }

    // One action: its icon, its name, a switch. A click anywhere on the tile turns the switch.
    component ActionTile: Rectangle {
        id: tile
        required property var modelData
        // The page: an inline component does not see its ids
        required property var host
        property bool reserve: false
        readonly property var action: modelData

        implicitHeight: body.implicitHeight + 2 * Kirigami.Units.largeSpacing
        radius: Kirigami.Units.smallSpacing * 2
        color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, tap.containsMouse ? 0.10 : 0.06)
        border.width: 1
        border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10)

        MouseArea {
            id: tap
            anchors.fill: parent
            hoverEnabled: true
            onClicked: {
                toggle.checked = !toggle.checked;
                tile.host.setAction(toggle, tile.action.id, toggle.checked);
            }
        }
        ColumnLayout {
            id: body
            anchors.fill: parent
            anchors.margins: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.smallSpacing
            RowLayout {
                Layout.fillWidth: true
                Kirigami.Icon {
                    source: tile.host.tileIcon(tile.action)
                    fallback: "utilities-terminal"
                    implicitWidth: Kirigami.Units.iconSizes.smallMedium
                    implicitHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Item { Layout.fillWidth: true }
                QQC2.Switch {
                    id: toggle
                    checked: tile.action.on
                    onToggled: tile.host.setAction(toggle, tile.action.id, checked)
                    QQC2.ToolTip.text: tile.action.name
                    Accessible.name: tile.action.name
                }
            }
            // Names show as they are, never as markup
            QQC2.Label {
                Layout.fillWidth: true
                font.bold: true
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                text: tile.action.name
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: tile.reserve
                opacity: 0.7
                font: Kirigami.Theme.smallFont
                textFormat: Text.PlainText
                text: tile.action.confirm ? tile.host.app.t("Asks first on {device}", { device: tile.host.name }) : " "
            }
        }
    }

    ColumnLayout {
        Layout.fillWidth: true
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.error !== ""
            type: Kirigami.MessageType.Error
            text: page.error
        }

        Kirigami.SearchField {
            Layout.fillWidth: true
            placeholderText: page.app.t("Search Actions")
            onTextChanged: page.query = text
        }

        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            textFormat: Text.PlainText
            text: page.app.t("What {device} may do on this computer. Arrange its buttons on {device}: open its deck and tap the pencil.", { device: page.name })
        }

        // What the computer keeps of the phone's layouts
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            visible: page.needle === ""
            QQC2.Label {
                Layout.fillWidth: true
                font.bold: true
                elide: Text.ElideRight
                textFormat: Text.PlainText
                text: page.layout ? page.app.t("Layout: {name}", { name: page.layout.name }) : page.app.t("No layout saved yet")
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                opacity: 0.7
                textFormat: Text.PlainText
                text: !page.layout
                    ? page.app.t("Until {device} saves one, it shows a first layout of what is offered here.", { device: page.name })
                    : page.app.t("{saved}, version {version}", {
                        saved: page.layout.saved_at ? page.app.t("Saved from {device} on {when}", { device: page.name, when: page.whenSaved(page.layout.saved_at) }) : page.app.t("Made on this computer"),
                        version: page.layout.version })
            }
        }

        // What its saved tiles run that is off here: the phone asked, you decide
        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.largeSpacing
            spacing: 0
            visible: page.needle === "" && page.askedOptions.length > 0
            RowLayout {
                Layout.fillWidth: true
                Kirigami.Heading {
                    Layout.fillWidth: true
                    level: 4
                    elide: Text.ElideRight
                    text: page.app.t("Asked for by {device}", { device: page.name })
                }
                QQC2.ToolButton {
                    flat: true
                    text: page.app.t("Allow All")
                    onClicked: page.setMany(page.askedOptions.map(a => a.id), true)
                }
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                opacity: 0.7
                text: page.app.t("Its deck has buttons for these, which stay off until you allow them.")
            }
            Flow {
                id: askedGrid
                // A tile is one column wide, even when it is alone on its line
                readonly property real cell: Math.max(0, Math.floor((width - spacing * (page.columnsFor(width) - 1)) / page.columnsFor(width)))
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.smallSpacing
                spacing: Kirigami.Units.largeSpacing
                Repeater {
                    model: page.askedOptions
                    delegate: ActionTile {
                        width: askedGrid.cell
                        host: page
                        reserve: page.askedOptions.some(a => !!a.confirm)
                    }
                }
            }
        }

        Repeater {
            model: page.groups
            delegate: ColumnLayout {
                id: group
                required property string modelData
                readonly property var shown: page.shownIn(modelData)
                readonly property var every: page.plain(modelData)
                visible: shown.length > 0
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.largeSpacing
                spacing: 0
                RowLayout {
                    Layout.fillWidth: true
                    Kirigami.Heading {
                        Layout.fillWidth: true
                        level: 4
                        elide: Text.ElideRight
                        text: page.groupTitle(group.modelData)
                    }
                    QQC2.ToolButton {
                        readonly property bool allOn: group.every.every(a => a.on)
                        flat: true
                        visible: group.every.length > 1
                        text: allOn ? page.app.t("Turn All Off") : page.app.t("Turn All On")
                        onClicked: page.setMany(group.every.map(a => a.id), !allOn)
                    }
                }
                Flow {
                    id: grid
                    readonly property real cell: Math.max(0, Math.floor((width - spacing * (page.columnsFor(width) - 1)) / page.columnsFor(width)))
                    Layout.fillWidth: true
                    Layout.topMargin: Kirigami.Units.smallSpacing
                    spacing: Kirigami.Units.largeSpacing
                    Repeater {
                        model: group.shown
                        delegate: ActionTile {
                            width: grid.cell
                            host: page
                            reserve: group.shown.some(a => !!a.confirm)
                        }
                    }
                }
            }
        }

        // Your own commands
        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.largeSpacing
            spacing: 0
            visible: page.needle === ""
            RowLayout {
                Layout.fillWidth: true
                Kirigami.Heading {
                    Layout.fillWidth: true
                    level: 4
                    text: page.app.t("Your Commands")
                }
                QQC2.Button {
                    icon.name: "list-add"
                    text: page.app.t("Add Command…")
                    onClicked: form.openFor(-1)
                }
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                opacity: 0.7
                textFormat: Text.PlainText
                text: page.app.t("Run anything on this computer from {device}'s deck: a script, a shortcut, a switch for a VPN. Each runs as you, with sh -c. Turn it on under its group above.", { device: page.name })
            }
            Repeater {
                model: page.commands
                delegate: RowLayout {
                    id: command
                    required property var modelData
                    required property int index
                    Layout.fillWidth: true
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: command.modelData.name
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: command.modelData.kind === "toggle"
                                ? command.modelData.command + " / " + (command.modelData.off ? command.modelData.off : page.app.t("same command"))
                                : command.modelData.command
                            textFormat: Text.PlainText
                            font.family: "monospace"
                            wrapMode: Text.WrapAnywhere
                            maximumLineCount: 2
                            elide: Text.ElideRight
                            opacity: 0.7
                        }
                    }
                    QQC2.ToolButton {
                        icon.name: "document-edit"
                        text: page.app.t("Edit Command")
                        display: QQC2.AbstractButton.IconOnly
                        QQC2.ToolTip.text: text
                        QQC2.ToolTip.visible: hovered
                        onClicked: form.openFor(command.index)
                    }
                    QQC2.ToolButton {
                        icon.name: "user-trash"
                        text: page.app.t("Remove Command")
                        display: QQC2.AbstractButton.IconOnly
                        QQC2.ToolTip.text: text
                        QQC2.ToolTip.visible: hovered
                        onClicked: page.saveCommands(page.commands.filter((o, i) => i !== command.index))
                    }
                }
            }
        }
    }

    // A command's form: a new one, or the one at `editing`; Save once it has what it needs
    Kirigami.Dialog {
        id: form
        // Where the command is in the list; -1: a new one
        property int editing: -1
        property var groupKeys: []
        property var iconKeys: []
        property string problem: ""
        readonly property bool switchKind: kindBox.currentIndex === 1
        readonly property bool newGroup: groupBox.currentIndex === groupKeys.length
        readonly property bool filled: nameField.text.trim() !== "" && onField.text.trim() !== ""
            && (!switchKind || stateField.text.trim() !== "") && (!newGroup || newGroupField.text.trim() !== "")
        title: editing >= 0 ? page.app.t("Edit Command") : page.app.t("Add Command")
        preferredWidth: Kirigami.Units.gridUnit * 28
        standardButtons: Kirigami.Dialog.NoButton

        function openFor(index) {
            const c = index >= 0 ? page.commands[index] : {};
            editing = index;
            problem = "";
            const custom = [];
            for (const other of page.commands) if (other.group && !custom.includes(other.group)) custom.push(other.group);
            groupKeys = [""].concat(custom);
            iconKeys = [""].concat(page.icons);
            nameField.text = c.name ?? "";
            kindBox.currentIndex = c.kind === "toggle" ? 1 : 0;
            onField.text = c.command ?? "";
            offField.text = c.off ?? "";
            stateField.text = c.state ?? "";
            groupBox.currentIndex = Math.max(0, groupKeys.indexOf(c.group ?? ""));
            newGroupField.text = "";
            iconBox.currentIndex = Math.max(0, iconKeys.indexOf(c.icon ?? ""));
            askBox.checked = c.confirm === true;
            tried.text = "";
            open();
            nameField.forceActiveFocus();
        }

        function save() {
            if (!filled) return;
            const chosen = {
                id: editing >= 0 ? page.commands[editing].id : "",
                name: nameField.text.trim(),
                command: onField.text.trim(),
                group: newGroup ? newGroupField.text.trim() : groupKeys[groupBox.currentIndex],
                icon: iconKeys[iconBox.currentIndex] ?? "",
                confirm: askBox.checked,
                kind: switchKind ? "toggle" : "button",
                off: switchKind ? offField.text.trim() : "",
                state: switchKind ? stateField.text.trim() : ""
            };
            const list = page.commands.slice();
            if (editing >= 0) list[editing] = chosen;
            else list.push(chosen);
            page.saveCommands(list, (ok, why) => {
                if (ok) form.close();
                else form.problem = why;
            });
        }

        // Run a line once on this computer and say how it went
        function tryLine(line) {
            if (line.trim() === "") return;
            tried.text = page.app.t("Running…");
            page.app.request("test_deck_command", { line: line }, (ok, run) => {
                if (!ok) {
                    tried.text = page.capitalize(run);
                    return;
                }
                const code = run.timed_out ? page.app.t("It was still running after 10 seconds, so it was stopped.")
                    : run.code === null ? page.app.t("It didn't start.") : page.app.t("Ended with code {code}.", { code: run.code });
                tried.text = run.output ? code + "\n" + run.output : code;
            });
        }

        customFooterActions: [
            Kirigami.Action {
                text: form.editing >= 0 ? page.app.t("Save") : page.app.t("Add")
                icon.name: "document-save"
                enabled: form.filled
                onTriggered: form.save()
            },
            Kirigami.Action {
                text: page.app.t("Cancel")
                icon.name: "dialog-cancel"
                onTriggered: form.close()
            }
        ]

        ColumnLayout {
            spacing: Kirigami.Units.smallSpacing

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: form.problem !== ""
                type: Kirigami.MessageType.Error
                text: form.problem
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                opacity: 0.7
                text: page.app.t("Runs on this computer as you, with sh -c. A switch turns something on and off; its check says which it is, by succeeding while it is on.")
            }
            QQC2.Label { text: page.app.t("Name") }
            QQC2.TextField {
                id: nameField
                Layout.fillWidth: true
                Layout.minimumWidth: Kirigami.Units.gridUnit * 18
                Accessible.name: page.app.t("Name")
                onAccepted: form.save()
            }
            QQC2.Label { text: page.app.t("Type") }
            QQC2.ComboBox {
                id: kindBox
                Layout.fillWidth: true
                model: [page.app.t("Button"), page.app.t("Switch")]
                Accessible.name: page.app.t("Type")
            }
            QQC2.Label { text: page.app.t("Command") }
            RowLayout {
                Layout.fillWidth: true
                QQC2.TextField {
                    id: onField
                    Layout.fillWidth: true
                    font.family: "monospace"
                    Accessible.name: page.app.t("Command")
                    onAccepted: form.save()
                }
                QQC2.ToolButton {
                    icon.name: "media-playback-start"
                    text: page.app.t("Try It Now")
                    display: QQC2.AbstractButton.IconOnly
                    QQC2.ToolTip.text: text
                    QQC2.ToolTip.visible: hovered
                    onClicked: form.tryLine(onField.text)
                }
            }
            QQC2.Label { text: page.app.t("Command to turn it off (optional)"); visible: form.switchKind }
            RowLayout {
                Layout.fillWidth: true
                visible: form.switchKind
                QQC2.TextField {
                    id: offField
                    Layout.fillWidth: true
                    font.family: "monospace"
                    Accessible.name: page.app.t("Command to turn it off (optional)")
                }
                QQC2.ToolButton {
                    icon.name: "media-playback-start"
                    text: page.app.t("Try It Now")
                    display: QQC2.AbstractButton.IconOnly
                    QQC2.ToolTip.text: text
                    QQC2.ToolTip.visible: hovered
                    onClicked: form.tryLine(offField.text)
                }
            }
            QQC2.Label { text: page.app.t("Check: succeeds while it is on"); visible: form.switchKind }
            RowLayout {
                Layout.fillWidth: true
                visible: form.switchKind
                QQC2.TextField {
                    id: stateField
                    Layout.fillWidth: true
                    font.family: "monospace"
                    Accessible.name: page.app.t("Check: succeeds while it is on")
                }
                QQC2.ToolButton {
                    icon.name: "media-playback-start"
                    text: page.app.t("Try It Now")
                    display: QQC2.AbstractButton.IconOnly
                    QQC2.ToolTip.text: text
                    QQC2.ToolTip.visible: hovered
                    onClicked: form.tryLine(stateField.text)
                }
            }
            QQC2.Label {
                id: tried
                Layout.fillWidth: true
                visible: text !== ""
                wrapMode: Text.Wrap
                opacity: 0.7
                font.family: "monospace"
                textFormat: Text.PlainText
            }
            QQC2.Label { text: page.app.t("Group") }
            QQC2.ComboBox {
                id: groupBox
                Layout.fillWidth: true
                model: form.groupKeys.map(k => k === "" ? page.groupTitle("Commands") : k).concat([page.app.t("New Group…")])
                Accessible.name: page.app.t("Group")
            }
            QQC2.TextField {
                id: newGroupField
                Layout.fillWidth: true
                visible: form.newGroup
                placeholderText: page.app.t("New group's name")
                Accessible.name: page.app.t("New group's name")
                onAccepted: form.save()
            }
            QQC2.Label { text: page.app.t("Icon") }
            QQC2.ComboBox {
                id: iconBox
                Layout.fillWidth: true
                model: form.iconKeys.map(k => k === "" ? page.app.t("Default") : page.iconTitle(k))
                Accessible.name: page.app.t("Icon")
            }
            QQC2.CheckBox {
                id: askBox
                text: page.app.t("Ask First")
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                opacity: 0.7
                text: page.app.t("The phone asks \"Run it?\" before it does.")
            }
        }
    }
}
