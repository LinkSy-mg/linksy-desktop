// What's New: this version's highlights (linksy-news), the first time it opens after an
// update and from the menu. Each fades and rises in a moment after the one before, its icon
// popping; with Plasma's animations off, they're simply there.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog
    required property var app
    property var news: ({ version: "", notes: "", repo: "", highlights: [] })
    // Plasma's animation speed: 0 when animations are off
    readonly property int step: Kirigami.Units.longDuration

    title: app.t("What's New")
    standardButtons: Kirigami.Dialog.NoButton
    preferredWidth: Kirigami.Units.gridUnit * 26
    padding: Kirigami.Units.gridUnit

    // After an update: once. From the menu: any time.
    function offer() {
        news = JSON.parse(backend.whatsNew(false));
        if (news.highlights.length) open();
    }
    function showAnyTime() {
        news = JSON.parse(backend.whatsNew(true));
        open();
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing * 2

        Kirigami.Icon {
            id: logo
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: Kirigami.Units.iconSizes.huge
            Layout.preferredHeight: Kirigami.Units.iconSizes.huge
            source: "mg.diary.linksy.Linksy"
            fallback: "network-connect"
            scale: dialog.opened ? 1 : 0.5
            Behavior on scale { NumberAnimation { duration: dialog.step * 1.6; easing.type: Easing.OutBack } }
        }
        Kirigami.Heading {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            level: 1
            text: dialog.app.t("What's New in Linksy {version}", { version: dialog.news.version })
        }

        Repeater {
            model: dialog.opened ? dialog.news.highlights : []
            delegate: RowLayout {
                id: row
                required property var modelData
                required property int index
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing
                opacity: 0
                transform: Translate { id: rise; y: Kirigami.Units.gridUnit * 0.6 }

                // One after another, fading and rising in, the icon popping
                SequentialAnimation {
                    running: true
                    PauseAnimation { duration: dialog.step * (row.index + 1) * 0.5 }
                    ParallelAnimation {
                        NumberAnimation { target: row; property: "opacity"; to: 1; duration: dialog.step * 1.2; easing.type: Easing.OutCubic }
                        NumberAnimation { target: rise; property: "y"; to: 0; duration: dialog.step * 1.2; easing.type: Easing.OutCubic }
                        NumberAnimation { target: bubble; property: "scale"; from: 0.5; to: 1; duration: dialog.step * 1.6; easing.type: Easing.OutBack }
                    }
                }

                Rectangle {
                    id: bubble
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: Kirigami.Units.iconSizes.medium + Kirigami.Units.smallSpacing * 2
                    Layout.preferredHeight: Layout.preferredWidth
                    radius: width / 2
                    color: Qt.alpha(Kirigami.Theme.highlightColor, 0.15)
                    Kirigami.Icon {
                        anchors.centerIn: parent
                        width: Kirigami.Units.iconSizes.smallMedium
                        height: width
                        source: row.modelData.icon
                        color: Kirigami.Theme.highlightColor
                        isMask: true
                    }
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    Kirigami.Heading {
                        Layout.fillWidth: true
                        level: 4
                        wrapMode: Text.Wrap
                        text: row.modelData.title
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        opacity: 0.7
                        text: row.modelData.body
                    }
                }
            }
        }

        Kirigami.UrlButton {
            Layout.alignment: Qt.AlignHCenter
            url: dialog.news.notes
            text: dialog.app.t("All Changes in {version}", { version: dialog.news.version })
        }

        QQC2.Button {
            Layout.alignment: Qt.AlignHCenter
            highlighted: true
            text: dialog.app.t("Continue")
            onClicked: dialog.close()
        }
        // A word for the project: the star wiggles when pointed at
        QQC2.Button {
            id: star
            Layout.alignment: Qt.AlignHCenter
            icon.name: "starred-symbolic"
            text: dialog.app.t("Star Linksy on GitHub")
            onClicked: Qt.openUrlExternally(dialog.news.repo)
            onHoveredChanged: if (hovered && dialog.step > 0) wiggle.restart()
            SequentialAnimation {
                id: wiggle
                RotationAnimation { target: star.contentItem; to: -10; duration: dialog.step * 0.6; easing.type: Easing.OutQuad }
                RotationAnimation { target: star.contentItem; to: 7; duration: dialog.step * 0.6; easing.type: Easing.InOutQuad }
                RotationAnimation { target: star.contentItem; to: 0; duration: dialog.step * 0.6; easing.type: Easing.InQuad }
            }
        }
        QQC2.Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            opacity: 0.7
            font: Kirigami.Theme.smallFont
            text: dialog.app.t("Linksy is free and open source. A star on GitHub helps others find it.")
        }
    }
}
