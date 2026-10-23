// The video item GStreamer draws a screen into. In its own file: when GStreamer's Qt 6 plugin
// is missing, only this fails to load, not the window.
import QtQuick
import org.freedesktop.gstreamer.Qt6GLVideoItem

GstGLQt6VideoItem {
    objectName: "linksyScreen"
}
