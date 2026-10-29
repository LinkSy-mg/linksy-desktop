//! Send with Linksy in file managers, from the Flatpak: its own copies of these entries are
//! inside the sandbox, where Dolphin, GNOME Files and Thunar don't look. The service writes
//! them into the home folder for the file managers installed, each starting the Flatpak
//! with the files handed over (`--file-forwarding`, so the sandbox may read them).

const DOLPHIN: &str = include_str!("../../../data/linksy-send.desktop");
const THUNAR: &str = include_str!("../../../data/linksy-thunar.desktop");
const NAUTILUS: &str = include_str!("../../../data/nautilus/linksy.py");

/// Each file manager's entry for the Flatpak `app`: (program, where under the data folder,
/// contents, executable).
fn entries(app: &str) -> [(&'static str, &'static str, String, bool); 3] {
    let exec = format!("Exec=flatpak run --file-forwarding {app} @@ %F @@");
    let launch = format!("return [\"flatpak\", \"run\", \"--file-forwarding\", \"{app}\", \"@@\", *paths, \"@@\"]");
    [
        // Dolphin only reads service menus in the home folder when they're executable
        ("dolphin", "kio/servicemenus/linksy-send.desktop", DOLPHIN.replace("Exec=linksy-open %F", &exec), true),
        ("thunar", "Thunar/sendto/linksy.desktop", THUNAR.replace("Exec=linksy-open %F", &exec), false),
        ("nautilus", "nautilus-python/extensions/linksy.py", NAUTILUS.replace("return [\"linksy-open\", *paths]", &launch), false),
    ]
}

/// Write the entries that are missing or out of date. Blocking: for a thread of its own.
pub fn install() {
    let Ok(app) = std::env::var("FLATPAK_ID") else { return };
    let data = linksy_host::data_dir();
    for (program, file, contents, executable) in entries(&app) {
        let path = data.join(file);
        if !linksy_host::installed(program) || linksy_host::read(&path).as_deref() == Some(contents.as_str()) {
            continue;
        }
        if linksy_host::write(&path, contents.as_bytes()).is_ok() && executable {
            let _ = linksy_host::command("chmod", &[]).arg("+x").arg(&path).status();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_starts_the_flatpak() {
        for (_, file, contents, _) in entries("mg.diary.linksy.Linksy") {
            assert!(contents.contains("--file-forwarding"), "{file} still starts linksy-open");
            assert!(!contents.contains("linksy-open"), "{file} still starts linksy-open");
        }
    }
}
