//! Linksy's languages, the same as Linksy for Android's: English (what the code is written
//! in), German, Spanish, French, Italian, Japanese, Malagasy, Portuguese, Russian and
//! Chinese. The translations are built in (`i18n/<code>.json`, English → translation), so
//! every install has them. The language follows the system (LANGUAGE, LC_ALL, LC_MESSAGES,
//! LANG) unless one is chosen in Preferences (`~/.config/linksy/language`); the app and the
//! service read it when they start.
//!
//! `t!("Send Files")` translates; `t!("Asking {name}…", name = n)` (or just `name` for a
//! variable of that name) translates, then fills in `{name}`. A string missing from a
//! language shows in English.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Every language: its code and its own name for itself.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("fr", "Français"),
    ("it", "Italiano"),
    ("ja", "日本語"),
    ("mg", "Malagasy"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("zh", "中文"),
];

fn catalog(code: &str) -> Option<&'static str> {
    Some(match code {
        "de" => include_str!("../i18n/de.json"),
        "es" => include_str!("../i18n/es.json"),
        "fr" => include_str!("../i18n/fr.json"),
        "it" => include_str!("../i18n/it.json"),
        "ja" => include_str!("../i18n/ja.json"),
        "mg" => include_str!("../i18n/mg.json"),
        "pt" => include_str!("../i18n/pt.json"),
        "ru" => include_str!("../i18n/ru.json"),
        "zh" => include_str!("../i18n/zh.json"),
        _ => return None,
    })
}

/// Where the chosen language is kept (none: the system's).
pub fn choice_file() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))).unwrap_or_default();
    config.join("linksy").join("language")
}

/// The language chosen in Preferences, if one is.
pub fn chosen() -> Option<String> {
    let code = std::fs::read_to_string(choice_file()).ok()?.trim().to_string();
    LANGUAGES.iter().any(|(c, _)| *c == code).then_some(code)
}

/// Choose a language (none: follow the system again). Takes effect at the next start.
pub fn choose(code: Option<&str>) -> std::io::Result<()> {
    let file = choice_file();
    match code {
        Some(code) => {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(file, code)
        }
        None => match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// The system's language, when it's one of Linksy's.
pub fn system() -> String {
    let vars = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"];
    let wanted = vars.iter().filter_map(|v| std::env::var(v).ok()).filter(|v| !v.is_empty()).flat_map(|v| v.split(':').map(str::to_string).collect::<Vec<_>>());
    for locale in wanted {
        let code = locale.split(['_', '.', '@', '-']).next().unwrap_or_default().to_lowercase();
        if code == "c" || code == "posix" {
            break;
        }
        if LANGUAGES.iter().any(|(c, _)| *c == code) {
            return code;
        }
    }
    "en".into()
}

/// The language in use.
pub fn current() -> &'static str {
    static CURRENT: OnceLock<String> = OnceLock::new();
    CURRENT.get_or_init(|| chosen().unwrap_or_else(system))
}

fn table() -> &'static HashMap<String, String> {
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| catalog(current()).and_then(|json| serde_json::from_str(json).ok()).unwrap_or_default())
}

/// `english` in the language in use.
pub fn tr(english: &'static str) -> &'static str {
    table().get(english).filter(|t| !t.is_empty()).map_or(english, String::as_str)
}

#[macro_export]
macro_rules! t {
    ($s:literal) => {
        $crate::tr($s)
    };
    ($s:literal, $($k:ident $(= $v:expr)?),+ $(,)?) => {{
        let mut out = $crate::tr($s).to_string();
        $( out = out.replace(concat!("{", stringify!($k), "}"), &$crate::t!(@value $k $(= $v)?).to_string()); )+
        out
    }};
    (@value $k:ident = $v:expr) => { $v };
    (@value $k:ident) => { $k };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_parse_and_keep_placeholders() {
        for (code, _) in LANGUAGES.iter().skip(1) {
            let map: HashMap<String, String> = serde_json::from_str(catalog(code).unwrap()).unwrap_or_else(|e| panic!("{code}: {e}"));
            for (en, tr) in &map {
                let holes = |s: &str| {
                    let mut h: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string())).collect();
                    h.sort();
                    h
                };
                assert_eq!(holes(en), holes(tr), "{code}: placeholders differ in {en:?}");
            }
        }
    }

    #[test]
    fn fills_in_names() {
        let name = "Pixel 9";
        assert_eq!(t!("Asking {name}…", name), "Asking Pixel 9…");
        assert_eq!(t!("{a} and {b}", a = 1, b = "two"), "1 and two");
        assert_eq!(t!("Plain"), "Plain");
    }
}
