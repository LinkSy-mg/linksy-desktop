// Another device's screen, played in the window: the service hands over a Unix socket with
// the H.264 stream; GStreamer decodes it into a GstGLQt6VideoItem (qml6glsink). Input for
// control goes back on the same socket as framed JSON (docs/protocol.md §4.8).
#pragma once

#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRunnable>
#include <QString>

#include <gst/gst.h>

#include <arpa/inet.h>
#include <cstring>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

#include "cxx-qt-lib/qstring.h"

namespace linksy_video {

inline QQmlApplicationEngine *&engine()
{
    static QQmlApplicationEngine *e = nullptr;
    return e;
}

// Starts the pipeline from the render thread, once the window's OpenGL context exists for
// the sink to share (as GStreamer's qml6 example does)
class SetPlaying : public QRunnable
{
public:
    explicit SetPlaying(GstElement *pipeline)
        : pipeline(GST_ELEMENT(gst_object_ref(pipeline)))
    {
    }
    ~SetPlaying() override { gst_object_unref(pipeline); }
    void run() override { gst_element_set_state(pipeline, GST_STATE_PLAYING); }

private:
    GstElement *pipeline;
};

struct Playing {
    GstElement *pipeline = nullptr;
    int fd = -1;
};

inline Playing &playing()
{
    static Playing p;
    return p;
}

// Called before the QML loads: GStreamer registers the video item's QML type when its
// sink is first made.
inline bool prepare()
{
    gst_init(nullptr, nullptr);
    GstElement *sink = gst_element_factory_make("qml6glsink", nullptr);
    if (!sink)
        return false;
    gst_object_unref(sink);
    return true;
}

// Search the visual tree: items a Loader or a page stack makes aren't always QObject
// children of what shows them
inline QQuickItem *find_in(QQuickItem *item, const QString &name)
{
    if (item->objectName() == name)
        return item;
    for (QQuickItem *child : item->childItems()) {
        if (QQuickItem *found = find_in(child, name))
            return found;
    }
    return nullptr;
}

inline QQuickItem *find_item(const QString &name)
{
    if (!engine())
        return nullptr;
    for (QObject *root : engine()->rootObjects()) {
        if (auto *window = qobject_cast<QQuickWindow *>(root)) {
            if (QQuickItem *found = find_in(window->contentItem(), name))
                return found;
        }
    }
    return nullptr;
}

} // namespace linksy_video

inline void stop_stream()
{
    auto &p = linksy_video::playing();
    if (p.pipeline) {
        gst_element_set_state(p.pipeline, GST_STATE_NULL);
        gst_object_unref(p.pipeline);
        p.pipeline = nullptr;
    }
    if (p.fd >= 0) {
        ::close(p.fd);
        p.fd = -1;
    }
}

// Play `description` (a GStreamer pipeline ending in a qml6glsink named "sink") in the QML
// item named `item`, `fd` closing with it. Empty when it plays, else what went wrong.
inline QString play_pipeline(const QString &item, const QByteArray &description, int fd)
{
    QQuickItem *target = linksy_video::find_item(item);
    if (!target) {
        if (fd >= 0)
            ::close(fd);
        return QStringLiteral("no video item");
    }
    GError *error = nullptr;
    GstElement *pipeline = gst_parse_launch(description.constData(), &error);
    if (!pipeline) {
        QString why = QString::fromUtf8(error ? error->message : "pipeline");
        g_clear_error(&error);
        if (fd >= 0)
            ::close(fd);
        return why;
    }
    GstElement *sink = gst_bin_get_by_name(GST_BIN(pipeline), "sink");
    g_object_set(sink, "widget", target, nullptr);
    gst_object_unref(sink);
    if (QQuickWindow *window = target->window())
        window->scheduleRenderJob(new linksy_video::SetPlaying(pipeline), QQuickWindow::BeforeSynchronizingStage);
    else
        gst_element_set_state(pipeline, GST_STATE_PLAYING);
    linksy_video::playing() = { pipeline, fd };
    return {};
}

// Play the stream at `socket_path` in the QML item named `item`. Empty when it plays, else
// what went wrong.
inline QString play_stream(const QString &item, const QString &socket_path)
{
    stop_stream();
    const char *decoder = nullptr;
    for (const char *d : { "vah264dec", "avdec_h264", "openh264dec" }) {
        if (GstElementFactory *f = gst_element_factory_find(d)) {
            gst_object_unref(f);
            decoder = d;
            break;
        }
    }
    if (!decoder)
        return QStringLiteral("missing-part");

    int fd = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    sockaddr_un addr {};
    addr.sun_family = AF_UNIX;
    const QByteArray path = socket_path.toLocal8Bit();
    std::strncpy(addr.sun_path, path.constData(), sizeof(addr.sun_path) - 1);
    if (fd < 0 || ::connect(fd, reinterpret_cast<sockaddr *>(&addr), sizeof(addr)) != 0) {
        if (fd >= 0)
            ::close(fd);
        return QStringLiteral("couldn't connect to the stream");
    }

    // libav's frame threading holds a frame per thread first; slices keep it immediate
    const QByteArray dec = qstrcmp(decoder, "avdec_h264") == 0 ? QByteArray("avdec_h264 thread-type=slice") : QByteArray(decoder);
    return play_pipeline(item, QByteArray("fdsrc fd=") + QByteArray::number(fd) + " ! h264parse ! " + dec + " ! videoconvert ! glupload ! qml6glsink name=sink sync=false", fd);
}

// Show a local video source (a GStreamer source description: the Linksy Camera, as apps
// read it) in the QML item named `item`
inline QString play_source(const QString &item, const QString &source)
{
    stop_stream();
    return play_pipeline(item, source.toUtf8() + " ! videoconvert ! glupload ! qml6glsink name=sink sync=false", -1);
}

// The stream's picture size, "1920x1080" (empty until the first frame), for mapping the
// pointer onto it
inline QString stream_size()
{
    auto &p = linksy_video::playing();
    if (!p.pipeline)
        return {};
    GstElement *sink = gst_bin_get_by_name(GST_BIN(p.pipeline), "sink");
    if (!sink)
        return {};
    QString out;
    if (GstPad *pad = gst_element_get_static_pad(sink, "sink")) {
        if (GstCaps *caps = gst_pad_get_current_caps(pad)) {
            const GstStructure *s = gst_caps_get_structure(caps, 0);
            int w = 0, h = 0;
            if (gst_structure_get_int(s, "width", &w) && gst_structure_get_int(s, "height", &h))
                out = QStringLiteral("%1x%2").arg(w).arg(h);
            gst_caps_unref(caps);
        }
        gst_object_unref(pad);
    }
    gst_object_unref(sink);
    return out;
}

// One control message (JSON) to the device whose screen this is: a 4-byte big-endian
// length, then the JSON
inline bool send_control(const QString &json)
{
    auto &p = linksy_video::playing();
    if (p.fd < 0)
        return false;
    const QByteArray body = json.toUtf8();
    const uint32_t len = htonl(static_cast<uint32_t>(body.size()));
    QByteArray frame(reinterpret_cast<const char *>(&len), 4);
    frame += body;
    return ::send(p.fd, frame.constData(), frame.size(), MSG_NOSIGNAL | MSG_DONTWAIT) == frame.size();
}
