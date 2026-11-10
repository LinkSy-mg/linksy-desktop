//! Custom deck themes (docs/protocol.md §4.26): one of the five looks a phone draws as the
//! art (`base`: icon style, glyph letters, bars, covers) with the colours, shape and type a
//! user sets. The five are presets of the same data, to start from.

use serde::{Deserialize, Serialize};

use crate::deck_layout::{THEMES, is_color};

pub const MAX_THEMES: usize = 24;
pub const MAX_THEME_NAME: usize = 40;
/// The corner of a tile, in the units the built-in looks use (Stock 14, Dot Matrix 18).
pub const MAX_RADIUS: u8 = 28;
const MAX_ID: usize = 16;

/// How a tile is outlined.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Border {
    #[default]
    None,
    /// One unit, in `edge`.
    Line,
    /// Two units, in `edge`.
    Thick,
    /// Pixel's notched corners, bevels and a hard shadow on the play button.
    Bevel,
}

/// What is behind the tiles besides the colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Texture {
    #[default]
    Plain,
    /// A dot grid.
    Dots,
    /// Scanlines.
    Scan,
}

/// Where a tile's own colour goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tint {
    /// As the base look does it.
    #[default]
    Base,
    None,
    Fill,
    Icon,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Font {
    #[default]
    Sans,
    Mono,
}

/// A theme the user made. Colours are "#rrggbb".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckThemeDef {
    /// `t:` and 1 to 16 of `a-z0-9`: what a layout's theme names.
    pub id: String,
    pub name: String,
    /// One of [`THEMES`]: the art.
    pub base: String,
    pub screen: String,
    pub ink: String,
    pub tile: String,
    /// A tile that is on, and its text and icons.
    pub on: String,
    pub on_ink: String,
    /// The outline, for [`Border::Line`] and [`Border::Thick`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub edge: String,
    #[serde(default)]
    pub radius: u8,
    #[serde(default)]
    pub border: Border,
    #[serde(default)]
    pub texture: Texture,
    #[serde(default)]
    pub tint: Tint,
    #[serde(default)]
    pub font: Font,
    #[serde(default)]
    pub caps: bool,
    #[serde(default)]
    pub glow: bool,
}

/// Whether `id` looks like a custom theme's: `t:` and 1 to 16 of `a-z0-9`.
pub fn is_theme_id(id: &str) -> bool {
    id.strip_prefix("t:").is_some_and(|rest| (1..=MAX_ID).contains(&rest.len()) && rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

impl DeckThemeDef {
    /// One of the five looks as a theme to start from, with an `id` and `name` of its own.
    /// The colours are the look's at its default accent; the phone draws the built-in itself.
    pub fn from_preset(base: &str, id: &str, name: &str) -> Option<Self> {
        let theme = |screen: &str, ink: &str, tile: &str, on: &str, on_ink: &str, edge: &str, radius, border, texture, font, caps, glow| Self {
            id: id.into(),
            name: name.into(),
            base: base.into(),
            screen: screen.into(),
            ink: ink.into(),
            tile: tile.into(),
            on: on.into(),
            on_ink: on_ink.into(),
            edge: edge.into(),
            radius,
            border,
            texture,
            tint: Tint::Base,
            font,
            caps,
            glow,
        };
        Some(match base {
            "stock" => theme("#1d1d20", "#ffffff", "#2e2e32", "#3584e4", "#ffffff", "", 14, Border::None, Texture::Plain, Font::Sans, false, false),
            "dots" => theme("#000000", "#f2f2f2", "#070707", "#0b1626", "#3b8beb", "#212121", 18, Border::Line, Texture::Dots, Font::Mono, true, false),
            "eink" => theme("#e7e7e1", "#111111", "#f5f5f0", "#111111", "#f5f5f0", "#111111", 10, Border::Thick, Texture::Plain, Font::Mono, false, false),
            "term" => theme("#040904", "#39ff7a", "#071109", "#34e86f", "#041204", "#1c7839", 4, Border::Line, Texture::Plain, Font::Mono, false, true),
            "pixel" => theme("#1d2b53", "#fff1e8", "#5f574f", "#ff004d", "#fff1e8", "", 0, Border::Bevel, Texture::Scan, Font::Mono, true, false),
            _ => return None,
        })
    }

    /// Whether this can be kept and sent: an id and name that fit, a known base, colours
    /// that are colours, a corner within range.
    pub fn check(&self) -> Result<(), String> {
        if !is_theme_id(&self.id) {
            return Err(format!("a theme's id is \"t:\" and 1 to {MAX_ID} of a-z and 0-9 (\"{}\")", self.id));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > MAX_THEME_NAME {
            return Err(format!("a theme's name is 1 to {MAX_THEME_NAME} characters"));
        }
        if !THEMES.contains(&self.base.as_str()) {
            return Err(format!("theme \"{}\": no base look \"{}\"", self.name, self.base));
        }
        for (field, color) in [("screen", &self.screen), ("ink", &self.ink), ("tile", &self.tile), ("on", &self.on), ("on_ink", &self.on_ink)] {
            if !is_color(color) {
                return Err(format!("theme \"{}\": {field} is a colour like #2563ff", self.name));
            }
        }
        if !self.edge.is_empty() && !is_color(&self.edge) {
            return Err(format!("theme \"{}\": edge is a colour like #2563ff", self.name));
        }
        if self.radius > MAX_RADIUS {
            return Err(format!("theme \"{}\": a corner of at most {MAX_RADIUS}", self.name));
        }
        Ok(())
    }
}

/// Checks a whole library: each theme, and ids that differ.
pub fn check_all(themes: &[DeckThemeDef]) -> Result<(), String> {
    if themes.len() > MAX_THEMES {
        return Err(format!("at most {MAX_THEMES} themes"));
    }
    let mut ids = std::collections::HashSet::new();
    for t in themes {
        t.check()?;
        if !ids.insert(t.id.as_str()) {
            return Err(format!("two themes have the id \"{}\"", t.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sunset() -> DeckThemeDef {
        DeckThemeDef::from_preset("dots", "t:7f3a09", "Sunset").unwrap()
    }

    #[test]
    fn the_five_are_themes_to_start_from() {
        for base in THEMES {
            let t = DeckThemeDef::from_preset(base, "t:abc", "Copy").unwrap();
            assert_eq!(t.check(), Ok(()), "{base}");
            assert_eq!(t.base, *base);
        }
        assert!(DeckThemeDef::from_preset("glass", "t:abc", "Copy").is_none(), "the phone's own looks aren't bases");
    }

    #[test]
    fn ids_names_colours_and_corners_are_checked() {
        assert_eq!(sunset().check(), Ok(()));
        for id in ["", "t:", "abc", "t:ABC", "t:with space", "t:0123456789abcdef0", "stock"] {
            assert!(DeckThemeDef { id: id.into(), ..sunset() }.check().is_err(), "{id:?}");
        }
        assert!(is_theme_id("t:0123456789abcdef"));
        assert!(DeckThemeDef { name: "  ".into(), ..sunset() }.check().is_err());
        assert!(DeckThemeDef { name: "x".repeat(41), ..sunset() }.check().is_err());
        assert!(DeckThemeDef { base: "glass".into(), ..sunset() }.check().is_err());
        assert!(DeckThemeDef { screen: "red".into(), ..sunset() }.check().is_err());
        assert!(DeckThemeDef { edge: "#12".into(), ..sunset() }.check().is_err());
        assert!(DeckThemeDef { edge: String::new(), ..sunset() }.check().is_ok(), "no edge is fine");
        assert!(DeckThemeDef { radius: 29, ..sunset() }.check().is_err());
        assert!(DeckThemeDef { radius: 28, ..sunset() }.check().is_ok());
    }

    #[test]
    fn a_library_has_limits_and_distinct_ids() {
        assert_eq!(check_all(&[]), Ok(()));
        assert!(check_all(&[sunset(), sunset()]).is_err(), "the same id twice");
        let many: Vec<_> = (0..=MAX_THEMES).map(|i| DeckThemeDef { id: format!("t:{i}"), ..sunset() }).collect();
        assert!(check_all(&many).is_err());
        assert_eq!(check_all(&many[..MAX_THEMES]), Ok(()));
    }

    #[test]
    fn travels_as_the_protocol_says() {
        let json = serde_json::to_string(&sunset()).unwrap();
        assert!(json.contains(r##""on_ink":"#3b8beb""##) && json.contains(r#""border":"line""#) && json.contains(r#""texture":"dots""#), "{json}");
        assert_eq!(serde_json::from_str::<DeckThemeDef>(&json).unwrap(), sunset());
        let bare: DeckThemeDef = serde_json::from_str(r##"{"id":"t:a","name":"A","base":"stock","screen":"#000000","ink":"#ffffff","tile":"#111111","on":"#2563ff","on_ink":"#ffffff"}"##).unwrap();
        assert_eq!((bare.border, bare.texture, bare.tint, bare.font, bare.radius, bare.edge.as_str()), (Border::None, Texture::Plain, Tint::Base, Font::Sans, 0, ""));
        assert!(!serde_json::to_string(&bare).unwrap().contains("edge"), "no empty edge");
    }
}
