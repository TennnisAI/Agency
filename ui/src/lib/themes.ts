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
      o0: "#574c43", o1: "#4a4039", o2: "#3b322c",
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

/** Explicit monospace stack for the embedded terminal. xterm's built-in default
 *  leads with `courier-new`, which lacks the quadrant block elements (U+2596–259F)
 *  used by TUI logos; pinning Menlo first (full box-drawing/block coverage) avoids
 *  relying on WebKit's per-glyph font fallback. Defensive — the actual bundle logo
 *  breakage was a missing UTF-8 locale (see pathenv::repair), not the font. */
export const TERMINAL_FONT_FAMILY = "Menlo, Monaco, 'Courier New', monospace";

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

/** The active theme's light/dark scheme — used to pick matching syntax-highlight
 *  and other canvas-rendered colors that can't read CSS vars. */
export function currentScheme(): "dark" | "light" {
  return resolveTheme(active).scheme;
}

/** Minimum contrast ratio to enforce on terminal text for the active theme.
 *  Light themes sit on a pale background where dim/faint ANSI text — and the
 *  light accent colors TUIs emit assuming a dark background (e.g. pi's cyan
 *  command hints) — wash out to near-invisibility. Telling xterm a minimum
 *  ratio makes it lift any sub-contrast foreground to a readable level against
 *  the background; xterm leaves already-readable colors untouched. Dark themes
 *  separate cleanly, so we disable the adjustment there (1 = no change). */
export function minContrastRatio(): number {
  return currentScheme() === "light" ? 4.5 : 1;
}

/** The active theme's id — used to pick the matching Shiki highlight theme so
 *  diff colors are drawn from the same palette (and the same background-tuned
 *  contrast) as the rest of the UI. */
export function currentThemeId(): ThemeId {
  return active;
}
