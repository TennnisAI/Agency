import { describe, expect, it } from "vitest";
import { APP_MENUS, MenuItemSpec, chordMatches, itemEnabled, itemForKey } from "./appMenu";
import menuRs from "../../../crates/agency-app/src/menu.rs?raw";

const key = (k: string, mods: Partial<{ ctrl: boolean; meta: boolean; shift: boolean; alt: boolean }> = {}) => ({
  key: k,
  ctrlKey: mods.ctrl ?? false,
  metaKey: mods.meta ?? false,
  shiftKey: mods.shift ?? false,
  altKey: mods.alt ?? false,
});

const items = (): MenuItemSpec[] =>
  APP_MENUS.flatMap((m) => m.items).filter((i): i is MenuItemSpec => i.kind !== "separator");

describe("chordMatches", () => {
  it("reads ⌘ as Ctrl on the platforms this menu is drawn on, never as Super", () => {
    expect(chordMatches(key("t", { ctrl: true }), "⌘T")).toBe(true);
    expect(chordMatches(key("t", { meta: true }), "⌘T")).toBe(false);
    expect(chordMatches(key("t", { ctrl: true, meta: true }), "⌘T")).toBe(false);
    expect(chordMatches(key("t"), "⌘T")).toBe(false);
  });

  it("wants every modifier the chord names and no other", () => {
    expect(chordMatches(key("g", { ctrl: true, shift: true }), "⌘⇧G")).toBe(true);
    expect(chordMatches(key("g", { ctrl: true }), "⌘⇧G")).toBe(false);
    expect(chordMatches(key("g", { ctrl: true, shift: true }), "⌘G")).toBe(false);
    expect(chordMatches(key("f", { ctrl: true, alt: true }), "⌥⌘F")).toBe(true);
    expect(chordMatches(key("Enter", { ctrl: true }), "⌘↵")).toBe(true);
    expect(chordMatches(key("F11"), "F11")).toBe(true);
  });

  it("is case-insensitive on the key, as a shifted letter reports upper case", () => {
    expect(chordMatches(key("O", { ctrl: true, shift: true }), "⌘⇧O")).toBe(true);
  });
});

describe("itemForKey", () => {
  it("finds the chords the binder owns and skips the ones bound elsewhere", () => {
    expect(itemForKey(key("t", { ctrl: true }))?.action).toBe("new-terminal");
    expect(itemForKey(key("b", { ctrl: true }))?.action).toBe("toggle-sidebar");
    expect(itemForKey(key("q", { ctrl: true }))?.action).toBe("quit");
    // ⌘K is useShortcuts' and ⌘C is the webview's: neither may fire twice.
    expect(itemForKey(key("k", { ctrl: true }))).toBeNull();
    expect(itemForKey(key("c", { ctrl: true }))).toBeNull();
    expect(itemForKey(key("x"))).toBeNull();
  });
});

describe("the menu model", () => {
  it("names every action App.onMenu routes and no duplicates", () => {
    const actions = items().map((i) => i.action);
    expect(new Set(actions).size).toBe(actions.length - 1); // Close Window sits in File and Window, like the native one.
    for (const a of ["settings", "new-agent", "add-project", "init-repo", "find-next", "palette", "approve", "report-issue"]) {
      expect(actions).toContain(a);
    }
  });

  it("gates items on the selection the way the native menu does", () => {
    const ctx = { project: false, focusedAgent: false, gitless: false };
    const byAction = Object.fromEntries(items().map((i) => [i.action, i]));
    expect(itemEnabled(byAction["new-agent"], ctx)).toBe(false);
    expect(itemEnabled(byAction["new-agent"], { ...ctx, project: true })).toBe(true);
    expect(itemEnabled(byAction["init-repo"], { ...ctx, project: true })).toBe(false);
    expect(itemEnabled(byAction["init-repo"], { ...ctx, project: true, gitless: true })).toBe(true);
    expect(itemEnabled(byAction["archive"], { ...ctx, focusedAgent: true })).toBe(true);
    expect(itemEnabled(byAction["add-project"], ctx)).toBe(true);
  });

  it("marks every chord the app already binds so the binder leaves it alone", () => {
    // useShortcuts.ts binds K , ⇧D ↵ N D; App.tsx binds plain ⌘F; the webview
    // owns the editing chords. A chord in this list without the flag would
    // fire its action twice on Linux.
    const elsewhere = ["⌘K", "⌘,", "⌘⇧D", "⌘↵", "⌘N", "⌘D", "⌘F", "⌘Z", "⌘⇧Z", "⌘X", "⌘C", "⌘V", "⌘A", "⌘W"];
    for (const i of items()) {
      if (i.accel && elsewhere.includes(i.accel)) expect(i.boundElsewhere, i.label).toBe(true);
      if (i.accel && !elsewhere.includes(i.accel)) expect(i.boundElsewhere ?? false, i.label).toBe(false);
    }
  });
});

describe("parity with the native menu", () => {
  it("carries every menu.rs action, so Linux users get the same menu", () => {
    const menuIds = [...new Set([...menuRs.matchAll(/"menu:([a-z-]+)"/g)].map((m) => m[1]))];
    expect(menuIds.length).toBeGreaterThan(10);
    const actions = new Set(items().map((i) => i.action));
    for (const id of menuIds) expect(actions, `menu.rs action "${id}" missing from APP_MENUS`).toContain(id);
  });

  it("spells each accelerator the way menu.rs does", () => {
    // menu.rs: `with_id("menu:x", …).accelerator("CmdOrCtrl+Shift+D")`.
    const native = new Map<string, string>();
    for (const m of menuRs.matchAll(/"menu:([a-z-]+)", "[^"]*"\)\s*\.accelerator\("([^"]+)"\)/g)) native.set(m[1], m[2]);
    expect(native.size).toBeGreaterThan(5);
    const spell = (accel: string) =>
      accel
        .replace("⌘", "CmdOrCtrl+")
        .replace("⇧", "Shift+")
        .replace("⌥", "Alt+")
        .replace("↵", "Return");
    for (const i of items()) {
      const theirs = native.get(i.action);
      if (!theirs || !i.accel) continue;
      // Modifier order differs between the two spellings; compare as sets.
      const norm = (s: string) => s.split("+").sort().join("+");
      expect(norm(spell(i.accel)), i.label).toBe(norm(theirs));
    }
  });
});
