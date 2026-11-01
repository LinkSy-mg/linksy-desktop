//! Linksy for KDE Plasma: a Qt 6 / Kirigami window on linksyd, like the GTK app.

mod app;
mod backend;

fn main() {
    linksy_logs::catch_crashes("linksy-qt", env!("CARGO_PKG_VERSION"));
    rest();
    let args: cxx_qt_lib::QStringList = std::env::args().skip(1).map(|a| cxx_qt_lib::QString::from(a.as_str())).collect();
    std::process::exit(app::ffi::run_app(&args));
}

/// Light at rest (desktop#26), as in the GTK app: what the allocator keeps after pages are
/// rebuilt goes back to the system soon after start, then every minute.
fn rest() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(10));
        loop {
            // SAFETY: only hands the allocator's free memory back
            #[cfg(target_env = "gnu")]
            unsafe {
                libc::malloc_trim(0);
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });
}
