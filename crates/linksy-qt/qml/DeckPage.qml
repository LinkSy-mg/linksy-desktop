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
    }
}
