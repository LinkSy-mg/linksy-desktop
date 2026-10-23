// Finish Setup: the system pieces some features need (the checks are linksy-setup's, as
// in the GTK app), with one button to add them all, or each on its own; one password.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    required property var app
    title: app.t("Finish Setup")

    readonly property var setup: app.setup
    property bool working: false
    property string step: ""

    function fix(ids) {
        working = true;
        step = "";
        backend.runSetup(JSON.stringify(ids));
    }

    Connections {
        target: backend
        function onSetupStep(text) { page.step = text }
        function onSetupFinished(json) {
            const r = JSON.parse(json);
            page.working = false;
            if (r.ok) app.toast(app.t("All set"));
            else if (r.error) app.toast(r.error);
            else if (r.restart) { restartDialog.subtitle = r.restart; restartDialog.open() }
            backend.checkSetup();
        }
    }

    Component.onCompleted: backend.checkSetup()

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            text: app.t("A few parts of your system make every feature work. Linksy adds what's missing in one go; your computer asks for your password once.")
        }

        QQC2.Button {
            Layout.alignment: Qt.AlignHCenter
            visible: (page.setup?.everything ?? 0) > 1
            enabled: !page.working
            highlighted: true
            icon.name: "run-build-install"
            text: app.t("Set Up Everything")
            onClicked: page.fix([])
        }

        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            visible: page.working
            QQC2.BusyIndicator { running: page.working; Layout.preferredHeight: Kirigami.Units.iconSizes.medium }
            QQC2.Label { text: page.step || app.t("Waiting for your password…"); wrapMode: Text.Wrap; opacity: 0.7 }
        }

        Kirigami.PlaceholderMessage {
            Layout.fillWidth: true
            visible: page.setup === null
            icon.name: "view-refresh"
            text: app.t("Checking…")
        }

        Repeater {
            model: page.setup?.items ?? []
            delegate: Kirigami.AbstractCard {
                id: card
                required property var modelData
                Layout.fillWidth: true
                contentItem: RowLayout {
                    spacing: Kirigami.Units.largeSpacing
                    Kirigami.Icon {
                        Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                        Layout.preferredHeight: width
                        Layout.alignment: Qt.AlignTop
                        // Rows with a button need no sign; the space stays, so the text lines up
                        opacity: card.modelData.need === "ready" || card.modelData.need === "manual" ? 1 : 0
                        source: card.modelData.need === "ready" ? "checkmark" : "dialog-information"
                        color: card.modelData.need === "ready" ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.textColor
                        isMask: card.modelData.need === "ready"
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Kirigami.Heading {
                            Layout.fillWidth: true
                            level: 4
                            text: card.modelData.title
                            wrapMode: Text.Wrap
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: card.modelData.why
                            wrapMode: Text.Wrap
                            opacity: 0.7
                        }
                    }
                    QQC2.Button {
                        visible: card.modelData.need === "install" || card.modelData.need === "setup"
                        enabled: !page.working
                        text: card.modelData.need === "setup" ? app.t("Set Up") : app.t("Install")
                        onClicked: page.fix([card.modelData.id])
                    }
                }
            }
        }

        // The same, by hand
        ColumnLayout {
            Layout.fillWidth: true
            visible: !!page.setup?.command
            Layout.topMargin: Kirigami.Units.largeSpacing
            Kirigami.Heading {
                level: 4
                text: app.t("Prefer a Terminal?")
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                opacity: 0.7
                text: app.t("Run this instead, then reopen Finish Setup")
            }
            RowLayout {
                Layout.fillWidth: true
                QQC2.TextArea {
                    id: command
                    Layout.fillWidth: true
                    readOnly: true
                    selectByMouse: true
                    wrapMode: Text.WrapAnywhere
                    font.family: "monospace"
                    text: page.setup?.command ?? ""
                }
                QQC2.Button {
                    Layout.alignment: Qt.AlignTop
                    icon.name: "edit-copy"
                    display: QQC2.AbstractButton.IconOnly
                    text: app.t("Copy")
                    QQC2.ToolTip.text: text
                    QQC2.ToolTip.visible: hovered
                    onClicked: { backend.setClipboard(command.text); icon.name = "checkmark" }
                }
            }
        }
    }

    // Secure Boot holds the new camera driver until it's approved at the next restart
    Kirigami.PromptDialog {
        id: restartDialog
        title: app.t("Restart to Finish")
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: app.t("Restart Now")
                icon.name: "system-reboot"
                onTriggered: backend.reboot()
            },
            Kirigami.Action {
                text: app.t("Later")
                onTriggered: restartDialog.close()
            }
        ]
    }
}
