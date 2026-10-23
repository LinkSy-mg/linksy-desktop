use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("mg.diary.linksy").qml_files([
        "qml/Main.qml",
        "qml/DevicesPage.qml",
        "qml/DevicePage.qml",
        "qml/ActivityRow.qml",
        "qml/InviteDialog.qml",
        "qml/PrefsPage.qml",
    ]))
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qt_module("Widgets")
        .files(["src/backend.rs", "src/app.rs"]);
    // SAFETY: only adds this crate's own header folder; no flag that changes how the
    // generated code is compiled.
    let builder = unsafe { builder.cc_builder(|cc| {
        cc.include("include");
    }) };
    builder.build();
}
