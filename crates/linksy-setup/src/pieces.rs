//! The pieces of GStreamer each feature needs, in one place: what Finish Setup looks for,
//! what the helper's self-check reports, and what a test holds the pipelines to. A camera
//! that closed at once for a missing `v4l2sink` nobody checked (desktop#85) is what this
//! prevents: an element a pipeline names is in this table, or the test fails.

/// One piece: any of `elements` will do. The packages that bring it, per family.
#[derive(Debug, PartialEq)]
pub struct Piece {
    pub elements: &'static [&'static str],
    pub fedora: &'static [&'static str],
    pub debian: &'static [&'static str],
    pub arch: &'static [&'static str],
    pub suse: &'static [&'static str],
}

/// GStreamer's base set: converters, scalers, raw audio.
pub const BASE: Piece = Piece {
    elements: &["videoconvert"],
    fedora: &["gstreamer1-plugins-base"],
    debian: &["gstreamer1.0-plugins-base"],
    arch: &["gst-plugins-base"],
    suse: &["gstreamer-plugins-base"],
};
/// Reads the H.264 a phone sends and this computer makes.
pub const H264_PARSE: Piece = Piece {
    elements: &["h264parse"],
    fedora: &["gstreamer1-plugins-bad-free"],
    debian: &["gstreamer1.0-plugins-bad"],
    arch: &["gst-plugins-bad"],
    suse: &["gstreamer-plugins-bad"],
};
pub const H264_DECODER: Piece = Piece {
    elements: &["avdec_h264", "vah264dec", "openh264dec"],
    fedora: &["gstreamer1-plugin-openh264", "gstreamer1-plugin-libav"],
    debian: &["gstreamer1.0-libav"],
    arch: &["gst-libav"],
    suse: &["gstreamer-plugin-openh264", "gstreamer-plugins-libav"],
};
pub const H264_ENCODER: Piece = Piece {
    elements: &["vah264lpenc", "vah264enc", "x264enc", "openh264enc"],
    fedora: &["gstreamer1-plugin-openh264"],
    debian: &["gstreamer1.0-plugins-ugly"],
    arch: &["gst-plugins-ugly"],
    suse: &["gstreamer-plugin-openh264"],
};
/// Writes the phone's camera to the Linksy Camera device, for browsers and Zoom.
pub const CAMERA_DEVICE: Piece = Piece {
    elements: &["v4l2sink"],
    fedora: &["gstreamer1-plugins-good"],
    debian: &["gstreamer1.0-plugins-good"],
    arch: &["gst-plugins-good"],
    suse: &["gstreamer-plugins-good"],
};
/// Reads the screen, and offers the phone's camera and microphone to apps.
pub const PIPEWIRE: Piece = Piece {
    elements: &["pipewiresrc"],
    fedora: &["pipewire-gstreamer"],
    debian: &["gstreamer1.0-pipewire"],
    arch: &["gst-plugin-pipewire"],
    suse: &["gstreamer-plugin-pipewire"],
};
/// Plays the phone's sound and hears this computer's microphone.
pub const SOUND: Piece = Piece {
    elements: &["autoaudiosink"],
    fedora: &["gstreamer1-plugins-good"],
    debian: &["gstreamer1.0-plugins-good"],
    arch: &["gst-plugins-good"],
    suse: &["gstreamer-plugins-good"],
};
/// Cuts this computer's microphone into the short buffers a call wants.
pub const MIC_BUFFERS: Piece = Piece {
    elements: &["audiobuffersplit"],
    fedora: &["gstreamer1-plugins-bad-free"],
    debian: &["gstreamer1.0-plugins-bad"],
    arch: &["gst-plugins-bad"],
    suse: &["gstreamer-plugins-bad"],
};
/// Reads an X11 display, where there is no portal.
pub const X11_SCREEN: Piece = Piece {
    elements: &["ximagesrc"],
    fedora: &["gstreamer1-plugins-good"],
    debian: &["gstreamer1.0-plugins-good"],
    arch: &["gst-plugins-good"],
    suse: &["gstreamer-plugins-good"],
};
/// The GTK window's video.
pub const GTK_SINK: Piece = Piece {
    elements: &["gtk4paintablesink"],
    fedora: &["gstreamer1-plugin-gtk4"],
    debian: &["gstreamer1.0-gtk4"],
    arch: &["gst-plugin-gtk4"],
    suse: &["gstreamer-plugins-rs"],
};
/// The Qt window's video.
pub const QT_SINK: Piece = Piece {
    elements: &["qml6glsink"],
    fedora: &["gstreamer1-plugins-good-qt6"],
    debian: &["gstreamer1.0-qt6"],
    arch: &["gst-plugin-qml6"],
    suse: &["gstreamer-plugins-good-qtqml6"],
};

/// What a person does with Linksy, as the helper runs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Feature {
    /// The phone's camera as a webcam here.
    Camera,
    /// The phone's microphone as a microphone here.
    Microphone,
    /// The phone's sound on this computer's speakers.
    Sound,
    /// This computer's screen, or a second display, on another device.
    Screen,
    /// This computer's microphone on a phone.
    ShareMicrophone,
}

impl Feature {
    pub const ALL: [Feature; 5] = [Feature::Camera, Feature::Microphone, Feature::Sound, Feature::Screen, Feature::ShareMicrophone];

    pub fn name(self) -> &'static str {
        match self {
            Feature::Camera => "phone camera",
            Feature::Microphone => "phone microphone",
            Feature::Sound => "phone sound",
            Feature::Screen => "screen sharing",
            Feature::ShareMicrophone => "this computer's microphone",
        }
    }

    /// Every piece it can't run without. The camera's two outputs are not here: one of
    /// them is enough (see `CAMERA_DEVICE` in Finish Setup).
    pub fn pieces(self) -> &'static [&'static Piece] {
        match self {
            Feature::Camera => &[&BASE, &H264_PARSE, &H264_DECODER],
            Feature::Microphone => &[&BASE, &PIPEWIRE],
            Feature::Sound => &[&BASE, &SOUND],
            Feature::Screen => &[&BASE, &H264_PARSE, &H264_ENCODER, &PIPEWIRE],
            Feature::ShareMicrophone => &[&BASE, &SOUND, &MIC_BUFFERS],
        }
    }

    /// The pieces it lacks here, by what `has` finds.
    pub fn lacking(self, has: impl Fn(&str) -> bool) -> Vec<&'static Piece> {
        self.pieces().iter().copied().filter(|p| !p.found(&has)).collect()
    }
}

impl Piece {
    pub fn found(&self, has: impl Fn(&str) -> bool) -> bool {
        self.elements.iter().any(|e| has(e))
    }
}

/// Every element a pipeline may name without it being a piece of its own: GStreamer's core
/// and base set (one `BASE` check covers base), the other names of a piece, and the ones
/// the code only uses when it finds them.
#[cfg(test)]
const KNOWN: &[&str] = &[
    // core
    "fdsrc", "fdsink", "fakesink", "queue", "tee", "capsfilter", "identity",
    // base
    "videoconvert", "videoscale", "videorate", "videotestsrc", "audioconvert", "audioresample", "audiotestsrc", "rawaudioparse", "volume",
    // with PIPEWIRE, SOUND and CAMERA_DEVICE (the same plugins)
    "pipewiresink", "autoaudiosrc", "v4l2src",
    // used only where found
    "capssetter", "vapostproc",
];

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [&Piece; 11] = [&BASE, &H264_PARSE, &H264_DECODER, &H264_ENCODER, &CAMERA_DEVICE, &PIPEWIRE, &SOUND, &MIC_BUFFERS, &X11_SCREEN, &GTK_SINK, &QT_SINK];

    /// The names in a source text that look like GStreamer elements.
    fn elements_in(source: &str) -> Vec<String> {
        const ENDS: [&str; 12] = ["sink", "src", "parse", "enc", "dec", "convert", "rate", "setter", "split", "scale", "resample", "postproc"];
        let mut found: Vec<String> = source
            .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
            .filter(|w| w.len() > 4 && ENDS.iter().any(|e| w.ends_with(e) && w.len() > e.len()))
            // Words of the code, not of a pipeline
            .filter(|w| !matches!(*w, "crate" | "bitrate" | "framerate" | "enumerate" | "vibrate" | "fixed_rate" | "adapt_rate" | "camera_sink" | "video_sink" | "low_rate" | "max_rate" | "constant_bitrate" | "picture_rate"))
            .map(String::from)
            .collect();
        found.sort();
        found.dedup();
        found
    }

    #[test]
    fn every_element_a_pipeline_names_is_checked() {
        let sources = [
            include_str!("../../linksy-screen/src/main.rs"),
            include_str!("../../linksy-screen/src/virtual_mic.rs"),
            include_str!("../../linksy-screen/src/x11.rs"),
            include_str!("../../linksy-gtk/src/window/screen.rs"),
        ];
        let covered = |name: &str| KNOWN.contains(&name) || ALL.iter().any(|p| p.elements.contains(&name));
        let unknown: Vec<String> = sources.iter().flat_map(|s| elements_in(s)).filter(|e| !covered(e)).collect();
        assert!(unknown.is_empty(), "elements no piece covers (add them to pieces.rs): {unknown:?}");
    }

    #[test]
    fn a_piece_names_packages_for_every_family() {
        for p in ALL {
            assert!(!p.elements.is_empty() && !p.fedora.is_empty() && !p.debian.is_empty() && !p.arch.is_empty() && !p.suse.is_empty(), "{p:?}");
        }
    }

    #[test]
    fn a_feature_says_what_it_lacks() {
        let has = |e: &str| e != "h264parse" && !e.contains("dec");
        assert_eq!(Feature::Camera.lacking(has), vec![&H264_PARSE, &H264_DECODER]);
        assert!(Feature::Sound.lacking(has).is_empty());
        assert_eq!(Feature::Screen.lacking(has), vec![&H264_PARSE]);
    }
}
