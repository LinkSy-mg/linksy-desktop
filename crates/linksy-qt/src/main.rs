//! Linksy for KDE Plasma: a Qt 6 / Kirigami window on linksyd, like the GTK app.

mod app;
mod backend;

fn main() {
    linksy_logs::catch_crashes("linksy-qt", env!("CARGO_PKG_VERSION"));
    std::process::exit(app::ffi::run_app());
}
