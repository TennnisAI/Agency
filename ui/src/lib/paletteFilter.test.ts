import { describe, expect, it } from "vitest";
import { filterEntries, PaletteEntry } from "./paletteFilter";

const E: PaletteEntry[] = [
  { kind: "project", id: "p1", projectId: "p1", label: "Senba", sublabel: "/repo/senba" },
  { kind: "run", id: "r1", projectId: "p1", label: "claude: fix the bug", sublabel: "agent/r1" },
  { kind: "run", id: "r2", projectId: "p1", label: "pi: add tests", sublabel: "agent/r2" },
];

describe("filterEntries", () => {
  it("returns all when query empty", () => {
    expect(filterEntries("", E)).toHaveLength(3);
  });
  it("matches case-insensitively on label", () => {
    const r = filterEntries("FIX", E);
    expect(r.map((e) => e.id)).toEqual(["r1"]);
  });
  it("matches on sublabel", () => {
    const r = filterEntries("senba", E);
    expect(r.map((e) => e.id)).toContain("p1");
  });
  it("matches in-order subsequences as the weakest tier", () => {
    const r = filterEntries("ftb", E); // f…t…b in "claude: fix the bug"
    expect(r.map((e) => e.id)).toEqual(["r1"]);
    const mixed = filterEntries("se", E); // "Senba" sublabel-substring beats any subsequence
    expect(mixed[0].id).toBe("p1");
  });
  it("matches haystack after sublabel, before fuzzy", () => {
    const entries: PaletteEntry[] = [
      { kind: "issue", id: "h", projectId: "p", label: "AGE-1 fix crash", sublabel: "Agency · Todo", haystack: "repro: resize the pane" },
      { kind: "issue", id: "f", projectId: "p", label: "roster page", sublabel: "Agency · Todo" },
    ];
    // "resize" hits only the haystack.
    expect(filterEntries("resize", entries).map((e) => e.id)).toEqual(["h"]);
    // A haystack substring outranks a fuzzy label subsequence ("rop" ⊂ "roster page").
    const both: PaletteEntry[] = [
      { kind: "issue", id: "fz", projectId: "p", label: "roster page", sublabel: "x" },
      { kind: "issue", id: "hy", projectId: "p", label: "AGE-2 other", sublabel: "x", haystack: "drop the cache" },
    ];
    expect(filterEntries("rop", both).map((e) => e.id)).toEqual(["hy", "fz"]);
  });
  it("orders label-prefix matches before substring matches", () => {
    const entries: PaletteEntry[] = [
      { kind: "run", id: "a", projectId: "p", label: "refactor parser", sublabel: "agent/a" },
      { kind: "run", id: "b", projectId: "p", label: "parser cleanup", sublabel: "agent/b" },
    ];
    const r = filterEntries("parser", entries);
    expect(r.map((e) => e.id)).toEqual(["b", "a"]); // "parser…" prefix first
  });
});
