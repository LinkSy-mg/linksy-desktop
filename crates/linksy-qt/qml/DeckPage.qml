// A phone's deck, from this computer's side: which actions the phone may run here, a switch
// each by group. The phone lays the buttons out.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    required property var app
    required property string phone
    required property string name
    // The actions the computer offers, each with whether it's on for this phone
    property var options: []
    property var commands: []
    property string error: ""
    // The groups, in the order the service gives them
    readonly property var groups: options.reduce((out, a) => out.includes(a.group) ? out : out.concat([a.group]), [])

    title: app.t("Deck")

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
        });
    }

    // The service names its groups in English; each shows in the app's language
    function groupTitle(group) {
        return ({ Media: app.t("Media"), Sound: app.t("Sound"), System: app.t("System"), Commands: app.t("Commands") })[group] ?? group;
    }

    // All of them go back (a new one without an id); then what's offered, with their actions
    function saveCommands(list) {
        error = "";
        app.request("set_deck_commands", { commands: list }, (ok, err) => {
            if (ok) page.load();
            else page.error = page.capitalize(err);
        });
    }

    // Saved, or the switch goes back with why
    function setAction(row, action, on) {
        error = "";
        app.request("set_deck_action", { phone: phone, action: action, on: on }, (ok, err) => {
            if (ok) return;
            page.error = page.capitalize(err);
            row.checked = !on;
        });
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.error !== ""
            type: Kirigami.MessageType.Error
            text: page.error
        }

        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            textFormat: Text.PlainText
            text: page.app.t("What {device} may do on this computer. Arrange the buttons on {device}.", { device: page.name })
        }

        Repeater {
            model: page.groups
            delegate: ColumnLayout {
                id: group
                required property string modelData
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.largeSpacing
                spacing: 0
                Kirigami.Heading {
                    level: 4
                    text: page.groupTitle(group.modelData)
                }
                Repeater {
                    model: page.options.filter(a => a.group === group.modelData)
                    delegate: QQC2.SwitchDelegate {
                        id: actionRow
                        required property var modelData
                        Layout.fillWidth: true
                        text: modelData.name
                        checked: modelData.on
                        onToggled: page.setAction(actionRow, modelData.id, checked)
                    }
                }
            }
        }

        Kirigami.Heading {
            Layout.topMargin: Kirigami.Units.largeSpacing
            level: 4
            text: page.app.t("Your Commands")
        }
        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            textFormat: Text.PlainText
            text: page.app.t("Each runs on this computer as you, with sh -c. Turn it on under Commands for {device}.", { device: page.name })
        }
        Repeater {
            model: page.commands
            delegate: RowLayout {
                id: command
                required property var modelData
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
                        text: command.modelData.command
                        textFormat: Text.PlainText
                        font.family: "monospace"
                        wrapMode: Text.WrapAnywhere
                        maximumLineCount: 2
                        elide: Text.ElideRight
                        opacity: 0.7
                    }
                }
                QQC2.ToolButton {
                    icon.name: "user-trash"
                    text: page.app.t("Remove Command")
                    display: QQC2.AbstractButton.IconOnly
                    QQC2.ToolTip.text: text
                    QQC2.ToolTip.visible: hovered
                    onClicked: page.saveCommands(page.commands.filter(o => o.id !== command.modelData.id))
                }
            }
        }
        QQC2.Button {
            icon.name: "list-add"
            text: page.app.t("Add Command…")
            onClicked: adding.open()
        }
    }

    // A name and a command line; Add once both are there
    Kirigami.PromptDialog {
        id: adding
        readonly property bool filled: nameField.text.trim() !== "" && lineField.text.trim() !== ""
        title: page.app.t("Add Command")
        subtitle: page.app.t("A button for the deck that runs this on this computer, as if typed in a terminal.")
        standardButtons: Kirigami.Dialog.NoButton
        onOpened: {
            nameField.text = "";
            lineField.text = "";
            nameField.forceActiveFocus();
        }
        function add() {
            if (!filled) return;
            page.saveCommands(page.commands.concat([{ name: nameField.text.trim(), command: lineField.text.trim() }]));
            close();
        }
        customFooterActions: [
            Kirigami.Action {
                text: page.app.t("Add")
                icon.name: "list-add"
                enabled: adding.filled
                onTriggered: adding.add()
            },
            Kirigami.Action {
                text: page.app.t("Cancel")
                icon.name: "dialog-cancel"
                onTriggered: adding.close()
            }
        ]
        ColumnLayout {
            QQC2.Label {
                text: page.app.t("Name")
            }
            QQC2.TextField {
                id: nameField
                Layout.fillWidth: true
                Layout.minimumWidth: Kirigami.Units.gridUnit * 18
                Accessible.name: page.app.t("Name")
                onAccepted: adding.add()
            }
            QQC2.Label {
                text: page.app.t("Command")
            }
            QQC2.TextField {
                id: lineField
                Layout.fillWidth: true
                font.family: "monospace"
                Accessible.name: page.app.t("Command")
                onAccepted: adding.add()
            }
        }
    }
}
