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
    property var photos: null
    property var updates: null
    readonly property var languages: JSON.parse(backend.languages())
    readonly property string address: backend.localAddress(app.status?.port ?? 0)

    Component.onCompleted: {
        app.request("photo_prefs", {}, (ok, p) => { if (ok) page.photos = p });
        app.request("updates", {}, (ok, u) => { if (ok) page.updates = u });
    }

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
        // For "Pair by Address" on networks that hide devices from each other
        RowLayout {
            Kirigami.FormData.label: app.t("Address") + ":"
            visible: page.address !== ""
            QQC2.Label {
                text: page.address
                textFormat: Text.PlainText
            }
            QQC2.ToolButton {
                icon.name: "edit-copy"
                display: QQC2.AbstractButton.IconOnly
                text: app.t("Copy Address")
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                onClicked: { backend.setClipboard(page.address); app.toast(app.t("Address copied")) }
            }
        }
        // Read when Linksy starts, so it offers to start Linksy (and its service) again
        QQC2.ComboBox {
            Kirigami.FormData.label: app.t("Language") + ":"
            model: [app.t("System Default")].concat(page.languages.map(l => l.name))
            currentIndex: page.languages.findIndex(l => l.code === backend.chosenLanguage()) + 1
            onActivated: index => {
                const why = backend.chooseLanguage(index === 0 ? "" : page.languages[index - 1].code);
                if (why) return app.toast(why);
                app.showPassiveNotification(app.t("Linksy uses it the next time it starts"), "long", app.t("Restart Now"), () => backend.restart());
            }
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
        QQC2.Switch {
            Kirigami.FormData.label: app.t("Import Photos From Phones") + ":"
            visible: page.photos !== null
            checked: page.photos?.import ?? false
            onToggled: {
                const next = Object.assign({}, page.photos, { import: checked });
                app.request("set_photo_prefs", { prefs: next }, (ok, err) => { if (ok) page.photos = next; else app.toast(err) });
            }
        }
        QQC2.Label {
            visible: page.photos !== null
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.Wrap
            text: app.t("New photos from your phone go to {folder}, in a folder named after it, without asking.", { folder: (page.photos?.folder ?? "").replace(/^\/home\/[^/]+/, "~") })
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
        // Self-hosting: a relay of the user's own, instead of Linksy's (linksy-server)
        RowLayout {
            Kirigami.FormData.label: app.t("Relay Server") + ":"
            QQC2.TextField {
                id: relayField
                text: page.s?.relay ?? ""
                placeholderText: "https://relay.example.com"
                Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                // Blank: back to Linksy's own relay
                onAccepted: page.update({ relay: text.trim() || null })
            }
            QQC2.Button {
                text: app.t("Save")
                enabled: relayField.text.trim() !== (page.s?.relay ?? "")
                onClicked: page.update({ relay: relayField.text.trim() || null })
            }
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.Wrap
            text: app.t("Away from home goes through Linksy's relay unless you give one you host yourself, as https://relay.example.com. Set the same relay on every device.")
            opacity: 0.7
            font: Kirigami.Theme.smallFont
        }
        // The screen other devices see: picked in the desktop's dialog once, then remembered
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: app.t("Screen Sharing")
        }
        QQC2.Button {
            id: chooseAgain
            icon.name: "video-display"
            text: app.t("Choose the Screen Again")
            onClicked: app.request("forget_screen", {}, (ok, err) => {
                if (!ok) return app.toast(err);
                chooseAgain.enabled = false;
                app.toast(app.t("The next time a device asks, you pick the screen."));
            })
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.Wrap
            text: app.t("Linksy remembers the screen you picked when a device first asked to see or control this computer. Choose again to pick another monitor the next time one asks.")
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

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: app.t("Updates")
            visible: page.updates !== null
        }
        QQC2.Switch {
            Kirigami.FormData.label: app.t("Check for Updates") + ":"
            visible: page.updates !== null
            checked: page.updates?.prefs.check ?? true
            onToggled: app.request("set_update_prefs", { prefs: { check: checked } }, (ok, err) => {
                if (!ok) app.toast(err);
                app.refreshUpdates();
            })
        }
        QQC2.Label {
            visible: page.updates !== null
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.Wrap
            text: app.t("Once a day, Linksy asks linksy.diary.mg for the newest version. Nothing about you or your devices is sent.") + " " + app.t("This is Linksy {version}.", { version: page.updates?.current ?? "" })
            opacity: 0.7
            font: Kirigami.Theme.smallFont
        }
    }
}
