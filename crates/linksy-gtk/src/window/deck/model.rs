//! The deck editor's rules, apart from GTK: the widget library made from what this computer
//! offers, the sizes a drag can snap to, where a drop lands, a face reflowed to another
//! grid, new ids, the Pixel theme's palette and the 5×7 glyphs some themes draw text with.

use std::collections::HashSet;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use linksy_i18n::t;
use linksy_ipc::DeckPrefs;
use linksy_ipc::linksy_core::deck_layout::{self, MAX_PAGES};
use linksy_ipc::linksy_core::{DeckFace, DeckKind, DeckLayout, DeckLayouts, DeckTheme, DeckTile};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Stock,
    Dots,
    Eink,
    Term,
    Pixel,
}

impl Theme {
    pub const ALL: [Theme; 5] = [Theme::Stock, Theme::Dots, Theme::Eink, Theme::Term, Theme::Pixel];

    pub fn id(self) -> &'static str {
        match self {
            Theme::Stock => "stock",
            Theme::Dots => "dots",
            Theme::Eink => "eink",
            Theme::Term => "term",
            Theme::Pixel => "pixel",
        }
    }

    /// A theme the editor doesn't know shows as Stock, as on the phone.
    pub fn from_id(id: &str) -> Self {
        Self::ALL.into_iter().find(|t| t.id() == id).unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Stock => t!("Stock"),
            Theme::Dots => t!("Dot Matrix"),
            Theme::Eink => t!("E-Ink"),
            Theme::Term => t!("Terminal"),
            Theme::Pixel => t!("Pixel"),
        }
    }

    /// Whether its text is drawn in 5×7 glyphs (dots or squares).
    pub fn glyphs(self) -> bool {
        matches!(self, Theme::Dots | Theme::Pixel)
    }
}

/// Tile colours to pick from, and accents for what's on.
pub const COLORS: [&str; 10] = ["#2563ff", "#22c3e6", "#0f766e", "#14532d", "#a16207", "#9a3412", "#be123c", "#3b2a63", "#24345c", "#334155"];
pub const ACCENTS: [&str; 6] = ["#3584e4", "#ff3b30", "#ffd400", "#39ff7a", "#22c3e6", "#ff7a59"];

/// The groups of the library, in order; others the service may send come after.
pub const GROUPS: [&str; 7] = ["Media", "Sound", "Time", "System", "Network", "Power", "Commands"];

/// Something the library offers: a kind of tile, with the action it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: &'static str,
    pub action: Option<String>,
    pub name: String,
    pub hint: String,
    pub group: String,
    /// Its data comes from the phone, not the computer.
    pub phone: bool,
    /// One of the user's commands, by id: it can be removed from the library.
    pub command: Option<String>,
}

impl Item {
    pub fn icon(&self) -> &'static str {
        match self.kind {
            "music" => "music",
            "clock" => "clock",
            "volume" => "volume",
            _ => icon_for(self.action.as_deref(), false),
        }
    }

    pub fn color(&self) -> &'static str {
        default_color(self.kind, self.action.as_deref())
    }
}

/// The library: the app's own widgets and one per action this computer offers, grouped.
pub fn library(prefs: &DeckPrefs) -> Vec<(String, Vec<Item>)> {
    let mut groups: Vec<(String, Vec<Item>)> = GROUPS.iter().map(|g| (g.to_string(), Vec::new())).collect();
    let mut push = |group: &str, item: Item| match groups.iter_mut().find(|(g, _)| g == group) {
        Some((_, items)) => items.push(item),
        None => groups.push((group.to_string(), vec![item])),
    };
    let own = |kind, name: &str, hint: &str, group: &str, phone| Item { kind, action: None, name: name.into(), hint: hint.into(), group: group.into(), phone, command: None };
    push("Media", own("music", t!("Music Player"), t!("The computer's player"), "Media", false));
    push("Time", own("clock", t!("Clock"), t!("Time and date"), "Time", true));
    for option in &prefs.actions {
        let a = &option.action;
        let command = a.id.strip_prefix("command.").map(String::from);
        let hint = match command.as_ref().and_then(|id| prefs.commands.iter().find(|c| c.id == *id)) {
            Some(c) => c.command.clone(),
            None => a.id.clone(),
        };
        let group = match (a.group.as_str(), &command) {
            ("", Some(_)) => "Commands",
            ("", None) => "System",
            (g, _) => g,
        };
        let kind = if a.kind == DeckKind::Slider { "volume" } else { "button" };
        push(group, Item { kind, action: Some(a.id.clone()), name: a.name.clone(), hint, group: group.into(), phone: false, command });
    }
    // Your Commands shows even with none, for its Add Command row
    groups.retain(|(g, items)| !items.is_empty() || g == "Commands");
    groups
}

pub fn matches(item: &Item, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || item.name.to_lowercase().contains(&q) || item.hint.to_lowercase().contains(&q)
}

/// The library's item for a tile, for its name and hint.
pub fn item_for<'a>(library: &'a [(String, Vec<Item>)], tile: &DeckTile) -> Option<&'a Item> {
    library.iter().flat_map(|(_, items)| items).find(|i| i.kind == tile.kind && i.action == tile.action)
}

/// The sizes `kind` takes that fit in `grid`, its default first.
pub fn sizes(kind: &str, grid: [u8; 2]) -> Vec<(u8, u8)> {
    deck_layout::kind(kind).map(|k| k.sizes.iter().copied().filter(|&(w, h)| w <= grid[0] && h <= grid[1]).collect()).unwrap_or_default()
}

/// A grid's cells in pixels: their side and the gap between them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub cell: f64,
    pub gap: f64,
}

impl Metrics {
    /// The biggest square cells `grid` fits in `width`×`height`.
    pub fn fit(width: f64, height: f64, grid: [u8; 2]) -> Self {
        let [cols, rows] = grid.map(|n| n.max(1) as f64);
        let gap = if rows > cols { 8.0 } else { 6.0 };
        let cell = ((width - gap * (cols - 1.0)) / cols).min((height - gap * (rows - 1.0)) / rows).floor().max(8.0);
        Self { cell, gap }
    }

    pub fn step(self) -> f64 {
        self.cell + self.gap
    }

    /// Where cell `n` starts.
    pub fn at(self, n: u8) -> f64 {
        n as f64 * self.step()
    }

    /// How long `n` cells are, with the gaps between them.
    pub fn span(self, n: u8) -> f64 {
        n as f64 * self.cell + (n.max(1) - 1) as f64 * self.gap
    }
}

/// Which edge of a picked tile is being pulled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Right,
    Bottom,
    Corner,
}

/// The size, in cells and fractions of one, a tile of `size` is pulled to by `dx`,`dy` pixels.
pub fn wanted(size: (u8, u8), edge: Edge, dx: f64, dy: f64, m: Metrics) -> (f64, f64) {
    let w = size.0 as f64 + if edge == Edge::Bottom { 0.0 } else { dx / m.step() };
    let h = size.1 as f64 + if edge == Edge::Right { 0.0 } else { dy / m.step() };
    (w, h)
}

/// The option nearest to `want`; the earlier one on a tie.
pub fn nearest(options: &[(u8, u8)], want: (f64, f64)) -> Option<(u8, u8)> {
    let dist = |(w, h): (u8, u8)| (w as f64 - want.0).hypot(h as f64 - want.1);
    options.iter().copied().fold(None, |best: Option<(u8, u8)>, o| match best {
        Some(b) if dist(b) <= dist(o) => Some(b),
        _ => Some(o),
    })
}

/// Where a drag from the library holds the tile, from its top left corner.
pub const HOTSPOT: f64 = 20.0;

/// The cell a `size` tile dropped at `x`,`y` (pixels from the grid's corner) lands on: the
/// one its corner is nearest, kept inside the grid.
pub fn drop_cell(m: Metrics, grid: [u8; 2], size: (u8, u8), x: f64, y: f64) -> (u8, u8) {
    let cell = |p: f64, n: u8, s: u8| ((p - HOTSPOT + m.cell / 2.0) / m.step()).floor().clamp(0.0, n.saturating_sub(s) as f64) as u8;
    (cell(x, grid[0], size.0), cell(y, grid[1], size.1))
}

/// The cell a tile at `from` is moved to by `dx`,`dy` pixels, kept inside the grid.
pub fn moved_cell(m: Metrics, grid: [u8; 2], from: (u8, u8), size: (u8, u8), dx: f64, dy: f64) -> (u8, u8) {
    let cell = |at: u8, d: f64, n: u8, s: u8| (at as f64 + (d / m.step()).round()).clamp(0.0, n.saturating_sub(s) as f64) as u8;
    (cell(from.0, dx, grid[0], size.0), cell(from.1, dy, grid[1], size.1))
}

/// Where a new tile goes on `page`: at `at` if it fits there, else the first free spot, trying
/// each of `sizes` in turn. None: the page is full.
pub fn place(face: &DeckFace, page: usize, sizes: &[(u8, u8)], at: Option<(u8, u8)>) -> Option<(u8, u8, u8, u8)> {
    sizes.iter().find_map(|&(w, h)| {
        let here = at.map(|(x, y)| (x.min(face.grid[0].saturating_sub(w)), y.min(face.grid[1].saturating_sub(h)))).filter(|&(x, y)| face.fits(page, x, y, w, h, None));
        here.or_else(|| face.free_spot(page, w, h)).map(|(x, y)| (x, y, w, h))
    })
}

/// `from`'s tiles in their order (page by page, row by row) on a face of `grid`: each keeps
/// its size where the kind allows it there and goes on the last page with room, a new page
/// when none is left. What doesn't fit in [`MAX_PAGES`] is left out.
pub fn reflow(from: &DeckFace, grid: [u8; 2], mut new_id: impl FnMut() -> String) -> DeckFace {
    let mut out = DeckFace { grid, pages: vec![Vec::new()] };
    for page in &from.pages {
        let mut tiles: Vec<&DeckTile> = page.iter().collect();
        tiles.sort_by_key(|t| (t.y, t.x));
        for t in tiles {
            let mut options = sizes(&t.kind, grid);
            if let Some(i) = options.iter().position(|&s| s == (t.w, t.h)) {
                options.swap(0, i);
            }
            let last = out.pages.len() - 1;
            let spot = place(&out, last, &options, None).map(|s| (last, s)).or_else(|| {
                (out.pages.len() < MAX_PAGES).then(|| out.pages.push(Vec::new()))?;
                place(&out, last + 1, &options, None).map(|s| (last + 1, s))
            });
            if let Some((p, (x, y, w, h))) = spot {
                out.pages[p].push(DeckTile { id: new_id(), x, y, w, h, ..t.clone() });
            }
        }
    }
    if out.pages.len() > 1 && out.pages.last().is_some_and(Vec::is_empty) {
        out.pages.pop();
    }
    out
}

/// A short random hex id that `taken` says is free.
pub fn new_id(taken: impl Fn(&str) -> bool) -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    loop {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(COUNT.fetch_add(1, Ordering::Relaxed));
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default());
        let id = format!("{:08x}", h.finish() as u32);
        if !taken(&id) {
            return id;
        }
    }
}

/// Every id in use: the layouts' and their tiles'.
pub fn ids(layouts: &DeckLayouts) -> HashSet<String> {
    let mut out = HashSet::new();
    for l in &layouts.layouts {
        out.insert(l.id.clone());
        for face in [&l.portrait, &l.landscape] {
            out.extend(face.pages.iter().flatten().map(|t| t.id.clone()));
        }
    }
    out
}

/// A first deck, for a phone that has none: the clock, the music, the volume and the media
/// buttons, of what this computer offers.
pub fn starter(prefs: &DeckPrefs, name: &str) -> DeckLayouts {
    let offered = |id: &str| prefs.actions.iter().any(|o| o.action.id == id);
    let mut face = DeckFace::portrait();
    let mut taken = HashSet::new();
    let mut add = |face: &mut DeckFace, kind: &str, action: Option<&str>, label: &str, x, y, w, h| {
        if action.is_some_and(|a| !offered(a)) || !face.fits(0, x, y, w, h, None) {
            return;
        }
        let id = new_id(|id| taken.contains(id));
        taken.insert(id.clone());
        let tile = DeckTile { id, kind: kind.into(), x, y, w, h, label: label.into(), color: default_color(kind, action).into(), action: action.map(Into::into), ..Default::default() };
        face.pages[0].push(tile);
    };
    let name_of = |id: &str| prefs.actions.iter().find(|o| o.action.id == id).map(|o| o.action.name.clone()).unwrap_or_default();
    add(&mut face, "clock", None, t!("Clock"), 0, 0, 2, 1);
    add(&mut face, "button", Some("sound.mute"), &name_of("sound.mute"), 2, 0, 1, 1);
    add(&mut face, "button", Some("sound.mic_mute"), &name_of("sound.mic_mute"), 3, 0, 1, 1);
    add(&mut face, "music", None, t!("Music Player"), 0, 1, 4, 2);
    add(&mut face, "volume", Some("sound.volume"), &name_of("sound.volume"), 0, 3, 4, 1);
    for (x, a) in ["media.previous", "media.play_pause", "media.next"].into_iter().enumerate() {
        add(&mut face, "button", Some(a), &name_of(a), x as u8, 4, 1, 1);
    }
    let id = new_id(|id| taken.contains(id));
    DeckLayouts { version: 0, active: id.clone(), layouts: vec![DeckLayout { id, name: name.into(), theme: DeckTheme::default(), portrait: face, landscape: DeckFace::landscape() }] }
}

/// A tile's colour when it has none of its own.
pub fn default_color(kind: &str, action: Option<&str>) -> &'static str {
    match (kind, action.and_then(|a| a.split_once('.')).map(|(group, _)| group)) {
        ("music", _) => "#3b2a63",
        ("clock", _) => "#0f766e",
        ("volume", _) => "#1d4fd8",
        (_, Some("media")) => "#24345c",
        (_, Some("sound")) => "#2b3550",
        (_, Some("command")) => "#14532d",
        (_, Some("network")) => "#155e75",
        (_, Some("power")) => "#a16207",
        _ => "#334155",
    }
}

/// The 16-colour console palette the Pixel theme keeps its tiles to (but the background's
/// navy, black and white).
pub const PICO: [&str; 13] = ["#7e2553", "#008751", "#ab5236", "#5f574f", "#c2c3c7", "#ff004d", "#ffa300", "#ffec27", "#00e436", "#29adff", "#83769c", "#ff77a8", "#ffccaa"];

fn rgb(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.strip_prefix('#').filter(|h| h.len() == 6)?;
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The palette's nearest colour to `hex` (its index in [`PICO`]), and whether dark ink reads
/// better on it.
pub fn pico(hex: &str) -> (usize, bool) {
    let [r, g, b] = rgb(hex).unwrap_or([0x33, 0x41, 0x55]).map(i32::from);
    let dist = |p: &str| rgb(p).map(|[pr, pg, pb]| (r - pr as i32).pow(2) + (g - pg as i32).pow(2) + (b - pb as i32).pow(2)).unwrap_or(i32::MAX);
    let i = (0..PICO.len()).min_by_key(|&i| dist(PICO[i])).unwrap_or(0);
    let [pr, pg, pb] = rgb(PICO[i]).unwrap_or_default().map(f64::from);
    (i, 0.299 * pr + 0.587 * pg + 0.114 * pb > 160.0)
}

/// The Pixel theme's music cover, 10×10, row by row.
pub fn pixel_art() -> [[&'static str; 10]; 10] {
    let sky = ["#29adff", "#83769c", "#ff77a8"];
    let mut out = [[""; 10]; 10];
    for (y, row) in out.iter_mut().enumerate() {
        for (x, px) in row.iter_mut().enumerate() {
            let (fx, fy) = (x as f64, y as f64);
            let sun = (fx - 6.5).powi(2) + (fy - 3.5).powi(2) < 5.0;
            let hill = fy > 6.0 + (fx.sin() * 0.8).round();
            *px = match (sun, hill) {
                (true, _) => "#ffec27",
                (false, true) if y > 8 => "#1d2b53",
                (false, true) => "#7e2553",
                _ => sky[((fy / 2.4).floor() as usize).min(2)],
            };
        }
    }
    out
}

/// Icons as 24×24 stroked paths, drawn the same in every theme: solid, dotted or in pixels.
pub const ICONS: &[(&str, &str)] = &[
    ("music", "M9 18V5l11-2v13M9 18a3 3 0 1 1-6 0a3 3 0 0 1 6 0zM20 16a3 3 0 1 1-6 0a3 3 0 0 1 6 0z"),
    ("play", "M8 5l11 7-11 7z"),
    ("pause", "M7 5v14M17 5v14"),
    ("next", "M6 6l9 6-9 6zM18 6v12"),
    ("prev", "M18 6l-9 6 9 6zM6 6v12"),
    ("volume", "M4 9h4l5-4v14l-5-4H4zM16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12"),
    ("mute", "M4 9h4l5-4v14l-5-4H4zM17 9l4 6M21 9l-4 6"),
    ("mic", "M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM5 11a7 7 0 0 0 14 0M12 18v3"),
    ("output", "M3 9h3l4-4v14l-4-4H3zM14 12h7M18 9l3 3-3 3"),
    ("clock", "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM12 7v5l3 2"),
    ("lock", "M6 11h12v9H6zM8 11V8a4 4 0 0 1 8 0v3"),
    ("restart", "M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5"),
    ("power", "M12 3v8M6.4 6.4a8 8 0 1 0 11.2 0"),
    ("moon", "M20 14A8 8 0 1 1 10 4a6 6 0 0 0 10 10z"),
    ("wifi", "M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.5 16a5 5 0 0 1 7 0M12 19.5v.5"),
    ("bluetooth", "M7 7l10 10-5 4V3l5 4L7 17"),
    ("airplane", "M12 3c-.8 0-1.3.8-1.3 1.8V9.5L3 14v2.5l7.7-2.4v4.4L8.5 20v1.5l3.5-1 3.5 1V20l-2.2-1.5v-4.4l7.7 2.4V14l-7.7-4.5V4.8C13.3 3.8 12.8 3 12 3z"),
    ("leaf", "M5 19C4 11 9 4 20 4c0 11-6 16-15 15zM5 19l8-8"),
    ("balanced", "M12 4v16M8 20h8M4 8h16M4 8l-2.5 6h5zM20 8l-2.5 6h5z"),
    ("gauge", "M4 18a8 8 0 1 1 16 0M12 18l4-6M12 8v1M7 11l.7.7M17 11l-.7.7"),
    ("auto", "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM8.5 16l3.5-9 3.5 9M9.7 13h4.6"),
    ("terminal", "M4 5h16v14H4zM7 9l3 3-3 3M12 15h5"),
    ("app", "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z"),
    ("x", "M7 7l10 10M17 7L7 17"),
];

pub fn icon_path(name: &str) -> &'static str {
    ICONS.iter().find(|(n, _)| *n == name).map(|(_, p)| *p).unwrap_or(ICONS[ICONS.len() - 2].1)
}

/// The icon of a tile that runs `action`; `playing` turns Play or Pause into pause.
pub fn icon_for(action: Option<&str>, playing: bool) -> &'static str {
    let Some(action) = action else { return "app" };
    match action {
        "media.play_pause" if playing => "pause",
        "media.play_pause" => "play",
        "media.next" => "next",
        "media.previous" => "prev",
        "sound.volume" => "volume",
        "sound.mute" => "mute",
        "sound.mic_mute" => "mic",
        "sound.output" => "output",
        "system.lock" => "lock",
        "system.restart" => "restart",
        "system.shutdown" => "power",
        "system.dnd" => "moon",
        "network.wifi" => "wifi",
        "network.bluetooth" => "bluetooth",
        "network.airplane" => "airplane",
        "power.saver" => "leaf",
        "power.balanced" => "balanced",
        "power.performance" => "gauge",
        "power.auto" => "auto",
        a if a.starts_with("command.") => "terminal",
        _ => "app",
    }
}

/// `c` as the glyphs have it: upper case, without its accent.
pub fn fold(c: char) -> char {
    let c = c.to_uppercase().next().unwrap_or(c);
    match c {
        'À'..='Å' => 'A',
        'Ç' => 'C',
        'È'..='Ë' => 'E',
        'Ì'..='Ï' => 'I',
        'Ñ' => 'N',
        'Ò'..='Ö' | 'Ø' => 'O',
        'Ù'..='Ü' => 'U',
        'Ý' | 'Ÿ' => 'Y',
        '×' => 'X',
        '…' => '.',
        '–' | '—' => '-',
        '’' | '‘' => '\'',
        c => c,
    }
}

/// A character's 5×7 glyph, a row of five bits a line, the left one highest.
pub fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match fold(c) {
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1f, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
        '6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
        'A' => [0x0e, 0x11, 0x11, 0x11, 0x1f, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e],
        'D' => [0x1c, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1c],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0e, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x0e, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0c],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'Q' => [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a],
        'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0a, 0x04, 0x04, 0x04],
        'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
        ' ' => [0; 7],
        ':' => [0x00, 0x0c, 0x0c, 0x00, 0x0c, 0x0c, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x0c],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0c, 0x04, 0x08],
        '-' => [0x00, 0x00, 0x00, 0x1f, 0x00, 0x00, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1f, 0x04, 0x04, 0x00],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '/' => [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x00],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '?' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '\'' => [0x0c, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00],
        '&' => [0x0c, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0d],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '#' => [0x0a, 0x0a, 0x1f, 0x0a, 0x1f, 0x0a, 0x0a],
        '=' => [0x00, 0x00, 0x1f, 0x00, 0x1f, 0x00, 0x00],
        '*' => [0x00, 0x04, 0x15, 0x0e, 0x15, 0x04, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1f],
        '·' => [0x00, 0x00, 0x00, 0x0c, 0x0c, 0x00, 0x00],
        _ => return None,
    })
}

/// Whether every character of `s` has a glyph; text that doesn't shows in a font instead.
pub fn has_glyphs(s: &str) -> bool {
    s.chars().all(|c| glyph(c).is_some())
}

/// `text` in lines of at most `per_line` glyphs, broken at spaces where it can, at most
/// `lines` of them; what's cut ends in a full stop.
pub fn wrap(text: &str, per_line: usize, lines: usize) -> Vec<String> {
    let per_line = per_line.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        loop {
            let len = line.chars().count();
            let room = if len == 0 { per_line } else { per_line.saturating_sub(len + 1) };
            if word.len() <= room {
                if len > 0 {
                    line.push(' ');
                }
                line.extend(word.iter());
                break;
            }
            if len == 0 {
                line.extend(word.drain(..per_line));
            }
            out.push(std::mem::take(&mut line));
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.len() > lines {
        out.truncate(lines.max(1));
        let last = out.last_mut().expect("a line");
        let mut chars: Vec<char> = last.chars().collect();
        chars.truncate(per_line.saturating_sub(1));
        while chars.last() == Some(&' ') {
            chars.pop();
        }
        chars.push('.');
        *last = chars.into_iter().collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(json: &str) -> DeckPrefs {
        serde_json::from_str(json).unwrap()
    }

    fn full() -> DeckPrefs {
        prefs(
            r#"{"actions":[
            {"id":"media.play_pause","name":"Play or Pause","kind":"button","group":"Media","on":true},
            {"id":"sound.volume","name":"Volume","kind":"slider","group":"Sound","on":true},
            {"id":"sound.mute","name":"Mute","kind":"toggle","group":"Sound","on":true},
            {"id":"network.wifi","name":"Wi-Fi","kind":"toggle","group":"Network","on":false},
            {"id":"system.lock","name":"Lock Screen","kind":"button","group":"System","on":false},
            {"id":"power.saver","name":"Power Saver","kind":"toggle","group":"Power","on":false},
            {"id":"lights.on","name":"Lights","kind":"button","group":"Home","on":false},
            {"id":"command.ab12","name":"Back Up","kind":"button","group":"Commands","on":false}],
            "commands":[{"id":"ab12","name":"Back Up","command":"rsync -a ~/p /mnt"}]}"#,
        )
    }

    #[test]
    fn the_library_is_what_the_computer_offers() {
        let lib = library(&full());
        let groups: Vec<&str> = lib.iter().map(|(g, _)| g.as_str()).collect();
        assert_eq!(groups, ["Media", "Sound", "Time", "System", "Network", "Power", "Commands", "Home"]);
        let kinds = |g: &str| lib.iter().find(|(n, _)| n == g).unwrap().1.iter().map(|i| (i.kind, i.action.clone())).collect::<Vec<_>>();
        assert_eq!(kinds("Media"), [("music", None), ("button", Some("media.play_pause".into()))]);
        assert_eq!(kinds("Sound"), [("volume", Some("sound.volume".into())), ("button", Some("sound.mute".into()))]);
        let command = &lib.iter().find(|(n, _)| n == "Commands").unwrap().1[0];
        assert_eq!((command.command.as_deref(), command.hint.as_str()), (Some("ab12"), "rsync -a ~/p /mnt"));
        assert!(lib.iter().flat_map(|(_, i)| i).all(|i| i.phone == (i.kind == "clock")));

        // Without wpctl or pactl the computer offers no sound, so no Volume
        let quiet = library(&prefs(r#"{"actions":[{"id":"media.next","name":"Next","kind":"button","group":"Media","on":true}],"commands":[]}"#));
        assert!(quiet.iter().all(|(g, _)| g != "Sound"));
        assert!(quiet.iter().any(|(g, items)| g == "Commands" && items.is_empty()), "Your Commands stays, to add one");
        let wifi = lib.iter().flat_map(|(_, i)| i).find(|i| i.action.as_deref() == Some("network.wifi")).unwrap();
        assert!(matches(wifi, "wi-") && matches(wifi, " NETWORK.") && matches(wifi, "") && !matches(wifi, "volume"));
    }

    #[test]
    fn a_resize_snaps_to_the_nearest_size() {
        let volume = [(4, 1), (2, 1), (1, 3)];
        let m = Metrics { cell: 58.0, gap: 8.0 };
        assert_eq!(nearest(&volume, wanted((4, 1), Edge::Right, -132.0, 300.0, m)), Some((2, 1)), "the right edge ignores the height");
        assert_eq!(nearest(&volume, wanted((4, 1), Edge::Corner, -200.0, 140.0, m)), Some((1, 3)));
        assert_eq!(nearest(&volume, wanted((2, 1), Edge::Bottom, 400.0, 0.0, m)), Some((2, 1)), "the bottom edge ignores the width");
        assert_eq!(nearest(&[(2, 1), (1, 2)], (1.5, 1.5)), Some((2, 1)), "the earlier on a tie");
        assert_eq!(nearest(&[], (1.0, 1.0)), None);
    }

    #[test]
    fn a_drop_lands_under_the_pointer() {
        let m = Metrics { cell: 58.0, gap: 8.0 };
        let grid = [4, 8];
        // The tile is held 20 px in from its corner
        assert_eq!(drop_cell(m, grid, (1, 1), 20.0, 20.0), (0, 0));
        assert_eq!(drop_cell(m, grid, (1, 1), 66.0 + 20.0 + 30.0, 2.0 * 66.0 + 20.0), (1, 2), "its corner nearer cell 1");
        assert_eq!(drop_cell(m, grid, (1, 1), 2.0 * 66.0 - 10.0 + 20.0, 20.0), (2, 0), "its corner nearer cell 2");
        assert_eq!(drop_cell(m, grid, (2, 1), 400.0, 900.0), (2, 7), "kept inside the grid");
        assert_eq!(drop_cell(m, grid, (4, 2), -50.0, -50.0), (0, 0));
        assert_eq!(moved_cell(m, grid, (1, 1), (1, 1), 40.0, -40.0), (2, 0));
        assert_eq!(moved_cell(m, grid, (1, 1), (2, 1), 500.0, 20.0), (2, 1));

        let mut face = DeckFace::portrait();
        face.pages[0].push(DeckTile { id: "a".into(), kind: "music".into(), x: 0, y: 0, w: 4, h: 2, ..Default::default() });
        assert_eq!(place(&face, 0, &[(1, 1)], Some((3, 4))), Some((3, 4, 1, 1)));
        assert_eq!(place(&face, 0, &[(2, 1)], Some((1, 1))), Some((0, 2, 2, 1)), "over the music: the first free spot");
        assert_eq!(place(&face, 0, &[(4, 2), (2, 2)], Some((3, 7))), Some((0, 6, 4, 2)), "pulled back inside the grid");
        face.pages[0].push(DeckTile { id: "b".into(), kind: "music".into(), x: 0, y: 2, w: 4, h: 6, ..Default::default() });
        assert_eq!(place(&face, 0, &[(1, 1)], None), None, "a full page");
    }

    #[test]
    fn portrait_reflows_to_landscape() {
        let tile = |id: &str, kind: &str, x, y, w, h| DeckTile { id: id.into(), kind: kind.into(), x, y, w, h, ..Default::default() };
        let mut portrait = DeckFace::portrait();
        portrait.pages[0] = vec![tile("v", "volume", 0, 3, 1, 3), tile("c", "clock", 0, 0, 2, 1), tile("m", "music", 0, 1, 4, 2)];
        portrait.pages.push((0..8).map(|i| tile(&format!("b{i}"), "button", i % 4, i / 4, 1, 1)).collect());
        let mut n = 0;
        let land = reflow(&portrait, [9, 4], || {
            n += 1;
            format!("n{n}")
        });
        assert_eq!(land.grid, [9, 4]);
        let first: Vec<(&str, u8, u8, u8, u8)> = land.pages[0].iter().map(|t| (t.kind.as_str(), t.x, t.y, t.w, t.h)).collect();
        assert_eq!(first[..3], [("clock", 0, 0, 2, 1), ("music", 2, 0, 4, 2), ("volume", 6, 0, 1, 3)], "in order, each at its size");
        assert_eq!(land.pages.iter().map(Vec::len).sum::<usize>(), 11);
        assert!(land.pages.iter().flatten().all(|t| t.id.starts_with('n')), "new ids");

        // A grid too small for the music: it takes its other size; past the last page, left out
        let mut packed = DeckFace::portrait();
        packed.pages[0] = (0..32).map(|i| tile(&format!("t{i}"), "button", i % 4, i / 4, 1, 1)).collect();
        packed.pages[0].push(tile("m", "music", 0, 0, 4, 2));
        let small = reflow(&packed, [2, 2], || "x".into());
        assert_eq!(small.pages.len(), MAX_PAGES);
        assert!(small.pages.iter().flatten().any(|t| t.kind == "music" && (t.w, t.h) == (2, 2)), "the music at the size it takes here");
        assert!(small.pages.iter().flatten().count() < 33, "what doesn't fit is left out");
    }

    #[test]
    fn the_pixel_palette_and_glyphs() {
        assert_eq!(PICO[pico("#2563ff").0], "#29adff");
        assert!(!pico("#2563ff").1, "light ink on blue");
        assert_eq!(pico("#ffd400"), (7, true), "dark ink on yellow");
        assert_eq!(PICO[pico("not a colour").0], "#5f574f");
        assert_eq!(pixel_art()[3][6], "#ffec27", "the sun");

        assert!(has_glyphs("12:34") && has_glyphs("Play/Pause") && has_glyphs("Écran 62%"));
        assert!(!has_glyphs("Громкость") && !has_glyphs("音量"));
        assert_eq!(glyph('a'), glyph('A'));
        assert_eq!(wrap("Mute Microphone", 6, 2), ["Mute", "Micro."]);
        assert_eq!(wrap("Lock Screen", 6, 2), ["Lock", "Screen"]);
        assert_eq!(wrap("Next", 8, 1), ["Next"]);
        assert_eq!(wrap("Previous Track", 5, 1), ["Prev."]);

        let taken: HashSet<String> = ["00000000".to_string()].into();
        let id = new_id(|id| taken.contains(id));
        assert!(id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(new_id(|_| false), new_id(|_| false));
    }

    #[test]
    fn a_starter_deck_has_what_is_offered() {
        let deck = starter(&full(), "Main");
        assert_eq!(deck.active, deck.layouts[0].id);
        let tiles: Vec<&str> = deck.layouts[0].portrait.pages[0].iter().map(|t| t.action.as_deref().unwrap_or(&t.kind)).collect();
        assert_eq!(tiles, ["clock", "sound.mute", "music", "sound.volume", "media.play_pause"]);
        assert_eq!(deck.check(|a| full().actions.iter().any(|o| o.action.id == a)), Ok(()));
    }
}
