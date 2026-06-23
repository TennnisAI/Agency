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
