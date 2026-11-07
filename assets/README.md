# EasyAI — logo asset kit

Two halves of one block, interlocking through a stepped seam. Orange is the
piece you add; ink is the system it joins.

`easyai-mark.svg` is the project's primary logo. The Tauri app icon and the
frontend favicon are generated from this set.

## Files

| File | Use |
| --- | --- |
| `easyai-mark.svg` | Primary logo — orange left, ink right |
| `easyai-mark-on-dark.svg` | Orange left, white right — dark surfaces |
| `easyai-mark-mono.svg` | One ink, transparent seam |
| `easyai-mark-mono-white.svg` | One white, transparent seam |
| `favicon.svg` | Adaptive: right half switches to white in dark mode |
| `easyai-lockup-horizontal.svg` / `-white.svg` | Mark + wordmark |
| `easyai-lockup-stacked.svg` / `-white.svg` | Vertical lockup |
| `export/` | PNG set, favicon.ico, PWA icons, manifest, head snippet |
| `GUIDELINES.md` | Colour, clear space, minimum sizes, misuse |

## Notes

- Geometry: 176 × 176 block on a 256 grid, radius 40, seam 20.
- The 20-unit seam closes below ~24 px, but the two-tone split still reads;
  the one-colour versions lose the split at small sizes.
- App/PWA icons are rendered on an Ink tile so both halves stay visible on
  light and dark docks; `favicon.svg` switches the right half to white in
  dark mode.
- The wordmark is outlined Noto Sans Bold (SIL OFL).
