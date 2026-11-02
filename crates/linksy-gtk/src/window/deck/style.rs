//! The editor's look and the five themes the phone draws, as the prototype has them. GTK has
//! no clip-path or mask: Pixel's notched corners are square, with bevels and a hard shadow.

use std::cell::OnceCell;

use gtk::gdk;

use super::model::{ACCENTS, COLORS, PICO};

const CSS: &str = r#"
.deck-bar { padding: 6px 12px; border-bottom: 1px solid alpha(currentColor, 0.12); }
.deck-chip { font-size: 0.85em; font-weight: 600; color: @success_color; }
.deck-chip.dirty { color: alpha(currentColor, 0.6); }
.deck-library { border-right: 1px solid alpha(currentColor, 0.12); }
.deck-inspector { border-left: 1px solid alpha(currentColor, 0.12); }
.deck-work.narrow .deck-inspector { border-left: none; border-top: 1px solid alpha(currentColor, 0.12); }
.deck-library, .deck-inspector-box { padding: 14px; }
.deck-group { font-size: 0.75em; font-weight: 700; letter-spacing: 0.06em; opacity: 0.6; margin: 10px 8px 2px; }
.deck-items { background: none; }
.deck-items > row { border-radius: 10px; padding: 6px 8px; }
.deck-item .ic { min-width: 32px; min-height: 32px; border-radius: 9px; background-color: alpha(@accent_bg_color, 0.12); color: @accent_color; }
.deck-item .name { font-weight: 500; }
.deck-item .hint { font-size: 0.85em; opacity: 0.65; }
.deck-tag { font-size: 0.72em; font-weight: 600; padding: 0 5px; border-radius: 6px; border: 1px solid alpha(currentColor, 0.2); opacity: 0.75; }
.deck-hint { font-size: 0.9em; opacity: 0.7; }
.deck-where { font-size: 0.9em; opacity: 0.75; border: 1px dashed alpha(currentColor, 0.2); border-radius: 10px; padding: 8px 10px; }
.deck-field { font-size: 0.85em; font-weight: 600; opacity: 0.7; }
.deck-warn { color: @warning_color; }

.deck-stage {
  background-color: alpha(currentColor, 0.04);
  background-image: radial-gradient(circle farthest-corner at 50% 30%, alpha(@accent_bg_color, 0.12), transparent 60%);
}
.deck-note { font-size: 0.85em; opacity: 0.75; }
.deck-note.flash { color: @warning_color; opacity: 1; }
.deck-theme { border-radius: 99px; padding: 3px 9px 3px 4px; min-height: 0; border: 1px solid alpha(currentColor, 0.15); background-color: @view_bg_color; box-shadow: none; font-size: 0.9em; font-weight: 500; }
.deck-theme:checked { border-color: @accent_bg_color; color: @accent_color; box-shadow: 0 0 0 1px @accent_bg_color; background-color: @view_bg_color; }
.deck-theme .sw { min-width: 18px; min-height: 18px; border-radius: 99px; }
.deck-themes flowboxchild { padding: 0; background: none; }
.sw-stock { background-color: #2e2e32; box-shadow: inset 0 0 0 5px #1d1d20, inset 0 0 0 9px #3584e4; }
.sw-dots { background-color: #000; background-image: radial-gradient(circle, #fff 1.4px, transparent 1.9px); background-size: 5px 5px; }
.sw-eink { background-color: #e7e7e1; box-shadow: inset 0 0 0 2px #111; }
.sw-term { background-color: #040904; box-shadow: inset 0 0 0 2px #39ff7a; }
.deck-theme .sw.sw-pixel { border-radius: 0; background-image: conic-gradient(#ff004d 25%, #ffec27 25%, #ffec27 50%, #29adff 50%, #29adff 75%, #00e436 75%); }
.deck-swatch { min-width: 26px; min-height: 26px; padding: 0; border-radius: 8px; box-shadow: none; }
.deck-swatch:checked { box-shadow: 0 0 0 2px @window_bg_color, 0 0 0 4px @window_fg_color; }
.deck-size { padding: 2px 9px; min-height: 0; font-feature-settings: "tnum"; }

.deck-phone { background-color: #05070d; border-radius: 34px; padding: 11px; box-shadow: 0 30px 60px rgba(0, 0, 0, 0.35), inset 0 0 0 1.5px #2a3040; }
.deck-screen { border-radius: 24px; color: #eef2fb; }
.deck-status { font-size: 11px; font-weight: 600; opacity: 0.85; padding: 8px 16px 0; min-height: 18px; }
.deck-name { padding: 2px 14px 0; min-height: 22px; }
.deck-name .layout { font-size: 12px; font-weight: 600; }
.deck-name .pageinfo { font-size: 11px; font-weight: 500; opacity: 0.6; }
.deck-grid { margin: 8px 10px 0; }
.deck-dots { min-height: 30px; margin-bottom: 6px; }
.land .deck-dots { min-height: 20px; margin-bottom: 4px; }
.deck-dots button { min-width: 8px; min-height: 8px; padding: 0; margin: 0 3px; border-radius: 99px; background-color: rgba(255, 255, 255, 0.28); box-shadow: none; transition: min-width 150ms ease; }
.deck-dots button.current { min-width: 18px; border-radius: 4px; background-color: #fff; }
.deck-dots button.add, .deck-dots button.remove { min-width: 18px; min-height: 18px; background: none; color: rgba(255, 255, 255, 0.6); font-size: 15px; font-weight: 600; }
.deck-dots button.add:hover, .deck-dots button.remove:hover { color: #fff; }

.deck-tile { border-radius: 16px; background-color: #243050; color: #fff; transition: transform 140ms ease, box-shadow 120ms ease, opacity 120ms ease; }
.deck-tile .face { padding: 9px; }
.deck-tile.one .face { padding: 5px; }
.deck-tile .t-label { font-size: 11px; font-weight: 600; }
.deck-tile.one .t-label { font-size: 10px; }
.deck-tile .small { font-size: 10.5px; opacity: 0.75; }
.deck-tile .title { font-size: 13px; font-weight: 600; }
.deck-tile .pct { font-size: 10.5px; font-weight: 600; }
.deck-tile .big, .deck-tile .mid { font-weight: 600; font-feature-settings: "tnum"; }
.deck-tile:hover { transform: translateY(-1px); }
.deck-tile.lifted { transform: scale(1.04); box-shadow: 0 14px 30px rgba(0, 0, 0, 0.45); opacity: 0.94; }
.deck-tile.picked { outline: 2px solid @accent_color; outline-offset: 3px; }
.deck-tile.blocked { outline: 2px solid #ff6b6b; outline-offset: 3px; }
.deck-tile:focus-visible:not(.picked) { outline: 2px dashed alpha(@accent_color, 0.7); outline-offset: 3px; }
.deck-tile.unavailable { opacity: 0.45; }
.deck-tile.pop { animation: deck-tile-in 240ms cubic-bezier(0.2, 1.4, 0.5, 1); }
.deck-tile.gone { animation: deck-tile-out 160ms ease-in forwards; }
@keyframes deck-tile-in { from { transform: scale(0.7); opacity: 0; } to { transform: none; opacity: 1; } }
@keyframes deck-tile-out { from { transform: none; opacity: 1; } to { transform: scale(0.6); opacity: 0; } }
@keyframes deck-pop { from { transform: scale(0.3); opacity: 0; } to { transform: none; opacity: 1; } }
.deck-play { border-radius: 99px; background-color: #fff; color: #111; }
.deck-confirm { min-width: 12px; min-height: 12px; margin: 5px; border-radius: 99px; background-color: #ffb020; color: #1d1d20; font-size: 9px; font-weight: 800; }

.deck-del { min-width: 18px; min-height: 18px; padding: 0; border-radius: 99px; border: 2px solid #0d1220; background-color: #e5484d; color: #fff; box-shadow: none; animation: deck-pop 180ms cubic-bezier(0.3, 1.6, 0.6, 1); transition: transform 120ms ease, background-color 120ms ease; }
.deck-del:hover { transform: scale(1.15); background-color: #ff3d45; }
.deck-handle { background-color: #fff; box-shadow: 0 1px 4px rgba(0, 0, 0, 0.45); animation: deck-pop 180ms cubic-bezier(0.3, 1.6, 0.6, 1); transition: transform 120ms ease; }
.deck-handle:hover { transform: scale(1.2); }
.deck-handle.right, .deck-handle.bottom { border-radius: 4px; }
.deck-handle.corner { border-radius: 99px; background-color: @accent_bg_color; border: 2px solid #fff; }
.deck-size-badge { background-color: #fff; color: #111; font-size: 11px; font-weight: 600; padding: 2px 7px; border-radius: 6px; box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3); font-feature-settings: "tnum"; }
.deck-ghost { background: none; }

.deck-screen.theme-stock { background-color: #1d1d20; color: #fff; }
.theme-stock .deck-tile { background-color: #2e2e32; border-radius: 14px; }
.theme-stock .deck-play { background-color: #fff; color: #1d1d20; }

.deck-screen.theme-dots { background-color: #000; background-image: radial-gradient(circle, rgba(255, 255, 255, 0.07) 1px, transparent 1.3px); background-size: 6px 6px; color: #f2f2f2; }
.theme-dots .deck-status, .theme-dots .deck-name label { font-family: monospace; font-weight: 800; letter-spacing: 0.06em; text-transform: uppercase; }
.theme-dots .deck-tile { background-color: rgba(255, 255, 255, 0.025); box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.13); border-radius: 18px; color: #f2f2f2; }
.theme-dots .deck-tile.lifted { background-color: #0b0b0b; box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.13), 0 14px 30px rgba(0, 0, 0, 0.45); }
.theme-dots .deck-tile label.t-label, .theme-dots .deck-tile label.title, .theme-dots .deck-tile label.pct { font-family: monospace; font-weight: 800; text-transform: uppercase; }

.deck-screen.theme-pixel { background-color: #1d2b53; background-image: linear-gradient(rgba(0, 0, 0, 0.18) 50%, transparent 50%); background-size: 100% 4px; border-radius: 16px; color: #fff1e8; }
.theme-pixel .deck-status, .theme-pixel .deck-name label, .theme-pixel .deck-tile label { font-family: monospace; font-weight: 700; text-transform: uppercase; }
.theme-pixel .deck-tile { border-radius: 0; color: #fff1e8; box-shadow: inset 0 3px 0 rgba(255, 255, 255, 0.28), inset 0 -3px 0 rgba(0, 0, 0, 0.3), inset 3px 0 0 rgba(255, 255, 255, 0.12), inset -3px 0 0 rgba(0, 0, 0, 0.18), 3px 3px 0 #000; }
.theme-pixel .deck-tile.lifted { box-shadow: inset 0 3px 0 rgba(255, 255, 255, 0.28), inset 0 -3px 0 rgba(0, 0, 0, 0.3), inset 3px 0 0 rgba(255, 255, 255, 0.12), inset -3px 0 0 rgba(0, 0, 0, 0.18), 6px 6px 0 #000; }
.theme-pixel .deck-tile.ink-dark { color: #1d2b53; }
.theme-pixel .deck-tile.on { background-color: #ff004d; color: #fff1e8; }
.theme-pixel .deck-tile.picked { outline-style: dashed; }
.theme-pixel .deck-play { border-radius: 0; background-color: #ff004d; color: #fff1e8; box-shadow: 2px 2px 0 #000; }
.theme-pixel .deck-dots button { border-radius: 0; }
.theme-pixel .deck-dots button.current { background-color: #ffec27; }

.deck-screen.theme-eink { background-color: #e7e7e1; color: #111; }
.theme-eink label { font-family: monospace; }
.theme-eink .deck-tile { background-color: #f5f5f0; color: #111; box-shadow: inset 0 0 0 2px #111; border-radius: 10px; }
.theme-eink .deck-tile.lifted { box-shadow: inset 0 0 0 2px #111, 0 14px 30px rgba(0, 0, 0, 0.3); }
.theme-eink .deck-tile.on { background-color: #111; color: #f5f5f0; }
.theme-eink .deck-play { background-color: #111; color: #f5f5f0; }
.theme-eink .deck-handle.right, .theme-eink .deck-handle.bottom { background-color: #111; box-shadow: 0 0 0 1.5px #f5f5f0; }
.theme-eink .deck-dots button { background-color: rgba(0, 0, 0, 0.25); }
.theme-eink .deck-dots button.current { background-color: #111; }
.theme-eink .deck-dots button.add, .theme-eink .deck-dots button.remove { background: none; color: #111; }

.deck-screen.theme-term { background-color: #040904; color: #39ff7a; }
.theme-term label { font-family: monospace; text-shadow: 0 0 6px rgba(57, 255, 122, 0.55); }
.theme-term .deck-tile { background-color: rgba(57, 255, 122, 0.04); color: #39ff7a; box-shadow: inset 0 0 0 1px rgba(57, 255, 122, 0.45); border-radius: 4px; }
.theme-term .deck-tile.lifted { background-color: #071407; box-shadow: inset 0 0 0 1px rgba(57, 255, 122, 0.45), 0 14px 30px rgba(0, 0, 0, 0.45); }
.theme-term .deck-tile.on { background-color: rgba(57, 255, 122, 0.85); color: #041204; }
.theme-term .deck-tile.on label { text-shadow: none; }
.theme-term .deck-tile .t-label { font-size: 12px; font-weight: 400; }
.theme-term .deck-tile.one .t-label { font-size: 10px; }
.theme-term .deck-play { background-color: #39ff7a; color: #041204; border-radius: 2px; }
.theme-term .deck-dots button { background-color: rgba(57, 255, 122, 0.3); }
.theme-term .deck-dots button.current { background-color: #39ff7a; }
.theme-term .deck-dots button.add, .theme-term .deck-dots button.remove { background: none; color: rgba(57, 255, 122, 0.7); }
"#;

thread_local! {
    static ACCENT: OnceCell<gtk::CssProvider> = const { OnceCell::new() };
}

/// The editor's stylesheet, once; then the layout's accent, whenever it changes.
pub fn load(accent: &str) {
    let Some(display) = gdk::Display::default() else { return };
    ACCENT.with(|cell| {
        let provider = cell.get_or_init(|| {
            let mut css = CSS.to_string();
            for (i, c) in PICO.iter().enumerate() {
                css.push_str(&format!(".theme-pixel .deck-tile.px{i} {{ background-color: {c}; }}\n"));
            }
            for c in COLORS.iter().chain(ACCENTS.iter()) {
                css.push_str(&format!(".deck-swatch.c{} {{ background-color: {c}; }}\n", &c[1..]));
            }
            let base = gtk::CssProvider::new();
            base.load_from_string(&css);
            gtk::style_context_add_provider_for_display(&display, &base, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            let accent = gtk::CssProvider::new();
            gtk::style_context_add_provider_for_display(&display, &accent, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
            accent
        });
        provider.load_from_string(&accent_css(accent));
    });
}

fn accent_css(a: &str) -> String {
    format!(
        ".theme-stock .deck-tile.on {{ background-color: {a}; color: #fff; }}\n\
         .theme-dots .deck-tile.on {{ color: {a}; box-shadow: inset 0 0 0 1px {a}; background-color: alpha({a}, 0.08); }}\n\
         .theme-dots .deck-play {{ background-color: {a}; color: #fff; }}\n\
         .theme-dots .deck-dots button.current {{ background-color: {a}; }}\n"
    )
}
