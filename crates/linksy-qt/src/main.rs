//! Linksy for KDE Plasma: a Qt 6 / Kirigami window on linksyd, like the GTK app.

mod app;
mod backend;

fn main() {
    std::process::exit(app::ffi::run_app());
}
