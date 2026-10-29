//! Development aid: what Finish Setup sees here, and what it would run.
//!
//! cargo run -p linksy-setup --example setup_check [-- --autostart]
//!
//! `--autostart` also turns on starting at login (no password).

fn main() {
    let items = linksy_setup::check(linksy_setup::Ui::Gtk);
    for i in &items {
        println!("{}: {:?}", i.id, i.need);
    }
    println!("missing: {}", linksy_setup::missing(&items));
    let args = linksy_setup::root_args(&linksy_setup::actionable(&items));
    match linksy_setup::root_command(&args) {
        Ok(argv) => println!("root: {}", argv.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" ")),
        Err(_) => println!("root: no helper"),
    }
    println!("terminal: {}", linksy_setup::terminal_command(&items).unwrap_or_default());
    println!("usb: {:?}", linksy_setup::usb_ready());
    if std::env::args().any(|a| a == "--autostart") {
        println!("autostart: {}", if linksy_setup::enable_autostart().is_ok() { "on" } else { "failed" });
    }
}
