import { describe, expect, it } from "vitest";
import { blockGeometry, bounds, ground, IsoNode, place, proj, route, TILE_H, TILE_W } from "./isomap";
import { LevelNode } from "./maplayout";

const node = (key: string, kind: "dir" | "file", files: number, symbols: number): LevelNode => ({
  key,
  kind,
  name: key,
  files,
  symbols,
  x: 0,
  y: 0,
  w: 0,
  h: 0,
});

const iso = (gx: number, gy: number, over: Partial<IsoNode> = {}): IsoNode => ({
  node: node(`n${gx}-${gy}`, "file", 1, 4),
  gx,
  gy,
  base: 0.6,
  floors: 1,
  floorH: 12,
  ...over,
});

describe("proj", () => {
  it("maps grid axes to the two screen diagonals", () => {
    expect(proj(0, 0)).toEqual({ x: 0, y: 0 });
    expect(proj(1, 0)).toEqual({ x: TILE_W / 2, y: TILE_H / 2 });
    expect(proj(0, 1)).toEqual({ x: -TILE_W / 2, y: TILE_H / 2 });
    expect(proj(1, 1)).toEqual({ x: 0, y: TILE_H });
  });
});

describe("place", () => {
  const level = () => [
    node("big/", "dir", 40, 300),
    node("small/", "dir", 2, 9),
    node("hub.ts", "file", 1, 100),
    node("a.ts", "file", 1, 25),
    node("b.ts", "file", 1, 4),
    node("c.ts", "file", 1, 1),
  ];

  it("gives every node its own integer cell, deterministically", () => {
    const a = place(level(), []);
    const b = place(level(), []);
    expect(a.length).toBe(6);
    expect(a.map((p) => [p.node.key, p.gx, p.gy])).toEqual(b.map((p) => [p.node.key, p.gx, p.gy]));
    const cells = new Set(a.map((p) => `${p.gx},${p.gy}`));
    expect(cells.size).toBe(6);
    for (const p of a) {
      expect(Number.isInteger(p.gx)).toBe(true);
      expect(Number.isInteger(p.gy)).toBe(true);
    }
  });

  it("shapes files as single slabs and directories as stacks", () => {
    const placed = place(level(), []);
    const by = new Map(placed.map((p) => [p.node.key, p]));
    expect(by.get("hub.ts")!.floors).toBe(1);
    // One floor per doubling: 40 files is 1 + floor(log2(40)) = 6, capped at
    // 5; 2 files is the 2-floor minimum stack.
    expect(by.get("big/")!.floors).toBe(5);
    expect(by.get("small/")!.floors).toBe(2);
    // The biggest symbol count gets the biggest footprint.
    expect(by.get("big/")!.base).toBeGreaterThan(by.get("small/")!.base);
    expect(by.get("hub.ts")!.base).toBeGreaterThan(by.get("c.ts")!.base);
    expect(by.get("hub.ts")!.floorH).toBeGreaterThan(by.get("c.ts")!.floorH);
  });

  it("draws back rows before front rows", () => {
    const placed = place(level(), []);
    const depths = placed.map((p) => p.gx + p.gy);
    expect([...depths].sort((a, b) => a - b)).toEqual(depths);
  });

  it("survives an empty level", () => {
    expect(place([], [])).toEqual([]);
    expect(bounds([])).toBe("0 0 100 100");
  });
});

describe("blockGeometry", () => {
  it("stacks slabs and anchors the label above the top", () => {
    const g = blockGeometry(iso(2, 1, { floors: 3, floorH: 9 }));
    expect(g.slabs).toHaveLength(3);
    for (const s of g.slabs) {
      expect(s.top.split(" ")).toHaveLength(4);
      expect(s.left.split(" ")).toHaveLength(4);
      expect(s.right.split(" ")).toHaveLength(4);
    }
    const { y } = proj(2, 1);
    // 3 floors of 9 with two 3px gaps, above the half-diamond.
    expect(g.topY).toBeCloseTo(y - (0.6 * TILE_H) / 2 - 33, 1);
    expect(g.cy).toBe(y);
  });
});

describe("route", () => {
  it("runs straight when aligned and L-shaped otherwise", () => {
    const straight = route(iso(0, 0), iso(3, 0));
    expect(straight.d).not.toContain("Q");
    const bent = route(iso(0, 0), iso(2, 2));
    expect(bent.d).toContain("Q");
    expect(bent.arrow.split(" ")).toHaveLength(3);
    expect(bent.dur).toBeGreaterThanOrEqual(3.5);
    expect(bent.dur).toBeLessThanOrEqual(10);
  });

  it("takes different corners in opposite directions", () => {
    const ab = route(iso(0, 0), iso(2, 2));
    const ba = route(iso(2, 2), iso(0, 0));
    expect(ab.d).not.toBe(ba.d);
  });
});

describe("ground", () => {
  it("wraps the placed cells in a diamond with a grid", () => {
    const g = ground([iso(0, 0), iso(2, 1)]);
    expect(g.outer.split(" ")).toHaveLength(4);
    expect(g.inner.split(" ")).toHaveLength(4);
    // Cells span 0..2 and 0..1 plus margins: at least a line per axis step.
    expect(g.lines.length).toBeGreaterThanOrEqual(5);
  });

  it("bounds cover the blocks with label headroom", () => {
    const placed = [iso(0, 0), iso(2, 1, { floors: 5, floorH: 9 })];
    const [x, y, w, h] = bounds(placed).split(" ").map(Number);
    expect(w).toBeGreaterThan(0);
    expect(h).toBeGreaterThan(0);
    const top = blockGeometry(placed[1]).topY;
    expect(y).toBeLessThan(top - 30);
    expect(x + w).toBeGreaterThan(proj(2, 1).x + (0.6 * TILE_W) / 2);
  });
});
