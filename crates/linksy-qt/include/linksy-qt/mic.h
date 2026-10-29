// The phone's microphone as apps hear it, measured for the wave on its device page: the
// PipeWire source the service publishes, through GStreamer's level element.
#pragma once

#include <gst/gst.h>

#include <algorithm>
#include <string>

namespace linksy_mic {

struct Meter {
    GstElement *pipeline = nullptr;
    double last = 0;
    gint64 at = 0;
};

inline Meter &meter()
{
    static Meter m;
    return m;
}

// Measure `source` (a GStreamer source description) instead of what was, or nothing
inline void measure(const char *source)
{
    Meter &m = meter();
    if (m.pipeline) {
        gst_element_set_state(m.pipeline, GST_STATE_NULL);
        gst_object_unref(m.pipeline);
    }
    m = {};
    if (!source)
        return;
    const std::string description = std::string(source) + " ! audioconvert ! level interval=60000000 ! fakesink sync=false";
    m.pipeline = gst_parse_launch(description.c_str(), nullptr);
    if (m.pipeline)
        gst_element_set_state(m.pipeline, GST_STATE_PLAYING);
}

} // namespace linksy_mic

inline void meter_microphone(bool on)
{
    if (on != (linksy_mic::meter().pipeline != nullptr))
        linksy_mic::measure(on ? "pipewiresrc target-object=linksy-mic" : nullptr);
}

// The loudest it got lately, 0 (-60 dB and below) to 1 (full scale), squared; -1 when not
// measuring
inline double microphone_level()
{
    linksy_mic::Meter &m = linksy_mic::meter();
    if (!m.pipeline)
        return -1;
    GstBus *bus = gst_element_get_bus(m.pipeline);
    double loudest = -1;
    while (GstMessage *msg = gst_bus_pop_filtered(bus, GST_MESSAGE_ELEMENT)) {
        const GstStructure *s = gst_message_get_structure(msg);
        const GValue *peaks = s && gst_structure_has_name(s, "level") ? gst_structure_get_value(s, "peak") : nullptr;
        G_GNUC_BEGIN_IGNORE_DEPRECATIONS
        if (peaks && G_VALUE_TYPE(peaks) == g_value_array_get_type()) {
            auto *array = static_cast<GValueArray *>(g_value_get_boxed(peaks));
            for (guint i = 0; array && i < array->n_values; i++) {
                // Squared, or speech fills every bar
                const double level = std::clamp((g_value_get_double(&array->values[i]) + 60) / 60, 0.0, 1.0);
                loudest = std::max(loudest, level * level);
            }
        }
        G_GNUC_END_IGNORE_DEPRECATIONS
        gst_message_unref(msg);
    }
    gst_object_unref(bus);
    // Asked about as often as it measures, not in step: hold the last a moment, then silence
    const gint64 now = g_get_monotonic_time();
    if (loudest >= 0) {
        m.last = loudest;
        m.at = now;
    } else if (now - m.at > 250000)
        m.last = 0;
    return m.last;
}
