// Another device's screen: watch it, take control of it (pointer, buttons, wheel, keys),
// or disconnect. Also this computer as another one's second display: full screen, with
// this mouse and keyboard working on it.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    required property var app
    required property string device
    required property string name
    property string session: ""
    property string kind: "screen"
    property string phase: "asking"   // asking, playing, ended
    property string why: ""
    property string detail: ""
    // A second display another computer started here: { session, socket }
    property var display: null
    // A video part is missing: Finish Setup adds it
    property bool missingPart: false
    readonly property bool controlling: (kind === "control" || kind === "display") && phase === "playing"

    title: kind === "display" ? app.t("Second Display for {name}", { name: name })
         : controlling ? app.t("Controlling {name}", { name: name }) : app.t("{name}'s Screen", { name: name })
    padding: 0
    // Full screen is all picture
    globalToolBarStyle: app.visibility === Window.FullScreen ? Kirigami.ApplicationHeaderStyle.None : Kirigami.ApplicationHeaderStyle.ToolBar

    actions: [
        Kirigami.Action {
            icon.name: "input-mouse"
            text: app.t("Take Control")
            checkable: true
            checked: page.controlling
            visible: page.kind !== "display"
            enabled: page.phase === "playing"
            tooltip: app.t("Use your mouse and keyboard on this screen")
            onTriggered: page.restart(checked ? "control" : "screen")
        },
        Kirigami.Action {
            icon.name: "view-fullscreen"
            text: app.t("Full Screen (F11)")
            shortcut: "F11"
            displayHint: Kirigami.DisplayHint.IconOnly
            onTriggered: app.visibility === Window.FullScreen ? app.showNormal() : app.showFullScreen()
        },
        Kirigami.Action {
            icon.name: "network-disconnect"
            text: app.t("Disconnect")
            onTriggered: page.close()
        }
    ]

    // The X11 keysym for a key (what the other device's input takes): named keys from a
    // table, letters and digits by key, anything else by the character it types
    function keysym(event) {
        const named = {
            [Qt.Key_Return]: 0xff0d, [Qt.Key_Enter]: 0xff8d, [Qt.Key_Backspace]: 0xff08, [Qt.Key_Tab]: 0xff09,
            [Qt.Key_Backtab]: 0xff09, [Qt.Key_Escape]: 0xff1b, [Qt.Key_Delete]: 0xffff, [Qt.Key_Insert]: 0xff63,
            [Qt.Key_Home]: 0xff50, [Qt.Key_End]: 0xff57, [Qt.Key_PageUp]: 0xff55, [Qt.Key_PageDown]: 0xff56,
            [Qt.Key_Left]: 0xff51, [Qt.Key_Up]: 0xff52, [Qt.Key_Right]: 0xff53, [Qt.Key_Down]: 0xff54,
            [Qt.Key_Shift]: 0xffe1, [Qt.Key_Control]: 0xffe3, [Qt.Key_Alt]: 0xffe9, [Qt.Key_AltGr]: 0xfe03,
            [Qt.Key_Meta]: 0xffeb, [Qt.Key_Super_L]: 0xffeb, [Qt.Key_Super_R]: 0xffec, [Qt.Key_Menu]: 0xff67,
            [Qt.Key_CapsLock]: 0xffe5, [Qt.Key_Space]: 0x20, [Qt.Key_Print]: 0xff61
        };
        if (event.key in named) return named[event.key];
        if (event.key >= Qt.Key_F1 && event.key <= Qt.Key_F24) return 0xffbe + (event.key - Qt.Key_F1);
        if (event.key >= Qt.Key_A && event.key <= Qt.Key_Z) return 0x61 + (event.key - Qt.Key_A);
        if (event.key >= Qt.Key_0 && event.key <= Qt.Key_9) return event.key;
        if (event.text.length === 1) {
            const c = event.text.codePointAt(0);
            if (c >= 0x20 && c <= 0x7e || c >= 0xa0 && c <= 0xff) return c;
            if (c > 0xff) return 0x1000000 + c;
        }
        return 0;
    }

    function start(k) {
        kind = k;
        phase = "asking";
        detail = k === "control" ? app.t("{name} needs to allow control and pick a screen.", { name: name })
                                 : app.t("{name} needs to allow sharing and pick a screen.", { name: name });
        app.request("start_session", { with: device, kind: k }, (ok, info) => {
            if (ok) return play(info);
            // Control refused: keep watching instead of losing the screen
            if (k === "control") {
                app.toast(info.includes("can't do this yet") ? app.t("{name} can't be controlled yet", { name: name })
                                                              : app.t("{name} didn't allow control", { name: name }));
                return start("screen");
            }
            if (info.includes("can't do this yet")) ended(app.t("{name} can't share its screen yet", { name: name }), app.t("This device doesn't support it in its current version."));
            else if (info.includes("declined")) ended(app.t("{name} didn't allow it", { name: name }), app.t("Nothing is shared unless the other device says yes."));
            else if (info.includes("timed out")) ended(app.t("No answer"), app.t("Nobody answered on {name}.", { name: name }));
            else ended(app.t("Couldn't Connect"), info);
        });
    }

    function play(info) {
        session = info.session;
        if (!info.socket) return ended(app.t("Couldn't Connect"), app.t("The service sent no stream."));
        const why = video.item ? backend.playStream("linksyScreen", info.socket) : "missing-part";
        if (why === "missing-part") {
            ended(app.t("Can't Show This Screen"), app.t("This computer is missing a video part Linksy needs. Finish Setup adds it, then watch again."));
            missingPart = true;
            return;
        }
        if (why) return ended(app.t("Couldn't Connect"), why);
        phase = "playing";
        if (controlling) surface.forceActiveFocus();
        // A second display fills this screen; F11 gets back to the window
        if (kind === "display") {
            app.showFullScreen();
            app.toast(app.t("Press F11 to leave full screen"));
        }
    }

    function stop() {
        backend.stopStream();
        if (session) app.request("end_session", { session: session });
        session = "";
    }

    function restart(k) {
        stop();
        start(k);
    }

    function ended(heading, text) {
        backend.stopStream();
        session = "";
        phase = "ended";
        missingPart = false;
        if (kind === "display" && heading === app.t("Sharing Stopped")) {
            heading = app.t("Second Display Ended");
            text = app.t("{name} stopped using this screen.", { name: name });
        }
        why = heading;
        detail = text;
        if (app.visibility === Window.FullScreen) app.showNormal();
    }

    function close() {
        stop();
        if (app.visibility === Window.FullScreen) app.showNormal();
        app.pageStack.layers.pop();
    }

    Connections {
        target: page.app
        function onSessionEnded(id) {
            if (id === page.session) page.ended(page.app.t("Sharing Stopped"), page.app.t("{name} stopped sharing its screen.", { name: page.name }));
        }
    }

    // Development aid: LINKSY_TEST_STREAM=<socket> plays that H.264 stream without asking
    // a device (scripts/qt-scenes/stream.py serves one)
    Component.onCompleted: {
        const test = backend.env("LINKSY_TEST_STREAM");
        if (display) { kind = "display"; play(display) }
        else if (test) play({ session: "", socket: test });
        else start("screen");
    }
    Component.onDestruction: stop()

    Rectangle {
        anchors.fill: parent
        color: "black"

        Loader {
            id: video
            anchors.fill: parent
            source: canShowScreens ? "ScreenVideo.qml" : ""
            visible: page.phase === "playing"
        }

        // Where the picture is inside the item (it keeps its shape, with bars around it)
        Item {
            id: picture
            readonly property var size: {
                page.phase;
                const s = backend.streamSize().split("x").map(Number);
                return s.length === 2 && s[0] > 0 ? s : [16, 9];
            }
            readonly property real scale: Math.min(parent.width / size[0], parent.height / size[1])
            width: size[0] * scale
            height: size[1] * scale
            anchors.centerIn: parent
        }

        // Input, while controlling: pointer as fractions of the picture, buttons, wheel, keys
        MouseArea {
            id: surface
            anchors.fill: picture
            enabled: page.controlling
            hoverEnabled: true
            acceptedButtons: Qt.AllButtons
            focus: page.controlling
            cursorShape: page.controlling ? Qt.BlankCursor : Qt.ArrowCursor

            function send(msg) { backend.sendControl(JSON.stringify(msg)) }
            function button(b) {
                return b === Qt.RightButton ? "right" : b === Qt.MiddleButton ? "middle" : b === Qt.BackButton ? "back" : b === Qt.ForwardButton ? "forward" : "left";
            }
            onPositionChanged: mouse => send({ type: "pointer", x: mouse.x / width, y: mouse.y / height })
            onPressed: mouse => { forceActiveFocus(); send({ type: "button", button: button(mouse.button), pressed: true }) }
            onReleased: mouse => send({ type: "button", button: button(mouse.button), pressed: false })
            onWheel: wheel => {
                const discrete = wheel.pixelDelta.x === 0 && wheel.pixelDelta.y === 0;
                const d = discrete ? Qt.point(wheel.angleDelta.x / 120, wheel.angleDelta.y / 120) : wheel.pixelDelta;
                send({ type: "scroll", dx: -d.x, dy: -d.y, discrete: discrete });
            }
            Keys.onPressed: event => { const k = page.keysym(event); if (k) { send({ type: "key", keysym: k, pressed: true }); event.accepted = true } }
            Keys.onReleased: event => { const k = page.keysym(event); if (k) { send({ type: "key", keysym: k, pressed: false }); event.accepted = true } }
        }

        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: page.phase !== "playing"
            Kirigami.Theme.colorSet: Kirigami.Theme.Complementary
            Kirigami.Theme.inherit: false
            icon.name: page.phase === "asking" ? "video-display" : "dialog-warning"
            text: page.phase === "asking" ? (page.kind === "control" ? page.app.t("Asking to control {name}…", { name: page.name }) : page.app.t("Asking {name}…", { name: page.name })) : page.why
            explanation: page.detail
            helpfulAction: page.missingPart ? finishSetup : reconnect
            Kirigami.Action {
                id: reconnect
                enabled: page.phase === "ended" && page.kind !== "display"
                icon.name: "view-refresh"
                text: page.app.t("Reconnect")
                onTriggered: page.start("screen")
            }
            Kirigami.Action {
                id: finishSetup
                icon.name: "run-build-install"
                text: page.app.t("Finish Setup")
                onTriggered: page.app.openSetup()
            }
        }
    }
}
