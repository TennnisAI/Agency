import { describe, expect, it } from "vitest";
import { loadRecency, recentByPrefix, recencyIndex, recordActivation } from "./recency";

function fakeStore(): Pick<Storage, "getItem" | "setItem"> & { map: Map<string, string> } {
  const map = new Map<string, string>();
  return {
    map,
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { map.set(k, v); },
  };
}

describe("recency", () => {
  it("records newest first and dedupes by key", () => {
    const s = fakeStore();
    recordActivation("cmd:settings", "Settings…", "", s, 1);
    recordActivation("project:p1", "Senba", "/repo", s, 2);
    recordActivation("cmd:settings", "Settings…", "", s, 3);
    const list = loadRecency(s);
    expect(list.map((e) => e.key)).toEqual(["cmd:settings", "project:p1"]);
    expect(list[0].t).toBe(3);
  });

  it("caps at 100 entries, evicting the oldest", () => {
    const s = fakeStore();
    for (let i = 0; i < 105; i++) recordActivation(`cmd:c${i}`, `C${i}`, "", s, i);
    const list = loadRecency(s);
    expect(list).toHaveLength(100);
    expect(list[0].key).toBe("cmd:c104");
    expect(list.some((e) => e.key === "cmd:c4")).toBe(false);
  });

  it("filters by prefix with a cap", () => {
    const s = fakeStore();
    recordActivation("note:p1:a.md", "A", "a.md", s, 1);
    recordActivation("file:p1:src/x.ts", "x.ts", "src/x.ts", s, 2);
    recordActivation("note:p1:b.md", "B", "b.md", s, 3);
    expect(recentByPrefix("note:", 5, s).map((e) => e.label)).toEqual(["B", "A"]);
    expect(recentByPrefix("note:", 1, s)).toHaveLength(1);
  });

  it("indexes key → rank, 0 = most recent", () => {
    const s = fakeStore();
    recordActivation("a", "A", "", s, 1);
    recordActivation("b", "B", "", s, 2);
    const idx = recencyIndex(s);
    expect(idx.get("b")).toBe(0);
    expect(idx.get("a")).toBe(1);
  });

  it("swallows storage failures and corrupt payloads", () => {
    const broken = {
      getItem: () => { throw new Error("nope"); },
      setItem: () => { throw new Error("nope"); },
    };
    expect(() => recordActivation("a", "A", "", broken, 1)).not.toThrow();
    expect(loadRecency(broken)).toEqual([]);
    const corrupt = fakeStore();
    corrupt.map.set("palette:recency:v1", "{not json");
    expect(loadRecency(corrupt)).toEqual([]);
    const wrongShape = fakeStore();
    wrongShape.map.set("palette:recency:v1", JSON.stringify([{ key: 1 }, { key: "k", label: "L", sub: "", t: 5 }]));
    expect(loadRecency(wrongShape).map((e) => e.key)).toEqual(["k"]);
  });

  it("is a no-op without storage", () => {
    expect(() => recordActivation("a", "A", "", null, 1)).not.toThrow();
    expect(loadRecency(null)).toEqual([]);
    expect(recencyIndex(null).size).toBe(0);
  });
});
