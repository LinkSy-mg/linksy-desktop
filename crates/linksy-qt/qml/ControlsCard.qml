// A phone's controls on its page (docs/protocol.md §4.24): its three volumes, the ringer, Do Not
// Disturb and the flashlight, and keeping Do Not Disturb in step with this computer's. The
// service watches the phone; this shows what it last said and asks for what the user changes.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Card {
    id: card
    required property var app
    // The phone: its id and name
    required property string phoneId
    required property string phoneName

    // What the service last knew: { state, sync_dnd, can_sync }
    property var info: null
    readonly property var controls: info?.state ?? null
    readonly property bool access: controls?.dnd_access ?? false
    // What the user changed and not yet sent: many moves of a slider make a few asks
    property var pending: ({})

    readonly property var ringers: ["normal", "vibrate", "silent"]
    readonly property var dnds: ["off", "priority", "alarms", "silence"]

    leftPadding: Kirigami.Units.largeSpacing * 2
    rightPadding: Kirigami.Units.largeSpacing * 2
    Layout.fillWidth: true
    visible: controls !== null

    function load() {
        if (!phoneId) return;
        const id = phoneId;
        app.request("phone_controls", { phone: id }, (ok, got) => {
            if (ok && id === card.phoneId) card.info = got;
        });
    }

    function ask(change) {
        pending = Object.assign({}, pending, change);
        if (!sender.running) sender.start();
    }

    onPhoneIdChanged: {
        info = null;
        load();
    }
    Component.onCompleted: load()

    Timer {
        id: sender
        interval: 150
        onTriggered: {
            const change = card.pending;
            card.pending = ({});
            card.app.request("set_phone_controls", { phone: card.phoneId, change: change }, (ok, err) => {
                if (!ok) {
                    card.app.toast(err);
                    card.load();
                }
            });
        }
    }

    Connections {
        target: card.app
        function onControlsChanged(phone, said) {
            if (phone !== card.phoneId) return;
            if (card.info === null) card.load();
            else card.info = Object.assign({}, card.info, { state: said });
        }
    }

    contentItem: Item {
        implicitWidth: body.implicitWidth
        implicitHeight: body.implicitHeight
        ColumnLayout {
            id: body
            width: parent.width
            spacing: Kirigami.Units.largeSpacing

            Kirigami.Heading {
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.largeSpacing
                level: 3
                text: card.app.t("Sound and Silence")
            }

            // The three volumes
            Repeater {
                model: [
                    { key: "media", icon: "audio-speakers", label: card.app.t("Media Volume") },
                    { key: "ringtone", icon: "phone", label: card.app.t("Ringtone Volume") },
                    { key: "alarm", icon: "player-time", label: card.app.t("Alarm Volume") }
                ]
                delegate: RowLayout {
                    id: level
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.largeSpacing
                    Kirigami.Icon {
                        implicitWidth: Kirigami.Units.iconSizes.smallMedium
                        implicitHeight: implicitWidth
                        source: card.app.sym(level.modelData.icon)
                        fallback: level.modelData.icon
                    }
                    QQC2.Label {
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 9
                        text: level.modelData.label
                        elide: Text.ElideRight
                    }
                    QQC2.Slider {
                        id: slider
                        Layout.fillWidth: true
                        from: 0
                        to: 1
                        stepSize: 0.05
                        // The phone's older answers don't pull it back while it is being moved
                        Binding on value {
                            value: card.controls ? card.controls[level.modelData.key] : 0
                            when: !slider.pressed && !held.running
                            restoreMode: Binding.RestoreNone
                        }
                        Timer {
                            id: held
                            interval: 1200
                        }
                        onMoved: {
                            held.restart();
                            const change = {};
                            change[level.modelData.key] = value;
                            card.ask(change);
                        }
                        Accessible.name: level.modelData.label
                    }
                    QQC2.Label {
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 2.5
                        horizontalAlignment: Text.AlignRight
                        text: Math.round(slider.value * 100) + "%"
                        opacity: 0.7
                    }
                }
            }

            Kirigami.Separator { Layout.fillWidth: true; opacity: 0.6 }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing
                QQC2.Label {
                    Layout.fillWidth: true
                    text: card.app.t("Ringer")
                }
                QQC2.ComboBox {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 12
                    model: [card.app.t("Normal"), card.app.t("Vibrate"), card.app.t("Silent")]
                    currentIndex: card.controls ? card.ringers.indexOf(card.controls.ringer) : -1
                    onActivated: index => card.ask({ ringer: card.ringers[index] })
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: card.app.t("Do Not Disturb")
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        visible: !card.access
                        wrapMode: Text.Wrap
                        font: Kirigami.Theme.smallFont
                        opacity: 0.7
                        text: card.app.t("Turn on Do Not Disturb access for Linksy in the phone's settings to change it from here")
                    }
                }
                QQC2.ComboBox {
                    enabled: card.access
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 12
                    model: [card.app.t("Off"), card.app.t("Priority Only"), card.app.t("Alarms Only"), card.app.t("Total Silence")]
                    currentIndex: card.controls ? card.dnds.indexOf(card.controls.dnd) : -1
                    onActivated: index => card.ask({ dnd: card.dnds[index] })
                }
            }

            RowLayout {
                Layout.fillWidth: true
                visible: card.controls?.flashlight !== undefined && card.controls?.flashlight !== null
                spacing: Kirigami.Units.largeSpacing
                QQC2.Label {
                    Layout.fillWidth: true
                    text: card.app.t("Flashlight")
                }
                QQC2.Switch {
                    checked: card.controls?.flashlight ?? false
                    onToggled: card.ask({ flashlight: checked })
                    Accessible.name: card.app.t("Flashlight")
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.bottomMargin: Kirigami.Units.largeSpacing
                visible: card.info?.can_sync ?? false
                spacing: Kirigami.Units.largeSpacing
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: card.app.t("Keep Do Not Disturb in Sync")
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        font: Kirigami.Theme.smallFont
                        opacity: 0.7
                        text: card.app.t("Turning it on or off here or on the phone does the same on the other")
                    }
                }
                QQC2.Switch {
                    enabled: card.access
                    checked: card.info?.sync_dnd ?? false
                    onToggled: {
                        const on = checked;
                        card.app.request("set_dnd_sync", { phone: card.phoneId, on: on }, (ok, err) => {
                            if (ok) card.info = Object.assign({}, card.info, { sync_dnd: on });
                            else {
                                card.app.toast(err);
                                card.load();
                            }
                        });
                    }
                    Accessible.name: card.app.t("Keep Do Not Disturb in Sync")
                }
            }
        }
    }
}
