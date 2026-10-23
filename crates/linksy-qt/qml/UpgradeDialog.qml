// Update Now: the newest version checked, downloaded, verified against the release's
// checksums and installed (linksy-upgrade, one password), then Linksy started again on it
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog
    required property var app
    property string phase: "checking"   // checking, offer, manual, running
    property var plan: ({})
    property string step: ""

    title: phase === "offer" || phase === "manual" ? app.t("Linksy {version} Is Available", { version: plan.latest ?? "" }) : app.t("Updating Linksy")
    standardButtons: Kirigami.Dialog.NoButton
    preferredWidth: Kirigami.Units.gridUnit * 24
    padding: Kirigami.Units.largeSpacing * 2
    closePolicy: phase === "running" ? QQC2.Popup.NoAutoClose : QQC2.Popup.CloseOnEscape | QQC2.Popup.CloseOnPressOutside
    showCloseButton: phase !== "running"

    function check() {
        phase = "checking";
        open();
        backend.checkUpgrade();
    }

    Connections {
        target: backend
        function onUpgradeChecked(json) {
            const r = JSON.parse(json);
            if (r.error) { dialog.close(); return dialog.app.toast(r.error) }
            if (!r.latest) { dialog.close(); return dialog.app.toast(dialog.app.t("Linksy {version} is the newest version.", { version: backend.version() })) }
            dialog.plan = r;
            dialog.phase = r.manual ? "manual" : "offer";
        }
        function onUpgradeStep(text) { dialog.step = text + "…" }
        function onUpgradeFinished(json) {
            const r = JSON.parse(json);
            if (r.ok) {
                dialog.step = dialog.app.t("Starting Linksy again…");
                // The service already runs the new version; the new window opens on it
                backend.restart();
            } else {
                dialog.phase = "offer";
                dialog.close();
                dialog.app.toast(r.error);
            }
        }
    }

    customFooterActions: [
        Kirigami.Action {
            visible: dialog.phase === "offer" || dialog.phase === "manual"
            text: dialog.app.t("Later")
            onTriggered: dialog.close()
        },
        Kirigami.Action {
            visible: (dialog.phase === "offer" || dialog.phase === "manual") && !!dialog.plan.notes
            text: dialog.app.t("What's New")
            onTriggered: Qt.openUrlExternally(dialog.plan.notes)
        },
        Kirigami.Action {
            visible: dialog.phase === "manual"
            text: dialog.app.t("Open the Download Page")
            icon.name: "internet-web-browser"
            onTriggered: Qt.openUrlExternally("https://linksy.diary.mg/#download")
        },
        Kirigami.Action {
            visible: dialog.phase === "offer"
            text: dialog.app.t("Update Now")
            icon.name: "update-none"
            onTriggered: { dialog.phase = "running"; dialog.step = dialog.app.t("Checking the release") + "…"; backend.runUpgrade() }
        }
    ]

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing
        QQC2.BusyIndicator {
            Layout.alignment: Qt.AlignHCenter
            visible: dialog.phase === "checking" || dialog.phase === "running"
            running: visible
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.preferredWidth: Kirigami.Units.gridUnit * 20
            horizontalAlignment: dialog.phase === "checking" || dialog.phase === "running" ? Text.AlignHCenter : Text.AlignLeft
            wrapMode: Text.Wrap
            text: dialog.phase === "checking" ? dialog.app.t("Checking for a newer Linksy…")
                : dialog.phase === "running" ? dialog.step
                : dialog.phase === "manual" ? (dialog.plan.manual ?? "")
                : dialog.app.t("This is {current}. Linksy downloads the update, checks it, installs it (your computer asks for your password), then starts again.", { current: dialog.plan.current ?? "" })
        }
    }
}
