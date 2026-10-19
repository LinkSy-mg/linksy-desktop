# Linksy brand

## Logo mark: "Sender"

Two devices, each sending out one arc toward the other. The sending device is larger and
reaches further, and the receiver answers with a smaller arc. Reads as discovery and transfer.

Chosen from round 2 of the explorations (`exploration/round2.html`, variation "Sender").

| File | Use |
|---|---|
| `logo/linksy-mark.svg` | Single color, inherits `currentColor` (UI, theming) |
| `logo/linksy-mark--signal.svg` | Electric blue `#2563FF` + cyan `#22C3E6` |
| `logo/linksy-mark--aurora.svg` | Violet `#7457F5` + teal `#14BFA2` |
| `logo/linksy-mark--coral.svg` | Coral `#FF5A52` + amber `#FFAE3A` |
| `logo/linksy-mark--mint.svg` | Mint `#12B282` + ink `#1E2A3B` (use `#D5DEEA` for ink on dark) |
| `logo/linksy-mark--mono.svg` | Ink `#141821` only |

Geometry: 100×100 viewBox. Sender dot r12 at (25,75), arc r28 stroke 9.5;
receiver dot r8 at (78,22), arc r20 stroke 8. Round caps.

## Status

- [x] Mark chosen: Sender
- [ ] Palette (candidates above)
- [ ] Small-size version (16–32 px) with a heavier receiver side
- [ ] Wordmark "linksy" and lockups
- [ ] App icons (Linux, macOS, Windows) and PNG exports

## Explorations

`exploration/` holds rounds 1 and 2: source marks (`marks.js`, `marks-r2.js`),
SVG exports, and the showcase pages. Preview locally with `bun exploration/serve.ts`
(http://localhost:4800, round 2 at `/round2`).
