// A phone's camera used here as a webcam: what apps get from it, and under it the phone's
// own camera controls (docs/android-app.md §2.8). The phone says what it can do and how
// it's set in `camera_state`; every change here goes back as a `camera_control`.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    required property var app
    required property string session
    required property string name
    property var state: null
    property string why: ""

    title: name
    padding: 0

    readonly property var lenses: state?.lenses ?? []
    readonly property var zooms: state?.zoom_presets ?? []
    readonly property var whiteBalances: state?.white_balances ?? []
    readonly property var qualities: [["saver", "Data Saver"], ["standard", "Standard"], ["high", "High"]]

    function control(change) {
        app.request("camera_control", { session: session, control: change });
    }

    // "daylight" -> "Daylight", in the language in use
    function wbTitle(wb) {
        const known = { auto: "Auto", daylight: "Daylight", cloudy: "Cloudy", shade: "Shade", incandescent: "Incandescent", fluorescent: "Fluorescent" };
        return known[wb] ? app.t(known[wb]) : wb.charAt(0).toUpperCase() + wb.slice(1);
    }

    function zoomLabel(z) {
        return (Number.isInteger(z) ? z.toFixed(0) : String(z)) + "×";
    }

    Connections {
        target: page.app
        function onCameraState(id, s) { if (id === page.session) page.state = s }
        function onSessionEnded(id) { if (id === page.session) page.close() }
    }

    function close() {
        backend.stopStream();
        app.pageStack.layers.pop();
    }

    Component.onCompleted: {
        if (!video.item) { why = app.t("Can't preview: a video part is missing"); return }
        why = backend.previewCamera("linksyScreen");
        app.request("camera_state", { session: session }, (ok, s) => { if (ok && s) page.state = s });
        // Development aid: a phone's camera_state, to see the controls without a phone
        const test = backend.env("LINKSY_TEST_CAMERA");
        if (test) page.state = JSON.parse(test);
    }
    Component.onDestruction: backend.stopStream()

    Rectangle {
        anchors.fill: parent
        color: "black"

        Loader {
            id: video
            anchors.fill: parent
            anchors.bottomMargin: bar.height
            source: canShowScreens ? "ScreenVideo.qml" : ""
            visible: page.why === ""
        }

        // Where the picture is inside the item (it keeps its shape, with bars around it)
        Item {
            id: picture
            readonly property var size: {
                page.state;
                const s = backend.streamSize().split("x").map(Number);
                return s.length === 2 && s[0] > 0 ? s : [16, 9];
            }
            readonly property real scale: Math.min(video.width / size[0], video.height / size[1])
            width: size[0] * scale
            height: size[1] * scale
            anchors.centerIn: video
        }

        // A click on the picture focuses the phone there
        MouseArea {
            anchors.fill: picture
            enabled: (page.state?.focus_modes ?? []).includes("point")
            cursorShape: enabled ? Qt.CrossCursor : Qt.ArrowCursor
            onClicked: mouse => page.control({ focus: "point", focus_point: [Math.min(Math.max(mouse.x / width, 0), 1), Math.min(Math.max(mouse.y / height, 0), 1)] })
        }

        Kirigami.PlaceholderMessage {
            anchors.centerIn: video
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: page.why !== ""
            Kirigami.Theme.colorSet: Kirigami.Theme.Complementary
            Kirigami.Theme.inherit: false
            icon.name: "dialog-warning"
            text: page.why
        }

        // The phone's camera controls
        QQC2.ToolBar {
            id: bar
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            contentItem: RowLayout {
                spacing: Kirigami.Units.largeSpacing
                Item { Layout.fillWidth: true }
                QQC2.ToolButton {
                    visible: page.lenses.some(l => l.facing === "front") && page.lenses.some(l => l.facing === "back")
                    icon.name: "exchange-positions"
                    text: app.t("Switch Camera")
                    onClicked: page.control({ facing: page.state?.facing === "front" ? "back" : "front" })
                }
                RowLayout {
                    spacing: 0
                    visible: page.zooms.length > 0
                    QQC2.ButtonGroup { id: zoomGroup }
                    Repeater {
                        model: page.zooms
                        QQC2.ToolButton {
                            required property real modelData
                            text: page.zoomLabel(modelData)
                            checkable: true
                            checked: Math.abs(modelData - (page.state?.zoom ?? 1)) < 0.05
                            QQC2.ButtonGroup.group: zoomGroup
                            onClicked: page.control({ zoom: modelData })
                        }
                    }
                }
                QQC2.ToolButton {
                    visible: page.state?.has_torch ?? false
                    icon.name: "flashlight-on"
                    text: app.t("Torch")
                    checkable: true
                    checked: page.state?.torch ?? false
                    onToggled: page.control({ torch: checked })
                }
                QQC2.ToolButton {
                    visible: ["point", "manual"].includes(page.state?.focus ?? "")
                    text: app.t("Auto Focus")
                    QQC2.ToolTip.text: app.t("Click the picture to focus there")
                    QQC2.ToolTip.visible: hovered
                    onClicked: page.control({ focus: "auto" })
                }
                QQC2.ToolButton {
                    enabled: page.state !== null
                    icon.name: "configure"
                    text: app.t("Camera Settings")
                    onClicked: settings.open()
                }
                Item { Layout.fillWidth: true }
            }
        }
    }

    Kirigami.Dialog {
        id: settings
        title: app.t("Camera Settings")
        standardButtons: Kirigami.Dialog.NoButton
        padding: Kirigami.Units.largeSpacing
        preferredWidth: Kirigami.Units.gridUnit * 24

        Kirigami.FormLayout {
            QQC2.Slider {
                Kirigami.FormData.label: app.t("Exposure") + ":"
                Layout.preferredWidth: Kirigami.Units.gridUnit * 10
                from: page.state?.exposure_min ?? 0
                to: Math.max(page.state?.exposure_max ?? 0, from + 1)
                stepSize: 1
                snapMode: QQC2.Slider.SnapAlways
                enabled: (page.state?.exposure_max ?? 0) > (page.state?.exposure_min ?? 0)
                value: page.state?.exposure ?? 0
                onMoved: page.control({ exposure: Math.round(value) })
            }
            QQC2.ComboBox {
                Kirigami.FormData.label: app.t("White Balance") + ":"
                model: page.whiteBalances.map(wb => page.wbTitle(wb))
                enabled: page.whiteBalances.length > 1
                currentIndex: Math.max(page.whiteBalances.indexOf(page.state?.white_balance), 0)
                onActivated: index => page.control({ white_balance: page.whiteBalances[index] })
            }
            QQC2.ComboBox {
                Kirigami.FormData.label: app.t("Quality") + ":"
                model: page.qualities.map(q => app.t(q[1]))
                currentIndex: Math.max(page.qualities.findIndex(q => q[0] === (page.state?.quality ?? "standard")), 0)
                onActivated: index => page.control({ quality: page.qualities[index][0] })
            }
            QQC2.Switch {
                id: fullHd
                Kirigami.FormData.label: "1080p:"
                visible: (page.state?.size_options ?? []).some(s => s[0] === 1920 && s[1] === 1080)
                checked: page.state?.height === 1080
                onToggled: page.control({ size: checked ? [1920, 1080] : [1280, 720] })
            }
            Hint {
                visible: fullHd.visible
                text: app.t("Sharper picture, more data. The camera restarts in your apps for a moment.")
            }
            QQC2.Switch {
                id: smooth
                Kirigami.FormData.label: app.t("60 Frames per Second") + ":"
                visible: (page.state?.fps_options ?? []).includes(60)
                checked: page.state?.fps === 60
                onToggled: page.control({ fps: checked ? 60 : 30 })
            }
            Hint {
                visible: smooth.visible
                text: app.t("Smoother motion. The camera restarts in your apps for a moment.")
            }
            QQC2.Switch {
                Kirigami.FormData.label: app.t("Portrait") + ":"
                checked: page.state?.orientation === "portrait"
                onToggled: page.control({ orientation: checked ? "portrait" : "landscape" })
            }
            QQC2.Switch {
                Kirigami.FormData.label: app.t("Stabilization") + ":"
                visible: page.state?.has_stabilization ?? false
                checked: page.state?.stabilization ?? false
                onToggled: page.control({ stabilization: checked })
            }
        }
    }

    component Hint: QQC2.Label {
        Layout.maximumWidth: Kirigami.Units.gridUnit * 14
        wrapMode: Text.Wrap
        opacity: 0.7
        font: Kirigami.Theme.smallFont
    }
}
