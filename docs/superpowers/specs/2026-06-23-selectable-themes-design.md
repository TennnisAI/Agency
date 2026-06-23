# Selectable Themes — Design

**Date:** 2026-06-23
**Status:** Approved (design)

## Goal

Let users pick a UI theme from a new **Appearance** section in Settings. Ship five
new themes derived (as inspiration, not literal copies) from these palettes, alongside
the existing Catppuccin Mocha default:

- Ammo-8 — https://lospec.com/palette-list/ammo-8
- Twilight-5 — https://lospec.com/palette-list/twilight-5
- Paperback-2 — https://lospec.com/palette-list/paperback-2
- Nicole Punk 82 — https://lospec.com/palette-list/nicole-punk-82
- CherryMelon — https://lospec.com/palette-list/cherrymelon

## Decisions (from brainstorming)

- **Palette as inspiration**: each palette sets the mood/base; the full ~22-variable
  theme is extended with cohesive extra hues so status colors stay distinguishable.
- **Light themes allowed**: Paperback is a light/paper theme; the others are dark.
  This requires theming the terminal, color-scheme, and verifying no hardcoded colors
  break on light backgrounds.
- **Persistence: localStorage** (`"theme"`), applied at startup. No backend/Tauri
  changes.

## Architecture

Single source of truth lives in TypeScript so the canvas-based terminal and the CSS
UI stay in sync.

### `ui/src/lib/themes.ts` (new)
```ts
export type ThemeId = 'catppuccin' | 'ammo' | 'twilight' | 'paperback' | 'nicole';
export interface Theme {
  id: ThemeId;
  label: string;
  scheme: 'dark' | 'light';
  vars: {
    base, mantle, crust,
    s0, s1, s2,
    o0, o1, o2,
    text, sub1, sub0,
    blue, lav, mauve, pink, green, teal, yellow, peach, red,
    line,
  }; // string hex values
}
export const THEMES: Theme[];
export const DEFAULT_THEME: ThemeId = 'catppuccin';
export function getStoredTheme(): ThemeId;   // localStorage["theme"] || DEFAULT_THEME
export function applyTheme(id: ThemeId): void;
```

`applyTheme(id)`:
1. Look up the theme; for each `vars` entry set `root.style.setProperty('--base', ...)` etc.
2. `root.dataset.theme = id` and `root.style.colorScheme = theme.scheme`.
3. `localStorage["theme"] = id`.
4. `window.dispatchEvent(new CustomEvent('themechange', { detail: id }))`.

The **semantic** variables (`--running`, `--awaiting`, `--review`, `--exited`,
`--crashed`, `--agent-*`) and the **dims** stay defined in `theme.css :root` exactly
as today — they reference the accent vars, so they re-map automatically when the
accent vars change. `theme.css :root` also keeps the Catppuccin literal values as the
pre-JS default so first paint is correct before `applyTheme` runs.

### `ui/src/main.tsx`
Call `applyTheme(getStoredTheme())` immediately before `ReactDOM.createRoot(...).render`.

### `ui/src/lib/xtermTheme.ts`
Replace the static `xtermTheme` const with `currentXtermTheme(): ITheme` that derives
the terminal theme from the active theme's CSS variable values (same mapping used
today): `background→crust`, `foreground→text`, `cursor→text`,
`selectionBackground→s0`, `black→s1`, `red→red`, `green→green`, `yellow→yellow`,
`blue→blue`, `magenta→mauve`, `cyan→teal`, `white→sub1`, and the `bright*` variants
from the brighter accents / `o0`. Read values via
`getComputedStyle(document.documentElement).getPropertyValue('--x')`.

### `ui/src/components/FocusTerminal.tsx` & `MergeModal.tsx`
Use `currentXtermTheme()` instead of the const at terminal creation. In
`FocusTerminal`, add a `themechange` listener that sets
`term.options.theme = currentXtermTheme()` so an open terminal recolors live.

### `ui/src/components/Settings.tsx`
Add an **Appearance** `<section>` at the top (above "Agent profiles"):
- `<div className="settings-section-label">Appearance</div>`
- A grid of theme cards (`.settings-theme-grid` → `.settings-theme-card`). Each card:
  theme `label` + a row of swatch dots (`base`, `s1`, `text`, `green`, `yellow`, `red`).
  The active theme's card is outlined.
- Clicking a card calls `applyTheme(id)` (live preview + persisted immediately; no
  save button needed since it is localStorage). Track active id in component state,
  seeded from `getStoredTheme()`.

### `ui/src/styles.css`
Add `.settings-theme-grid` (responsive grid) and `.settings-theme-card`
(border, hover, `[data-active]` outline) plus `.settings-theme-swatches` /
`.settings-theme-dot`. All using existing CSS vars.

## Theme color values

All values are full theme definitions; semantic mappings stay as in `theme.css`
(`running=green, awaiting=yellow, review=blue, exited=o0, crashed=red,
agent-claude=peach, agent-pi=teal, agent-hermes=mauve`).

### Ammo (dark) — military green-khaki; amber warnings, rust crashes
```
base #112318  mantle #0a160e  crust #040c06
s0 #1e3a29    s1 #285034      s2 #305d42
o0 #4d8061    o1 #6b945f      o2 #89a257
text #eeffcc  sub1 #d4e8a6    sub0 #bedc7f
blue #6fb0a6  lav #c4d8a0     mauve #ad9bb0  pink #d6a98c
green #9bcf5e teal #5fb389    yellow #e8c252 peach #d6953f  red #cf5a33
line #1a3022
```

### Twilight (dark) — dusk plum/navy; coral + steel-blue
```
base #292831  mantle #232230  crust #1e1d25
s0 #333f58    s1 #3d4a66      s2 #485674
o0 #5a6a85    o1 #7c8aa0      o2 #9aa6ba
text #ece6ec  sub1 #cabfc9    sub0 #a59cad
blue #5f9fc0  lav #aab6e2     mauve #b89bd0  pink #fbbbad
green #82b39c teal #6fb0b0    yellow #e8c98d peach #f0a18e  red #ee8695
line #2f2e3b
```

### Paperback (light) — sage paper + brown ink; muted printed inks
```
base #c9d0c7  mantle #bfc7bd  crust #b3bcb2
s0 #a7b1a6    s1 #97a296      s2 #889486
o0 #6f6358    o1 #5a4f46      o2 #483d36
text #382b26  sub1 #4a3d35    sub0 #5c4f45
blue #3f6175  lav #5f6c84     mauve #6f5570  pink #9c5f64
green #4f6a45 teal #437067    yellow #9a7825 peach #a4633a  red #97402f
line #a7b1a5
```
`scheme: 'light'`.

### CherryMelon (dark) — dark-teal watermelon rind; cherry-red + melon-green, pale-pink text
```
base #012824  mantle #012420  crust #011d1a
s0 #0e3a32    s1 #1a4a3e      s2 #265935
o0 #4a7a5e    o1 #6f9a7e      o2 #9bbfa3
text #fcdeea  sub1 #f0c4d4    sub0 #d99fb4
blue #4f9ec0  lav #9fcabd     mauve #d98fc4  pink #ff8fb0
green #3fa85f teal #44b0a2    yellow #e6cf6a peach #ff8fa3  red #ff4d6d
line #0e3a32
```

### Nicole Punk (dark) — near-black + cream; burnt-orange/amber/acid-green
```
base #21181b  mantle #1b1316  crust #160f12
s0 #2e2226    s1 #3b2d30      s2 #49383b
o0 #7c6457    o1 #9c8070      o2 #c09a7d
text #faf5d8  sub1 #e6d9bd    sub0 #d8ae8b
blue #4f93a8  lav #c9a0d0     mauve #b07ab0  pink #e08b8b
green #9ab43f teal #54a890    yellow #f2ab37 peach #cd5f2a  red #db3f28
line #2b2024
```

## Risks / build-time checks

- **Hardcoded colors**: audit `styles.css` (and `lib/xtermTheme.ts` call sites) for
  any literal hex not using a var; route through vars so the light theme works.
- **Light-theme contrast**: accents in Paperback are deliberately darker so they read
  as ink on the light page. Verify any place an accent is used as a *background* fill
  behind dark text (e.g. badges) still reads — adjust if needed.
- **Box-shadows**: shadows tuned for dark may look heavy on Paperback; soften via a
  var if they read poorly.

## Testing / verification

Manual: open Settings → Appearance, switch through all five themes. Confirm the whole
UI (including the focus terminal) recolors live, the choice persists across reload,
and statuses (running/awaiting/review/crashed) remain distinguishable in each theme.
Build passes (`npm run build` in `ui/`).
