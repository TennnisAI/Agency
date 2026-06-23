# Selectable Themes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an Appearance section to Settings that lets users pick among six UI themes (existing Catppuccin + five new palette-inspired themes), persisted in localStorage and applied to both the CSS UI and the xterm terminal.

**Architecture:** A TypeScript theme registry (`ui/src/lib/themes.ts`) is the single source of truth. `applyTheme(id)` writes each color as an inline CSS custom property on `<html>`, sets `data-theme` + `color-scheme`, persists to localStorage, and fires a `themechange` event. The xterm terminal derives its `ITheme` from the same registry so it recolors with the UI. Semantic vars (`--running`, `--crashed`, `--agent-*`) stay in `theme.css` referencing the accent vars, so they re-map for free.

**Tech Stack:** React 18 + TypeScript, Vite, Vitest (node env), plain CSS custom properties, xterm.js.

## Global Constraints

- No Tailwind / CSS-in-JS — plain CSS custom properties only.
- No backend/Tauri changes — theme lives entirely in the frontend (localStorage).
- Pure logic must be testable in Vitest's **node** environment: inject `Storage` (mirror `ui/src/hooks/usePaneWidth.ts`), keep DOM side-effects in thin non-unit-tested glue.
- All colors flow through CSS vars; the Paperback theme is **light** (`color-scheme: light`), so no UI may hardcode dark-only colors.
- `vars` object keys are identical to their CSS variable names (key `base` → `--base`). 22 keys: `base, mantle, crust, s0, s1, s2, o0, o1, o2, text, sub1, sub0, blue, lav, mauve, pink, green, teal, yellow, peach, red, line`.
- Exact theme hex values are defined in `docs/superpowers/specs/2026-06-23-selectable-themes-design.md` and reproduced verbatim in Task 1.

---

### Task 1: Theme registry + pure helpers

**Files:**
- Create: `ui/src/lib/themes.ts`
- Test: `ui/src/lib/themes.test.ts`

**Interfaces:**
- Consumes: `import type { ITheme } from "@xterm/xterm"`.
- Produces:
  - `type ThemeId = 'catppuccin' | 'ammo' | 'twilight' | 'paperback' | 'nicole' | 'cherrymelon'`
  - `interface Theme { id: ThemeId; label: string; scheme: 'dark' | 'light'; vars: ThemeVars }`
  - `const THEMES: Theme[]`, `const DEFAULT_THEME: ThemeId`
  - `resolveTheme(id: string): Theme` — matched theme or the default
  - `getStoredTheme(storage?: Pick<Storage, 'getItem'>): ThemeId`
  - `themeVars(theme: Theme): Array<[string, string]>` — `['--base', value]` pairs
  - `xtermThemeFor(theme: Theme): ITheme`
  - `STORAGE_KEY = 'theme'`

- [ ] **Step 1: Write the failing test**

Create `ui/src/lib/themes.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import {
  THEMES, DEFAULT_THEME, resolveTheme, getStoredTheme, themeVars, xtermThemeFor,
} from "./themes";

function makeStorage(seed?: Record<string, string>): Pick<Storage, "getItem"> {
  const map = new Map(Object.entries(seed ?? {}));
  return { getItem: (k: string) => map.get(k) ?? null };
}

const VAR_KEYS = [
  "base","mantle","crust","s0","s1","s2","o0","o1","o2",
  "text","sub1","sub0","blue","lav","mauve","pink","green","teal","yellow","peach","red","line",
];

describe("THEMES registry", () => {
  it("contains all six themes with unique ids", () => {
    const ids = THEMES.map((t) => t.id);
    expect(ids).toEqual(["catppuccin", "ammo", "twilight", "paperback", "nicole", "cherrymelon"]);
    expect(new Set(ids).size).toBe(6);
  });
  it("defines every required color var on each theme", () => {
    for (const t of THEMES) {
      for (const k of VAR_KEYS) {
        expect(t.vars[k as keyof typeof t.vars], `${t.id}.${k}`).toMatch(/^#[0-9a-fA-F]{6}$/);
      }
    }
  });
  it("marks paperback as the only light theme", () => {
    expect(resolveTheme("paperback").scheme).toBe("light");
    expect(THEMES.filter((t) => t.scheme === "light").map((t) => t.id)).toEqual(["paperback"]);
  });
});

describe("resolveTheme", () => {
  it("returns the matching theme", () => {
    expect(resolveTheme("ammo").id).toBe("ammo");
  });
  it("falls back to the default for unknown ids", () => {
    expect(resolveTheme("nope").id).toBe(DEFAULT_THEME);
  });
});

describe("getStoredTheme", () => {
  it("returns the default when nothing is stored", () => {
    expect(getStoredTheme(makeStorage())).toBe(DEFAULT_THEME);
  });
  it("returns a stored valid theme id", () => {
    expect(getStoredTheme(makeStorage({ theme: "twilight" }))).toBe("twilight");
  });
  it("ignores an invalid stored id", () => {
    expect(getStoredTheme(makeStorage({ theme: "bogus" }))).toBe(DEFAULT_THEME);
  });
});

describe("themeVars", () => {
  it("emits one --prefixed pair per color key", () => {
    const pairs = themeVars(resolveTheme("catppuccin"));
    expect(pairs).toHaveLength(VAR_KEYS.length);
    expect(pairs).toContainEqual(["--base", "#1e1e2e"]);
    expect(pairs.every(([name]) => name.startsWith("--"))).toBe(true);
  });
});

describe("xtermThemeFor", () => {
  it("maps registry colors onto the xterm ITheme", () => {
    const t = resolveTheme("catppuccin");
    const x = xtermThemeFor(t);
    expect(x.background).toBe(t.vars.crust);
    expect(x.foreground).toBe(t.vars.text);
    expect(x.red).toBe(t.vars.red);
    expect(x.magenta).toBe(t.vars.mauve);
    expect(x.cyan).toBe(t.vars.teal);
    expect(x.brightBlack).toBe(t.vars.o0);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npx vitest run src/lib/themes.test.ts`
Expected: FAIL — `Failed to resolve import "./themes"` / functions not defined.

- [ ] **Step 3: Write minimal implementation**

Create `ui/src/lib/themes.ts`:

```ts
import type { ITheme } from "@xterm/xterm";

export type ThemeId = "catppuccin" | "ammo" | "twilight" | "paperback" | "nicole" | "cherrymelon";

export interface ThemeVars {
  base: string; mantle: string; crust: string;
  s0: string; s1: string; s2: string;
  o0: string; o1: string; o2: string;
  text: string; sub1: string; sub0: string;
  blue: string; lav: string; mauve: string; pink: string;
  green: string; teal: string; yellow: string; peach: string; red: string;
  line: string;
}

export interface Theme {
  id: ThemeId;
  label: string;
  scheme: "dark" | "light";
  vars: ThemeVars;
}

export const STORAGE_KEY = "theme";
export const DEFAULT_THEME: ThemeId = "catppuccin";

export const THEMES: Theme[] = [
  {
    id: "catppuccin", label: "Catppuccin Mocha", scheme: "dark",
    vars: {
      base: "#1e1e2e", mantle: "#181825", crust: "#11111b",
      s0: "#313244", s1: "#45475a", s2: "#585b70",
      o0: "#6c7086", o1: "#7f849c", o2: "#9399b2",
      text: "#cdd6f4", sub1: "#bac2de", sub0: "#a6adc8",
      blue: "#89b4fa", lav: "#b4befe", mauve: "#cba6f7", pink: "#f5c2e7",
      green: "#a6e3a1", teal: "#94e2d5", yellow: "#f9e2af", peach: "#fab387", red: "#f38ba8",
      line: "#25253a",
    },
  },
  {
    id: "ammo", label: "Ammo", scheme: "dark",
    vars: {
      base: "#112318", mantle: "#0a160e", crust: "#040c06",
      s0: "#1e3a29", s1: "#285034", s2: "#305d42",
      o0: "#4d8061", o1: "#6b945f", o2: "#89a257",
      text: "#eeffcc", sub1: "#d4e8a6", sub0: "#bedc7f",
      blue: "#6fb0a6", lav: "#c4d8a0", mauve: "#ad9bb0", pink: "#d6a98c",
      green: "#9bcf5e", teal: "#5fb389", yellow: "#e8c252", peach: "#d6953f", red: "#cf5a33",
      line: "#1a3022",
    },
  },
  {
    id: "twilight", label: "Twilight", scheme: "dark",
    vars: {
      base: "#292831", mantle: "#232230", crust: "#1e1d25",
      s0: "#333f58", s1: "#3d4a66", s2: "#485674",
      o0: "#5a6a85", o1: "#7c8aa0", o2: "#9aa6ba",
      text: "#ece6ec", sub1: "#cabfc9", sub0: "#a59cad",
      blue: "#5f9fc0", lav: "#aab6e2", mauve: "#b89bd0", pink: "#fbbbad",
      green: "#82b39c", teal: "#6fb0b0", yellow: "#e8c98d", peach: "#f0a18e", red: "#ee8695",
      line: "#2f2e3b",
    },
  },
  {
    id: "paperback", label: "Paperback", scheme: "light",
    vars: {
      base: "#c9d0c7", mantle: "#bfc7bd", crust: "#b3bcb2",
      s0: "#a7b1a6", s1: "#97a296", s2: "#889486",
      o0: "#6f6358", o1: "#5a4f46", o2: "#483d36",
      text: "#382b26", sub1: "#4a3d35", sub0: "#5c4f45",
      blue: "#3f6175", lav: "#5f6c84", mauve: "#6f5570", pink: "#9c5f64",
      green: "#4f6a45", teal: "#437067", yellow: "#9a7825", peach: "#a4633a", red: "#97402f",
      line: "#a7b1a5",
    },
  },
  {
    id: "nicole", label: "Nicole Punk", scheme: "dark",
    vars: {
      base: "#21181b", mantle: "#1b1316", crust: "#160f12",
      s0: "#2e2226", s1: "#3b2d30", s2: "#49383b",
      o0: "#7c6457", o1: "#9c8070", o2: "#c09a7d",
      text: "#faf5d8", sub1: "#e6d9bd", sub0: "#d8ae8b",
      blue: "#4f93a8", lav: "#c9a0d0", mauve: "#b07ab0", pink: "#e08b8b",
      green: "#9ab43f", teal: "#54a890", yellow: "#f2ab37", peach: "#cd5f2a", red: "#db3f28",
      line: "#2b2024",
    },
  },
  {
    id: "cherrymelon", label: "Cherry Melon", scheme: "dark",
    vars: {
      base: "#012824", mantle: "#012420", crust: "#011d1a",
      s0: "#0e3a32", s1: "#1a4a3e", s2: "#265935",
      o0: "#4a7a5e", o1: "#6f9a7e", o2: "#9bbfa3",
      text: "#fcdeea", sub1: "#f0c4d4", sub0: "#d99fb4",
      blue: "#4f9ec0", lav: "#9fcabd", mauve: "#d98fc4", pink: "#ff8fb0",
      green: "#3fa85f", teal: "#44b0a2", yellow: "#e6cf6a", peach: "#ff8fa3", red: "#ff4d6d",
      line: "#0e3a32",
    },
  },
];

export function resolveTheme(id: string): Theme {
  return THEMES.find((t) => t.id === id) ?? THEMES.find((t) => t.id === DEFAULT_THEME)!;
}

export function getStoredTheme(storage: Pick<Storage, "getItem"> = localStorage): ThemeId {
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (raw && THEMES.some((t) => t.id === raw)) return raw as ThemeId;
  } catch {
    /* storage unavailable */
  }
  return DEFAULT_THEME;
}

export function themeVars(theme: Theme): Array<[string, string]> {
  return (Object.entries(theme.vars) as Array<[string, string]>).map(([k, v]) => [`--${k}`, v]);
}

export function xtermThemeFor(theme: Theme): ITheme {
  const v = theme.vars;
  return {
    background: v.crust, foreground: v.text, cursor: v.text, selectionBackground: v.s0,
    black: v.s1, red: v.red, green: v.green, yellow: v.yellow,
    blue: v.blue, magenta: v.mauve, cyan: v.teal, white: v.sub1,
    brightBlack: v.o0, brightRed: v.red, brightGreen: v.green, brightYellow: v.yellow,
    brightBlue: v.blue, brightMagenta: v.mauve, brightCyan: v.teal, brightWhite: v.text,
  };
}

let active: ThemeId = DEFAULT_THEME;

export function applyTheme(id: string): void {
  const theme = resolveTheme(id);
  active = theme.id;
  const root = document.documentElement;
  for (const [name, value] of themeVars(theme)) root.style.setProperty(name, value);
  root.dataset.theme = theme.id;
  root.style.colorScheme = theme.scheme;
  try {
    localStorage.setItem(STORAGE_KEY, theme.id);
  } catch {
    /* storage unavailable */
  }
  window.dispatchEvent(new CustomEvent("themechange", { detail: theme.id }));
}

export function currentXtermTheme(): ITheme {
  return xtermThemeFor(resolveTheme(active));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ui && npx vitest run src/lib/themes.test.ts`
Expected: PASS (all describe blocks green).

- [ ] **Step 5: Commit**

```bash
git add ui/src/lib/themes.ts ui/src/lib/themes.test.ts
git commit -m "feat(ui): theme registry with five palettes and pure helpers"
```

---

### Task 2: Apply theme at startup + drive the terminal from the registry

**Files:**
- Modify: `ui/src/main.tsx`
- Delete: `ui/src/lib/xtermTheme.ts`
- Modify: `ui/src/components/FocusTerminal.tsx` (import line 7; terminal creation line 38)
- Modify: `ui/src/components/MergeModal.tsx` (import line 5; terminal creation line 42)

**Interfaces:**
- Consumes: `applyTheme`, `getStoredTheme`, `currentXtermTheme` from Task 1.
- Produces: terminals colored by the active theme; `themechange` listener live-updates the focus terminal.

This task is verified by `tsc`/build + manual check (DOM/canvas side-effects are not unit-testable in the node env).

- [ ] **Step 1: Apply the stored theme before render**

Edit `ui/src/main.tsx` — add the import and call `applyTheme` before `createRoot`:

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, getStoredTheme } from "./lib/themes";
import "./theme.css";
import "./styles.css";

applyTheme(getStoredTheme());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

- [ ] **Step 2: Remove the static xterm theme module**

Run:
```bash
git rm ui/src/lib/xtermTheme.ts
```
(Logic now lives in `themes.ts` as `currentXtermTheme` / `xtermThemeFor`.)

- [ ] **Step 3: Point FocusTerminal at the registry + live-update on theme change**

In `ui/src/components/FocusTerminal.tsx`:

Change the import on line 7 from:
```tsx
import { xtermTheme } from "../lib/xtermTheme";
```
to:
```tsx
import { currentXtermTheme } from "../lib/themes";
```

Change the terminal creation (line 38) from `theme: xtermTheme` to `theme: currentXtermTheme()`:
```tsx
    const term = new Terminal({ convertEol: true, fontSize: 13, cursorBlink: true, theme: currentXtermTheme() });
```

Inside the same `useEffect` (after `ro.observe(container);`), register a listener that recolors the open terminal, and dispose it in the cleanup:
```tsx
    const onThemeChange = () => { term.options.theme = currentXtermTheme(); };
    window.addEventListener("themechange", onThemeChange);
```
Then in the effect's cleanup/return, alongside the existing teardown, add:
```tsx
      window.removeEventListener("themechange", onThemeChange);
```

- [ ] **Step 4: Point MergeModal at the registry**

In `ui/src/components/MergeModal.tsx`:

Change the import on line 5 from:
```tsx
import { xtermTheme } from "../lib/xtermTheme";
```
to:
```tsx
import { currentXtermTheme } from "../lib/themes";
```

Change the terminal creation (line 42) from `theme: xtermTheme` to:
```tsx
    const term = new Terminal({ convertEol: true, fontSize: 12, theme: currentXtermTheme() });
```

- [ ] **Step 5: Verify build + existing tests**

Run: `cd ui && npm run build && npx vitest run`
Expected: build succeeds (no `xtermTheme` import errors), all tests pass.

- [ ] **Step 6: Commit**

```bash
git add ui/src/main.tsx ui/src/components/FocusTerminal.tsx ui/src/components/MergeModal.tsx ui/src/lib/xtermTheme.ts
git commit -m "feat(ui): apply theme at startup and drive xterm from the theme registry"
```

---

### Task 3: Appearance section in Settings

**Files:**
- Modify: `ui/src/components/Settings.tsx` (imports; new state; new section after line 105)
- Modify: `ui/src/styles.css` (after line 164)

**Interfaces:**
- Consumes: `THEMES`, `applyTheme`, `getStoredTheme`, `type ThemeId` from Task 1.
- Produces: a clickable theme picker that applies + persists immediately.

Verified by build + manual check.

- [ ] **Step 1: Import the registry and track the active theme**

In `ui/src/components/Settings.tsx`, add to the existing import block (after line 14):
```tsx
import { THEMES, ThemeId, applyTheme, getStoredTheme } from "../lib/themes";
```

Add state inside the component (after the `notif` state, line 31):
```tsx
  const [themeId, setThemeId] = useState<ThemeId>(getStoredTheme());

  function pickTheme(id: ThemeId) {
    setThemeId(id);
    applyTheme(id);
  }
```

- [ ] **Step 2: Render the Appearance section**

In `ui/src/components/Settings.tsx`, insert this section immediately after the `{error && ...}` line (after line 104, before the "Agent profiles" `<section>`):
```tsx
        <section className="settings-section">
          <div className="settings-section-label">Appearance</div>
          <div className="settings-theme-grid">
            {THEMES.map((t) => (
              <button
                key={t.id}
                type="button"
                className="settings-theme-card"
                data-active={t.id === themeId}
                onClick={() => pickTheme(t.id)}
              >
                <span className="settings-theme-name">{t.label}</span>
                <span className="settings-theme-swatches">
                  {([t.vars.base, t.vars.s1, t.vars.text, t.vars.green, t.vars.yellow, t.vars.red] as const).map(
                    (c, i) => (
                      <span key={i} className="settings-theme-dot" style={{ background: c }} />
                    ),
                  )}
                </span>
              </button>
            ))}
          </div>
        </section>
```

- [ ] **Step 3: Style the picker**

In `ui/src/styles.css`, add after line 164 (after the `.settings-ghost-btn:hover` rule):
```css
.settings-theme-grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 8px; }
.settings-theme-card {
  display: flex; flex-direction: column; gap: 9px; align-items: flex-start;
  background: var(--mantle); border: 1px solid var(--s0); border-radius: 10px;
  padding: 12px 14px; cursor: pointer; text-align: left;
}
.settings-theme-card:hover { border-color: var(--s2); }
.settings-theme-card[data-active="true"] { border-color: var(--blue); box-shadow: 0 0 0 1px var(--blue); }
.settings-theme-name { font-size: 12.5px; color: var(--text); }
.settings-theme-swatches { display: flex; gap: 5px; }
.settings-theme-dot { width: 14px; height: 14px; border-radius: 50%; border: 1px solid var(--line); }
```

- [ ] **Step 4: Verify build**

Run: `cd ui && npm run build`
Expected: build succeeds, no TS errors.

- [ ] **Step 5: Manual check**

Run the app (`cd ui && npm run dev` or the project's Tauri dev command), open Settings → Appearance, click each card. Confirm: UI recolors instantly, the active card is outlined, and the focus terminal recolors. Reload the app and confirm the chosen theme persists.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/Settings.tsx ui/src/styles.css
git commit -m "feat(ui): Appearance theme picker in Settings"
```

---

### Task 4: Light-theme audit + final verification

**Files:**
- Modify: `ui/src/theme.css` (add legacy-var aliases in `:root`)
- Modify: `ui/src/styles.css` (replace dark-only hardcoded colors)

**Interfaces:**
- Consumes: the CSS vars from Task 1.
- Produces: no remaining dark-only hardcoded colors that break the Paperback (light) theme.

- [ ] **Step 1: Find hardcoded colors**

Run:
```bash
cd ui && grep -nE "#[0-9a-fA-F]{3,8}" src/styles.css
grep -rnE "background:\s*#|color:\s*#|rgba?\(" src/components --include=*.tsx | grep -v "var(--"
```
Expected: the `styles.css` hits include `.resizer:hover { background: #313244; }`, `.run-error { color: #e06c75; }`, several `var(--muted, #9aa)` / `var(--panel, #1d1f26)` / `var(--fg, #e6e6e6)` / `var(--border, #2a2d36)` fallbacks (these vars are undefined today, so the dark fallback always wins), and `.run-preview { background: #fff; }`.

- [ ] **Step 2: Alias the legacy fallback vars to theme vars**

The undefined `--muted/--panel/--fg/--border` vars always fall back to dark literals. Define them in `ui/src/theme.css` so they track the active theme. Add these lines inside the `:root { ... }` block (next to the semantic vars):
```css
  /* legacy aliases (keep old var(--x, #fallback) usages theme-aware) */
  --muted:var(--sub0); --panel:var(--mantle); --fg:var(--text); --border:var(--s0);
```

- [ ] **Step 3: Replace the two dark-only literals in styles.css**

In `ui/src/styles.css`:

Change line 644 from:
```css
.resizer:hover { background: #313244; }
```
to:
```css
.resizer:hover { background: var(--s0); }
```

Change line 762 from:
```css
.run-error { color: #e06c75; font-size: 12px; }
```
to:
```css
.run-error { color: var(--red); font-size: 12px; }
```

Leave `.run-preview { background: #fff; }` (line 765) as-is — it is the background for embedded web-preview content, which should stay white regardless of theme.

- [ ] **Step 4: Verify build + tests**

Run: `cd ui && npm run build && npx vitest run`
Expected: build succeeds, all tests pass.

- [ ] **Step 5: Manual verification across themes**

Run the app, switch through all five themes via Settings → Appearance. Confirm in **Paperback (light)** specifically: panels, resizers, run errors, archived/review/diff-comment text (the `var(--muted/...)` consumers), and the terminal all read correctly on the light background with no leftover dark patches. Confirm statuses (running / awaiting / review / crashed) stay visually distinct in every theme.

- [ ] **Step 6: Commit**

```bash
git add ui/src/theme.css ui/src/styles.css
git commit -m "fix(ui): make legacy color fallbacks theme-aware for light themes"
```

---

## Self-Review

**Spec coverage:**
- Theme registry / single source of truth → Task 1.
- `applyTheme` (inline vars + data-theme + color-scheme + localStorage + event), `getStoredTheme`, startup application → Tasks 1 & 2.
- Terminal derives from registry + live update → Task 2.
- Semantic vars stay in `theme.css` → unchanged (verified in Task 4 edits touch only `:root` additions).
- Five new palettes with exact hex values → Task 1 (verbatim from spec).
- Light theme (Paperback) support incl. terminal + color-scheme + hardcoded-color audit → Tasks 1, 2, 4.
- Appearance settings section with swatch cards, live preview, persistence → Task 3.
- localStorage `"theme"` default `catppuccin` → Task 1 (`STORAGE_KEY`, `DEFAULT_THEME`).
- Risks (hardcoded colors, light contrast, box-shadows) → Task 4 audit + manual checks.

**Placeholder scan:** none — every code/test/command step shows full content.

**Type consistency:** `ThemeId`, `Theme`, `applyTheme`, `getStoredTheme`, `currentXtermTheme`, `xtermThemeFor`, `themeVars`, `resolveTheme`, `THEMES`, `DEFAULT_THEME`, `STORAGE_KEY` are defined in Task 1 and consumed with matching signatures in Tasks 2–3. `vars` keys match the 22 CSS var names used by the swatch render and `themeVars`.
