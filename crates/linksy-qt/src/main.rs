//! Linksy for KDE Plasma: a Qt 6 / Kirigami window on linksyd, like the GTK app.

mod app;
mod backend;

fn main() {
    linksy_logs::catch_crashes("linksy-qt", env!("CARGO_PKG_VERSION"));
    let args: cxx_qt_lib::QStringList = std::env::args().skip(1).map(|a| cxx_qt_lib::QString::from(a.as_str())).collect();
    std::process::exit(app::ffi::run_app(&args));
}
