# Linksy brand

## Logo mark: "Sender"

Two devices, each sending out one arc toward the other. The sending device is larger and
reaches further, and the receiver answers with a smaller arc. Reads as discovery and transfer.

Chosen from round 2 of the explorations (`exploration/round2.html`, variation "Sender").

| File | Use |
|---|---|
| `logo/linksy-mark.svg` | Single color, inherits `currentColor` (UI, theming) |
| `logo/linksy-mark--signal.svg` | **Brand colors**: Signal blue `#2563FF` + cyan `#22C3E6` |
| `logo/linksy-mark--mono.svg` | Ink `#141821` only, for one-color print |
| `logo/logo.png` | 1024×1024, Signal colors on white |

## Palette: Signal

| Role | Color |
|---|---|
| Primary (sender, accent, buttons) | `#2563FF` |
| Secondary (receiver, highlights) | `#22C3E6` |
| Deep (pressed states, text on light tints) | `#1438B8` |

The apps use Signal blue as their accent color. The other candidates (Aurora, Coral,
Mint & Ink) are kept in `exploration/palettes/`.

Geometry: 100×100 viewBox. Sender dot r12 at (25,75), arc r28 stroke 9.5;
receiver dot r8 at (78,22), arc r20 stroke 8. Round caps.

## Status

- [x] Mark chosen: Sender
- [x] Palette: Signal
- [ ] Small-size version (16–32 px) with a heavier receiver side
- [ ] Wordmark "linksy" and lockups
- [x] Linux app icon (`data/icons/`), `logo/logo.png`
- [ ] macOS and Windows app icons

## Explorations

`exploration/` holds rounds 1 and 2: source marks (`marks.js`, `marks-r2.js`),
SVG exports, and the showcase pages. Preview locally with `bun exploration/serve.ts`
(http://localhost:4800, round 2 at `/round2`).
