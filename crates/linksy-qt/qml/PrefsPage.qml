// Preferences: this computer's name, where files go, how devices connect, the terminal code
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    required property var app
    title: app.t("Preferences")

    readonly property var s: app.settings

    // Settings go back whole, with one field changed
    function update(change) {
        const next = Object.assign({}, s, change);
        app.request("update_settings", { settings: next }, (ok, err) => {
            if (ok) app.settings = next; else app.toast(err);
        });
    }

    FolderDialog {
        id: folderPicker
        title: app.t("Save Files To")
        currentFolder: page.s ? "file://" + page.s.download_dir : ""
        onAccepted: page.update({ download_dir: decodeURIComponent(selectedFolder.toString().replace(/^file:\/\//, "")) })
    }

    Kirigami.FormLayout {
        enabled: page.s !== null

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: app.t("General")
        }
        RowLayout {
            Kirigami.FormData.label: app.t("Device Name") + ":"
            QQC2.TextField {
                id: nameField
                text: page.s?.name ?? ""
                Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                onAccepted: if (text.trim()) page.update({ name: text.trim() })
            }
            QQC2.Button {
                text: app.t("Save")
                enabled: nameField.text.trim() !== "" && nameField.text.trim() !== (page.s?.name ?? "")
                onClicked: page.update({ name: nameField.text.trim() })
            }
        }
        QQC2.Label {
            text: app.t("The name other devices see.")
            opacity: 0.7
            font: Kirigami.Theme.smallFont
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: app.t("Receiving")
        }
        RowLayout {
            Kirigami.FormData.label: app.t("Save Files To") + ":"
            QQC2.Label {
                text: (page.s?.download_dir ?? "").replace(/^\/home\/[^/]+/, "~")
                elide: Text.ElideMiddle
                Layout.maximumWidth: Kirigami.Units.gridUnit * 14
            }
            QQC2.Button {
                icon.name: "folder-open"
                text: app.t("Choose Folder")
                onClicked: folderPicker.open()
            }
        }
        QQC2.Switch {
            Kirigami.FormData.label: app.t("Accept From Paired Devices") + ":"
            checked: page.s?.auto_accept ?? false
            onToggled: page.update({ auto_accept: checked })
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.Wrap
            text: app.t("Files from devices you paired arrive without asking.")
            opacity: 0.7
            font: Kirigami.Theme.smallFont
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: app.t("Connection")
        }
        QQC2.Switch {
            Kirigami.FormData.label: app.t("Stay Connected") + ":"
            checked: page.s?.stay_connected ?? false
            onToggled: page.update({ stay_connected: checked })
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.Wrap
            text: app.t("Keep paired devices connected for instant sharing. Uses more battery; otherwise devices connect only when there's something to share.")
            opacity: 0.7
            font: Kirigami.Theme.smallFont
        }
        QQC2.Switch {
            Kirigami.FormData.label: app.t("Away From Home") + ":"
            checked: page.s?.away ?? true
            onToggled: page.update({ away: checked })
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.Wrap
            text: app.t("Reach your paired devices over the internet too, for a week after they were last on the same Wi-Fi. Everything stays encrypted end to end; Linksy's relay only helps them find each other.")
            opacity: 0.7
            font: Kirigami.Theme.smallFont
        }
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: app.t("Terminal")
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.Wrap
            text: app.t("Phones you allow (on their page) can open a shell on this computer after typing this code. It's checked here; the phone never keeps it.")
            opacity: 0.7
            font: Kirigami.Theme.smallFont
        }
        RowLayout {
            Kirigami.FormData.label: app.t("Terminal Code") + ":"
            QQC2.TextField {
                id: code
                echoMode: TextInput.Password
                placeholderText: app.terminal.code_set ? app.t("New code") : app.t("Not set: no phone can open a terminal")
                Layout.preferredWidth: Kirigami.Units.gridUnit * 12
            }
            QQC2.Button {
                text: app.terminal.code_set ? app.t("Change…").replace("…", "") : app.t("Set")
                enabled: code.text.length >= 6
                onClicked: app.request("set_terminal_code", { code: code.text }, (ok, err) => {
                    if (!ok) return app.toast(err);
                    code.text = "";
                    app.toast(app.t("Terminal code set"));
                    app.request("terminal_status", {}, (ok2, st) => { if (ok2) app.terminal = st });
                })
            }
            QQC2.Button {
                visible: app.terminal.code_set
                icon.name: "edit-delete"
                text: app.t("Turn Off")
                QQC2.ToolTip.text: app.t("Forget the code: no phone can open a terminal, and running ones end")
                QQC2.ToolTip.visible: hovered
                onClicked: app.request("set_terminal_code", { code: null }, () =>
                    app.request("terminal_status", {}, (ok, st) => { if (ok) app.terminal = st }))
            }
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.Wrap
            text: app.t("At least 6 characters (digits are easiest on a phone). Your phone asks for it every time a terminal opens; only this computer knows it. 5 wrong in a row lock it out.")
            opacity: 0.7
            font: Kirigami.Theme.smallFont
        }
    }
}
