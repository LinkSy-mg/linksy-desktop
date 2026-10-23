use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("mg.diary.linksy").qml_files([
        "qml/Main.qml",
        "qml/DevicesPage.qml",
        "qml/DevicePage.qml",
        "qml/ActivityRow.qml",
        "qml/InviteDialog.qml",
        "qml/PrefsPage.qml",
        "qml/ScreenPage.qml",
        "qml/ScreenVideo.qml",
        "qml/SetupPage.qml",
        "qml/CameraPage.qml",
        "qml/UsbPage.qml",
        "qml/WhatsNew.qml",
    ]))
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qt_module("Widgets")
        .qt_module("Network")
        .files(["src/backend.rs", "src/app.rs"]);
    // GStreamer, for other devices' screens (include/linksy-qt/video.h); probing also links it
    let gst = pkg_config::probe_library("gstreamer-1.0").expect("GStreamer development files (gstreamer1-devel)");
    // SAFETY: only adds include folders (this crate's and GStreamer's); no flag that changes
    // how the generated code is compiled.
    let builder = unsafe {
        builder.cc_builder(move |cc| {
            cc.include("include");
            cc.includes(&gst.include_paths);
        })
    };
    builder.build();
}
