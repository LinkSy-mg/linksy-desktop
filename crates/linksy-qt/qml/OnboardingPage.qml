// The first run: a welcome in five slides, shown once where Linksy was never used, and
// skippable (as in the GTK app). What Linksy does, the phone's app (a QR code to its
// download), pairing (Add Device's code), Finish Setup (its checks and buttons), and where
// things are. A phone that pairs meanwhile moves it on.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    required property var app
    title: app.t("Welcome to Linksy")
    padding: 0
    globalToolBarStyle: Kirigami.ApplicationHeaderStyle.None

    readonly property int pairSlide: 2
    readonly property int setupSlide: 3
    readonly property var setup: app.setup
    property string link: ""
    property bool invited: false
    property string paired: ""
    property bool working: false
    property string step: ""

    function close() {
        backend.setOnboarded();
        if (invited) app.request("cancel_invite");
        app.pageStack.layers.pop();
    }

    function invite() {
        if (!backend.connected) { retry.start(); return }
        invited = true;
        app.request("create_invite", {}, (ok, uri) => { if (ok) page.link = uri });
    }

    function fix(ids) {
        working = true;
        step = "";
        backend.runSetup(JSON.stringify(ids));
    }

    // The service may still be starting on a first run
    Timer { id: retry; interval: 1000; onTriggered: page.invite() }
    // Invites last five minutes: a fresh one a little before
    Timer { interval: 270000; repeat: true; running: page.invited; onTriggered: page.invite() }

    Connections {
        target: backend
        function onEvent(json) {
            const ev = JSON.parse(json);
            if (ev.type !== "paired") return;
            page.paired = ev.device.name;
            if (slides.currentIndex === page.pairSlide) slides.currentIndex = page.setupSlide;
        }
        function onSetupStep(text) { page.step = text }
        function onSetupFinished(json) {
            const r = JSON.parse(json);
            page.working = false;
            if (r.ok) page.app.toast(page.app.t("All set"));
            else if (r.error) page.app.toast(r.error);
            else if (r.restart) { restartDialog.subtitle = r.restart; restartDialog.open() }
            backend.checkSetup();
        }
    }

    Component.onCompleted: {
        // Development aid (screenshots): LINKSY_ONBOARDING_SLIDE=3 opens on the third slide
        const n = Number(backend.env("LINKSY_ONBOARDING_SLIDE"));
        if (n >= 1 && n <= slides.count) slides.currentIndex = n - 1;
    }

    component Slide: QQC2.ScrollView {
        id: slide
        default property alias content: column.data
        contentWidth: availableWidth
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff
        Item {
            width: slide.availableWidth
            implicitHeight: Math.max(slide.availableHeight, column.implicitHeight + Kirigami.Units.gridUnit * 2)
            ColumnLayout {
                id: column
                anchors.centerIn: parent
                width: Math.min(parent.width - Kirigami.Units.gridUnit * 3, Kirigami.Units.gridUnit * 30)
                spacing: Kirigami.Units.largeSpacing
            }
        }
    }

    component SlideTitle: Kirigami.Heading {
        Layout.fillWidth: true
        level: 1
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
    }

    component SlideText: QQC2.Label {
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        opacity: 0.7
    }

    // One thing Linksy does, or where something is: an icon in a tinted circle and a line
    component Point: RowLayout {
        property alias icon: glyph.source
        property alias text: line.text
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: Kirigami.Units.gridUnit * 22
        Layout.maximumWidth: Kirigami.Units.gridUnit * 22
        spacing: Kirigami.Units.largeSpacing
        Rectangle {
            Layout.preferredWidth: Kirigami.Units.iconSizes.large
            Layout.preferredHeight: width
            radius: width / 2
            color: Qt.alpha(Kirigami.Theme.highlightColor, 0.14)
            Kirigami.Icon {
                id: glyph
                anchors.centerIn: parent
                width: Kirigami.Units.iconSizes.smallMedium
                height: width
                color: Kirigami.Theme.highlightColor
                isMask: true
            }
        }
        QQC2.Label { id: line; Layout.fillWidth: true; wrapMode: Text.Wrap }
    }

    component Code: Rectangle {
        property string text: ""
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: Kirigami.Units.gridUnit * 12
        Layout.preferredHeight: width
        radius: Kirigami.Units.cornerRadius
        // White in both styles: scanners expect dark on light
        color: "white"
        Image {
            anchors.fill: parent
            anchors.margins: Kirigami.Units.smallSpacing
            source: parent.text ? backend.qrCode(parent.text) : ""
            sourceSize: Qt.size(width * 2, height * 2)
            smooth: false
        }
        QQC2.BusyIndicator { anchors.centerIn: parent; visible: parent.text === ""; running: visible }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.smallSpacing
            Item { Layout.fillWidth: true }
            QQC2.Button { flat: true; text: page.app.t("Skip"); onClicked: page.close() }
        }

        QQC2.SwipeView {
            id: slides
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            onCurrentIndexChanged: {
                if (currentIndex === page.pairSlide && !page.invited) page.invite();
                if (currentIndex === page.setupSlide) backend.checkSetup();
            }

            Slide {
                Image {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.preferredWidth: Kirigami.Units.iconSizes.enormous
                    Layout.preferredHeight: width
                    source: backend.logo()
                    sourceSize: Qt.size(width * 2, height * 2)
                }
                SlideTitle { text: page.app.t("Welcome to Linksy") }
                SlideText { text: page.app.t("Your phone and this computer, working as one.") }
                Item { Layout.preferredHeight: Kirigami.Units.smallSpacing }
                Point { icon: "document-send"; text: page.app.t("Files and the clipboard, both ways") }
                Point { icon: "camera-web"; text: page.app.t("The phone as a webcam, a microphone or a second screen") }
                Point { icon: "mail-unread"; text: page.app.t("Its texts, notifications and calls on this computer") }
            }

            Slide {
                SlideTitle { text: page.app.t("Get Linksy on Your Phone") }
                Code { text: backend.phoneApp() }
                SlideText { text: page.app.t("Scan this with your phone's camera to download the Android app, then open it.") }
                Kirigami.UrlButton {
                    Layout.alignment: Qt.AlignHCenter
                    url: backend.phoneApp()
                    text: backend.phoneApp().replace("https://", "")
                }
            }

            Slide {
                SlideTitle { text: page.app.t("Pair Your Phone") }
                Code { text: page.link }
                SlideText { text: page.app.t("In Linksy on your phone, choose Add Device and scan this code. Whoever scans it can pair, so only show it to your own devices.") }
                QQC2.Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    font.bold: true
                    color: page.paired ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.textColor
                    text: page.paired ? page.app.t("Paired with {name}", { name: page.paired }) : page.app.t("Waiting for your phone…")
                }
            }

            Slide {
                SlideTitle { text: page.app.t("Finish Setup") }
                SlideText { text: page.app.t("A few parts of your system make every feature work. Linksy adds what's missing in one go; your computer asks for your password once.") }
                QQC2.Button {
                    Layout.alignment: Qt.AlignHCenter
                    visible: (page.setup?.everything ?? 0) > 1
                    enabled: !page.working
                    highlighted: true
                    icon.name: "run-build-install"
                    text: page.app.t("Set Up Everything")
                    onClicked: page.fix([])
                }
                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    visible: page.working
                    QQC2.BusyIndicator { running: page.working; Layout.preferredHeight: Kirigami.Units.iconSizes.medium }
                    QQC2.Label { text: page.step || page.app.t("Waiting for your password…"); wrapMode: Text.Wrap; opacity: 0.7 }
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
                                opacity: card.modelData.need === "ready" || card.modelData.need === "manual" ? 1 : 0
                                source: card.modelData.need === "ready" ? "checkmark" : "dialog-information"
                                color: card.modelData.need === "ready" ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.textColor
                                isMask: card.modelData.need === "ready"
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                Kirigami.Heading { Layout.fillWidth: true; level: 4; text: card.modelData.title; wrapMode: Text.Wrap }
                                QQC2.Label { Layout.fillWidth: true; text: card.modelData.why; wrapMode: Text.Wrap; opacity: 0.7 }
                            }
                            QQC2.Button {
                                visible: card.modelData.need === "install" || card.modelData.need === "setup"
                                enabled: !page.working
                                text: card.modelData.need === "setup" ? page.app.t("Set Up") : page.app.t("Install")
                                onClicked: page.fix([card.modelData.id])
                            }
                        }
                    }
                }
            }

            Slide {
                Kirigami.Icon {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.preferredWidth: Kirigami.Units.iconSizes.huge
                    Layout.preferredHeight: width
                    source: "checkmark"
                    color: Kirigami.Theme.positiveTextColor
                    isMask: true
                }
                SlideTitle { text: page.app.t("You're All Set") }
                Item { Layout.preferredHeight: Kirigami.Units.smallSpacing }
                Point { icon: "computer"; text: page.app.t("Your devices are in the list on the left: pick one to share with it") }
                Point { icon: "document-send"; text: page.app.t("Drop files on a device's page, or use Send with Linksy in your file manager") }
                Point { icon: "configure"; text: page.app.t("Finish Setup shows under the list while something is missing; Preferences are in the menu") }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing * 2
            Item {
                Layout.preferredWidth: next.implicitWidth
                Layout.preferredHeight: next.implicitHeight
                QQC2.Button {
                    anchors.left: parent.left
                    flat: true
                    visible: slides.currentIndex > 0
                    text: page.app.t("Back")
                    onClicked: slides.currentIndex -= 1
                }
            }
            Item { Layout.fillWidth: true }
            QQC2.PageIndicator { count: slides.count; currentIndex: slides.currentIndex }
            Item { Layout.fillWidth: true }
            QQC2.Button {
                id: next
                highlighted: true
                readonly property bool last: slides.currentIndex === slides.count - 1
                text: last ? page.app.t("Start Using Linksy") : page.app.t("Next")
                onClicked: last ? page.close() : slides.currentIndex += 1
            }
        }
    }

    // Secure Boot holds the new camera driver until it's approved at the next restart
    Kirigami.PromptDialog {
        id: restartDialog
        title: page.app.t("Restart to Finish")
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action { text: page.app.t("Later"); onTriggered: restartDialog.close() },
            Kirigami.Action { text: page.app.t("Restart Now"); onTriggered: backend.reboot() }
        ]
    }
}
