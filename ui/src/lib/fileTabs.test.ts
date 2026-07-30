import { describe, expect, it } from "vitest";
import {
  MAX_TABS, TabState, activateTab, closeTab, deserializeTabs, emptyTabs, openTab,
  removeTab, renameTab, serializeTabs, setDirtyTab,
} from "./fileTabs";

const openMany = (paths: string[]): TabState =>
  paths.reduce((s, p) => openTab(s, p), emptyTabs());

describe("openTab / activateTab", () => {
  it("appends new tabs and activates them", () => {
    const s = openMany(["a.ts", "b.ts"]);
    expect(s.open).toEqual(["a.ts", "b.ts"]);
    expect(s.active).toBe("b.ts");
  });

  it("re-opening an open tab just activates it", () => {
    const s = openTab(openMany(["a.ts", "b.ts"]), "a.ts");
    expect(s.open).toEqual(["a.ts", "b.ts"]);
    expect(s.active).toBe("a.ts");
  });

  it("activation updates recency", () => {
    let s = openMany(["a.ts", "b.ts", "c.ts"]);
    s = activateTab(s, "a.ts");
    expect(s.recency[s.recency.length - 1]).toBe("a.ts");
  });
});

describe("closeTab", () => {
  it("closing the active tab picks the nearest right neighbor, else left", () => {
    let s = openMany(["a.ts", "b.ts", "c.ts"]);
    s = activateTab(s, "b.ts");
    s = closeTab(s, "b.ts");
    expect(s.open).toEqual(["a.ts", "c.ts"]);
    expect(s.active).toBe("c.ts");
    s = closeTab(s, "c.ts");
    expect(s.active).toBe("a.ts");
    s = closeTab(s, "a.ts");
    expect(s.active).toBeNull();
    expect(s.open).toEqual([]);
  });

  it("closing an inactive tab keeps the active one", () => {
    const s = closeTab(openMany(["a.ts", "b.ts"]), "a.ts");
    expect(s.active).toBe("b.ts");
  });

  it("clears dirty and recency for the closed path", () => {
    let s = setDirtyTab(openMany(["a.ts", "b.ts"]), "a.ts", true);
    s = closeTab(s, "a.ts");
    expect(s.dirty.has("a.ts")).toBe(false);
    expect(s.recency).not.toContain("a.ts");
  });
});

describe("LRU eviction", () => {
  it("opening past the cap evicts the least-recently-activated clean tab", () => {
    let s = openMany(Array.from({ length: MAX_TABS }, (_, i) => `f${i}.ts`));
    s = activateTab(s, "f0.ts"); // f0 is now recent; f1 becomes LRU
    s = openTab(s, "new.ts");
    expect(s.open).toHaveLength(MAX_TABS);
    expect(s.open).not.toContain("f1.ts");
    expect(s.open).toContain("new.ts");
    expect(s.active).toBe("new.ts");
  });

  it("never evicts dirty tabs — the cap is exceeded instead", () => {
    let s = openMany(Array.from({ length: MAX_TABS }, (_, i) => `f${i}.ts`));
    for (let i = 0; i < MAX_TABS; i++) s = setDirtyTab(s, `f${i}.ts`, true);
    s = openTab(s, "new.ts");
    expect(s.open).toHaveLength(MAX_TABS + 1);
  });

  it("never evicts the active tab", () => {
    let s = openMany(Array.from({ length: MAX_TABS }, (_, i) => `f${i}.ts`));
    for (let i = 1; i < MAX_TABS; i++) s = setDirtyTab(s, `f${i}.ts`, true);
    s = activateTab(s, "f0.ts"); // only clean tab is also the active one
    s = openTab(s, "new.ts");
    expect(s.open).toContain("f0.ts");
    expect(s.open).toHaveLength(MAX_TABS + 1);
  });
});

describe("renameTab / removeTab", () => {
  it("retargets a renamed file everywhere", () => {
    let s = setDirtyTab(openMany(["src/a.ts", "b.ts"]), "src/a.ts", true);
    s = activateTab(s, "src/a.ts");
    s = renameTab(s, "src/a.ts", "src/z.ts");
    expect(s.open).toEqual(["src/z.ts", "b.ts"]);
    expect(s.active).toBe("src/z.ts");
    expect(s.dirty.has("src/z.ts")).toBe(true);
    expect(s.recency).toContain("src/z.ts");
  });

  it("retargets everything under a renamed directory", () => {
    const s = renameTab(openMany(["src/a.ts", "src/deep/b.ts", "other.ts"]), "src", "lib");
    expect(s.open).toEqual(["lib/a.ts", "lib/deep/b.ts", "other.ts"]);
  });

  it("removeTab closes the file or a whole deleted directory", () => {
    let s = openMany(["src/a.ts", "src/b.ts", "other.ts"]);
    s = activateTab(s, "src/a.ts");
    s = removeTab(s, "src");
    expect(s.open).toEqual(["other.ts"]);
    expect(s.active).toBe("other.ts");
  });
});

describe("serialize / deserialize", () => {
  it("round-trips open + active, dropping dirty and recency", () => {
    let s = setDirtyTab(openMany(["a.ts", "b.ts"]), "a.ts", true);
    s = activateTab(s, "a.ts");
    const restored = deserializeTabs(serializeTabs(s));
    expect(restored.open).toEqual(["a.ts", "b.ts"]);
    expect(restored.active).toBe("a.ts");
    expect(restored.dirty.size).toBe(0);
    expect(restored.recency).toEqual([]);
  });

  it("tolerates garbage", () => {
    expect(deserializeTabs(null).open).toEqual([]);
    expect(deserializeTabs("not json").open).toEqual([]);
    expect(deserializeTabs('{"open": "nope"}').open).toEqual([]);
    expect(deserializeTabs('{"open": ["a"], "active": "ghost"}').active).toBeNull();
  });
});
