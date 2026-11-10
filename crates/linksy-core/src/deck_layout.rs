//! A phone's deck layouts (docs/protocol.md §4.20): built in the desktop's editor, kept by
//! the computer, drawn by the phone. Each layout has a theme and a face per orientation, a
//! grid of pages of tiles. The rules both sides keep to live here: the kinds of tile and
//! the sizes each may take, inside the grid, no overlaps.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::deck_theme::DeckThemeDef;

pub const MAX_LAYOUTS: usize = 20;
/// Longest a tile's `icon` name is.
pub const MAX_ICON: usize = 32;
/// The names in the phone's own icon set (0.3.5), for a command's `icon` or a tile's.
pub const ICONS: &[&str] = &[
    "play_pause", "play", "pause", "previous", "next", "speaker", "speaker_off", "microphone", "microphone_off", "lock", "terminal", "lightning", "star", "moon", "sun", "house", "bell", "code", "power", "rocket", "game", "camera", "heart", "coffee",
    "music", "monitor", "broadcast", "flag", "folder", "globe", "cloud", "timer",
];
pub const MAX_PAGES: usize = 8;
pub const MAX_TILES: usize = 64;
pub const MAX_LABEL: usize = 60;
/// Largest the layouts may be as JSON, so they reach the phone in one `deck_layout`
/// message (with room for its envelope).
pub const MAX_BYTES: usize = crate::protocol::MAX_FRAME - 1024;
/// The grids a phone gets until it says its screen.
pub const PORTRAIT: [u8; 2] = [4, 8];
pub const LANDSCAPE: [u8; 2] = [9, 4];
/// The themes a phone draws.
pub const THEMES: &[&str] = &["stock", "dots", "eink", "term", "pixel"];

/// Every layout of one phone's deck.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeckLayouts {
    /// One more on every save, so the phone knows when it's behind.
    #[serde(default)]
    pub version: u64,
    /// The id of the layout the phone shows.
    #[serde(default)]
    pub active: String,
    #[serde(default)]
    pub layouts: Vec<DeckLayout>,
    /// The custom themes the layouts may name (§4.26), only on the way to a phone that
    /// draws them. The computer's own library is the one that counts, not this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<DeckThemeDef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeckLayout {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub theme: DeckTheme,
    #[serde(default = "DeckFace::portrait")]
    pub portrait: DeckFace,
    #[serde(default = "DeckFace::landscape")]
    pub landscape: DeckFace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckTheme {
    /// One of [`THEMES`]; a phone that doesn't know it draws "stock".
    pub id: String,
    /// "#rrggbb", for whatever is on.
    pub accent: String,
}

impl Default for DeckTheme {
    fn default() -> Self {
        Self { id: "stock".into(), accent: "#3584e4".into() }
    }
}

/// One orientation: its grid (columns, rows) and its pages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeckFace {
    pub grid: [u8; 2],
    pub pages: Vec<Vec<DeckTile>>,
}

impl DeckFace {
    pub fn portrait() -> Self {
        Self { grid: PORTRAIT, pages: vec![Vec::new()] }
    }

    pub fn landscape() -> Self {
        Self { grid: LANDSCAPE, pages: vec![Vec::new()] }
    }

    /// Whether a `w`×`h` tile at `x`,`y` is inside the grid and clear of `page`'s tiles
    /// (but the one with id `ignore`, being moved).
    pub fn fits(&self, page: usize, x: u8, y: u8, w: u8, h: u8, ignore: Option<&str>) -> bool {
        let [cols, rows] = self.grid;
        if w == 0 || h == 0 || x as u16 + w as u16 > cols as u16 || y as u16 + h as u16 > rows as u16 {
            return false;
        }
        self.pages.get(page).is_none_or(|tiles| {
            !tiles.iter().any(|t| Some(t.id.as_str()) != ignore && x < t.x + t.w && t.x < x + w && y < t.y + t.h && t.y < y + h)
        })
    }

    /// The first place, row by row, a `w`×`h` tile fits on `page`.
    pub fn free_spot(&self, page: usize, w: u8, h: u8) -> Option<(u8, u8)> {
        let [cols, rows] = self.grid;
        (0..=rows.checked_sub(h)?).flat_map(|y| (0..=cols.saturating_sub(w)).map(move |x| (x, y))).find(|&(x, y)| self.fits(page, x, y, w, h, None))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckTile {
    pub id: String,
    /// One of [`KINDS`]; a phone shows one it doesn't know as a plain tile with the label.
    pub kind: String,
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// "#rrggbb"; themes that colour tiles use it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// The deck action it runs (§4.19), for the kinds that run one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// What the kind needs besides: a clock's second time zone, say.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub props: BTreeMap<String, String>,
    /// A name from the phone's own icon set (§4.26), kept as it comes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
}

/// A kind of tile: the sizes it takes (width, height, the first one the default) and
/// whether it runs a deck action.
pub struct TileKind {
    pub id: &'static str,
    pub sizes: &'static [(u8, u8)],
    pub runs: bool,
}

pub const KINDS: &[TileKind] = &[
    // Any action the computer offers: media buttons, Mute, Mic Mute, Lock Screen, commands
    TileKind { id: "button", sizes: &[(1, 1), (2, 1)], runs: true },
    // sound.volume, as a slider
    TileKind { id: "volume", sizes: &[(4, 1), (2, 1), (1, 3)], runs: true },
    // The computer's player (media_state): cover, title, progress, its buttons
    TileKind { id: "music", sizes: &[(4, 2), (2, 2)], runs: false },
    // The phone's own time
    TileKind { id: "clock", sizes: &[(2, 1), (2, 2), (1, 1)], runs: false },
];

pub fn kind(id: &str) -> Option<&'static TileKind> {
    KINDS.iter().find(|k| k.id == id)
}

pub(crate) fn is_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl DeckLayouts {
    /// Whether these can be saved: known kinds at sizes they take, inside their grid and
    /// clear of each other, actions `offered` here, the five built-in themes, and within
    /// the limits.
    pub fn check(&self, offered: impl Fn(&str) -> bool) -> Result<(), String> {
        self.check_with(offered, |_| false)
    }

    /// [`check`](Self::check), with the custom themes `custom` says exist (§4.26).
    pub fn check_with(&self, offered: impl Fn(&str) -> bool, custom: impl Fn(&str) -> bool) -> Result<(), String> {
        if self.layouts.len() > MAX_LAYOUTS {
            return Err(format!("at most {MAX_LAYOUTS} layouts"));
        }
        if !self.active.is_empty() && !self.layouts.iter().any(|l| l.id == self.active) {
            return Err(format!("no layout \"{}\"", self.active));
        }
        let mut ids = HashSet::new();
        for l in &self.layouts {
            if l.id.is_empty() || !ids.insert(l.id.as_str()) {
                return Err(format!("layout ids must be there and different (\"{}\")", l.id));
            }
            if l.name.trim().is_empty() || l.name.chars().count() > MAX_LABEL {
                return Err(format!("a layout's name is 1 to {MAX_LABEL} characters"));
            }
            if !(THEMES.contains(&l.theme.id.as_str()) || custom(&l.theme.id)) || !is_color(&l.theme.accent) {
                return Err(format!("unknown theme \"{}\" or accent \"{}\"", l.theme.id, l.theme.accent));
            }
            for face in [&l.portrait, &l.landscape] {
                check_face(face, &offered)?;
            }
        }
        Ok(())
    }
}

impl DeckLayouts {
    /// What a phone gets (§4.26). Stored layouts carry no themes of their own: a phone that
    /// draws custom themes gets the `library` with them; one that doesn't gets each custom
    /// theme a layout names as its `base`. A custom id the library no longer has is Stock.
    pub fn for_phone(&self, library: &[DeckThemeDef], draws_custom: bool) -> DeckLayouts {
        let mut out = self.clone();
        out.themes = if draws_custom { library.to_vec() } else { Vec::new() };
        for l in &mut out.layouts {
            if THEMES.contains(&l.theme.id.as_str()) {
                continue;
            }
            l.theme.id = match library.iter().find(|t| t.id == l.theme.id) {
                Some(t) if draws_custom => t.id.clone(),
                Some(t) => t.base.clone(),
                None => "stock".into(),
            };
        }
        out
    }
}

fn check_face(face: &DeckFace, offered: &impl Fn(&str) -> bool) -> Result<(), String> {
    let [cols, rows] = face.grid;
    if !(1..=12).contains(&cols) || !(1..=12).contains(&rows) {
        return Err(format!("a grid of {cols} × {rows}"));
    }
    if face.pages.is_empty() || face.pages.len() > MAX_PAGES {
        return Err(format!("1 to {MAX_PAGES} pages"));
    }
    for (p, tiles) in face.pages.iter().enumerate() {
        if tiles.len() > MAX_TILES {
            return Err(format!("at most {MAX_TILES} tiles a page"));
        }
        let mut ids = HashSet::new();
        for (i, t) in tiles.iter().enumerate() {
            let k = kind(&t.kind).ok_or_else(|| format!("unknown kind of tile \"{}\"", t.kind))?;
            if t.id.is_empty() || !ids.insert(t.id.as_str()) {
                return Err(format!("tile ids must be there and different (\"{}\")", t.id));
            }
            if !k.sizes.contains(&(t.w, t.h)) {
                return Err(format!("a {} can't be {} × {}", t.kind, t.w, t.h));
            }
            if t.label.chars().count() > MAX_LABEL || (!t.color.is_empty() && !is_color(&t.color)) {
                return Err(format!("tile \"{}\": a label of at most {MAX_LABEL} characters, a colour like #2563ff", t.id));
            }
            if t.icon.len() > MAX_ICON || !t.icon.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                return Err(format!("tile \"{}\": an icon is up to {MAX_ICON} of a-z, 0-9 and _", t.id));
            }
            match (&t.action, k.runs) {
                (Some(a), true) if offered(a) => {}
                (Some(a), true) => return Err(format!("this computer doesn't offer \"{a}\"")),
                (None, true) => return Err(format!("a {} needs an action", t.kind)),
                (_, false) => {}
            }
            // Inside the grid, and clear of the tiles before it
            let earlier = DeckFace { grid: face.grid, pages: vec![tiles[..i].to_vec()] };
            if !earlier.fits(0, t.x, t.y, t.w, t.h, None) {
                return Err(format!("tile \"{}\" is outside the grid or over another on page {}", t.id, p + 1));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(id: &str, kind: &str, x: u8, y: u8, w: u8, h: u8, action: Option<&str>) -> DeckTile {
        DeckTile { id: id.into(), kind: kind.into(), x, y, w, h, action: action.map(Into::into), ..Default::default() }
    }

    fn one(tiles: Vec<DeckTile>) -> DeckLayouts {
        let mut portrait = DeckFace::portrait();
        portrait.pages[0] = tiles;
        DeckLayouts { version: 1, active: "work".into(), layouts: vec![DeckLayout { id: "work".into(), name: "Work".into(), theme: DeckTheme::default(), portrait, landscape: DeckFace::landscape() }], themes: Vec::new() }
    }

    #[test]
    fn fits_and_free_spots() {
        let mut face = DeckFace::portrait();
        face.pages[0].push(tile("a", "music", 0, 0, 4, 2, None));
        assert!(!face.fits(0, 0, 1, 1, 1, None), "over the music");
        assert!(face.fits(0, 0, 1, 1, 1, Some("a")), "moving the music itself");
        assert!(!face.fits(0, 3, 7, 2, 1, None), "past the right edge");
        assert_eq!(face.free_spot(0, 2, 1), Some((0, 2)));
        assert_eq!(face.free_spot(0, 5, 1), None);
    }

    #[test]
    fn checks_what_the_phone_would_draw() {
        let offered = |a: &str| a.starts_with("media.") || a == "sound.volume";
        assert_eq!(one(vec![tile("a", "music", 0, 0, 4, 2, None), tile("b", "button", 0, 2, 1, 1, Some("media.next"))]).check(offered), Ok(()));
        assert!(one(vec![tile("a", "music", 0, 0, 3, 2, None)]).check(offered).is_err(), "a size music doesn't take");
        assert!(one(vec![tile("a", "music", 0, 0, 4, 2, None), tile("b", "clock", 1, 1, 2, 1, None)]).check(offered).is_err(), "overlap");
        assert!(one(vec![tile("a", "button", 0, 0, 1, 1, Some("system.lock"))]).check(offered).is_err(), "not offered");
        assert!(one(vec![tile("a", "button", 0, 0, 1, 1, None)]).check(offered).is_err(), "a button without an action");
        assert!(one(vec![tile("a", "weather", 0, 0, 1, 1, None)]).check(offered).is_err(), "unknown kind");
        let mut themed = one(vec![]);
        themed.layouts[0].theme.id = "glass".into();
        assert!(themed.check(offered).is_err());
    }

    #[test]
    fn custom_themes_and_icons() {
        let none = |_: &str| true;
        let mut deck = one(vec![DeckTile { icon: "terminal".into(), ..tile("a", "button", 0, 0, 1, 1, Some("media.next")) }]);
        assert_eq!(deck.check(none), Ok(()), "an icon of the phone's set is kept as it comes");
        deck.layouts[0].theme = DeckTheme { id: "t:7f3a09".into(), accent: "#ff7a59".into() };
        assert!(deck.check(none).is_err(), "a custom theme nobody has");
        assert_eq!(deck.check_with(none, |id| id == "t:7f3a09"), Ok(()));
        assert!(deck.check_with(none, |id| id == "t:other").is_err());
        for icon in ["Terminal", "a b", "x".repeat(MAX_ICON + 1).as_str()] {
            let bad = one(vec![DeckTile { icon: icon.into(), ..tile("a", "button", 0, 0, 1, 1, Some("media.next")) }]);
            assert!(bad.check(none).is_err(), "{icon:?}");
        }
    }

    #[test]
    fn a_phone_gets_the_themes_it_can_draw() {
        let sunset = DeckThemeDef::from_preset("dots", "t:sun", "Sunset").unwrap();
        let mut deck = one(vec![]);
        let named = |id: &str| DeckLayout { id: id.into(), name: id.into(), theme: DeckTheme { id: id.into(), accent: "#ff7a59".into() }, ..deck.layouts[0].clone() };
        deck.layouts = vec![named("t:sun"), named("pixel"), named("t:gone")];
        let library = vec![sunset.clone()];
        let ids = |d: &DeckLayouts| d.layouts.iter().map(|l| l.theme.id.clone()).collect::<Vec<_>>();

        let modern = deck.for_phone(&library, true);
        assert_eq!((ids(&modern), modern.themes.clone()), (vec!["t:sun".into(), "pixel".into(), "stock".into()], library.clone()));
        let older = deck.for_phone(&library, false);
        assert_eq!((ids(&older), older.themes.is_empty()), (vec!["dots".into(), "pixel".into(), "stock".into()], true));
        assert_eq!(ids(&deck), ["t:sun", "pixel", "t:gone"], "what is stored is left as it was");
        assert!(deck.themes.is_empty());
    }

    #[test]
    fn themes_travel_only_when_there_are_some() {
        let mut deck = one(vec![]);
        assert!(!serde_json::to_string(&deck).unwrap().contains("themes"));
        deck.themes.push(DeckThemeDef::from_preset("stock", "t:abc", "Mine").unwrap());
        let json = serde_json::to_string(&deck).unwrap();
        assert!(json.contains(r#""themes":[{"id":"t:abc""#), "{json}");
        assert_eq!(serde_json::from_str::<DeckLayouts>(&json).unwrap(), deck);
        let old: DeckLayouts = serde_json::from_str(r#"{"version":2,"active":"a","layouts":[]}"#).unwrap();
        assert!(old.themes.is_empty(), "layouts kept before themes still load");
    }

    #[test]
    fn travels_as_json() {
        let deck = one(vec![tile("a", "clock", 0, 0, 2, 1, None)]);
        let json = serde_json::to_string(&deck).unwrap();
        assert!(json.contains(r##""theme":{"id":"stock","accent":"#3584e4"}"##));
        assert!(!json.contains("action"), "no empty fields");
        assert_eq!(serde_json::from_str::<DeckLayouts>(&json).unwrap(), deck);
        let old: DeckLayout = serde_json::from_str(r#"{"id":"x","name":"X"}"#).unwrap();
        assert_eq!((old.portrait.grid, old.landscape.grid, old.theme.id.as_str()), (PORTRAIT, LANDSCAPE, "stock"));
    }
}
