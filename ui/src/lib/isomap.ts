// Isometric geometry for the Map tab. maplayout.ts decides what a level
// contains and roughly where things want to sit; this module turns that into
// a small city: force positions snapped to an isometric ground grid, files as
// low halls and directories as stacked keeps (one floor per doubling of their
// files), dependencies as roads routed along the grid axes. Pure math, no
// DOM, deterministic throughout — the same level always builds the same city.

import { layout, LevelEdge, LevelNode } from "./maplayout";

/** One ground cell is a 2:1 diamond, the classic isometric tile. */
export const TILE_W = 96;
export const TILE_H = 48;

/** Grid cell to screen center. */
export function proj(gx: number, gy: number): { x: number; y: number } {
  return { x: (gx - gy) * (TILE_W / 2), y: (gx + gy) * (TILE_H / 2) };
}

export interface IsoNode {
  node: LevelNode;
  gx: number;
  gy: number;
  /** Footprint as a fraction of the tile, from the node's symbol weight. */
  base: number;
  /** Stacked slabs: 1 for files, 2..5 for directories by file count. */
  floors: number;
  /** Screen height of one slab. */
  floorH: number;
}

/**
 * Place a level on the grid. The force layout still decides neighborhoods
 * (connected boxes drift together); each node then takes the nearest free
 * cell, biggest-first from the middle out, so the heart of the level builds
 * the center of the city.
 */
export function place(nodes: LevelNode[], edges: LevelEdge[]): IsoNode[] {
  if (nodes.length === 0) return [];
  // Uniform footprints for the simulation: the grid handles real sizes.
  for (const n of nodes) {
    n.w = 110;
    n.h = 110;
  }
  layout(nodes, edges);

  // Normalize the force positions onto grid coordinates.
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const n of nodes) {
    minX = Math.min(minX, n.x);
    minY = Math.min(minY, n.y);
    maxX = Math.max(maxX, n.x);
    maxY = Math.max(maxY, n.y);
  }
  const span = Math.max(1, Math.ceil(Math.sqrt(nodes.length) * 1.6));
  const sx = (maxX - minX) || 1;
  const sy = (maxY - minY) || 1;
  const want = nodes.map((n) => ({
    n,
    gx: ((n.x - minX) / sx) * span,
    gy: ((n.y - minY) / sy) * span,
  }));

  // Free cells, center-out in deterministic ring order, comfortably more
  // than the level needs.
  const c = span / 2;
  const mid = Math.round(c);
  const cells: { gx: number; gy: number }[] = [];
  for (let radius = 0; cells.length < nodes.length * 2; radius++) {
    const ring: { gx: number; gy: number }[] = [];
    for (let gx = mid - radius; gx <= mid + radius; gx++) {
      for (let gy = mid - radius; gy <= mid + radius; gy++) {
        if (Math.max(Math.abs(gx - mid), Math.abs(gy - mid)) === radius) {
          ring.push({ gx, gy });
        }
      }
    }
    ring.sort((a, b) => a.gy - b.gy || a.gx - b.gx);
    cells.push(...ring);
  }

  // Nearest free cell, nodes ordered center-out by wanted position. Ties on
  // distance fall back to ring order so the result never depends on float
  // iteration quirks.
  const order = [...want].sort(
    (a, b) =>
      Math.hypot(a.gx - c, a.gy - c) - Math.hypot(b.gx - c, b.gy - c) ||
      a.n.key.localeCompare(b.n.key),
  );
  const taken = new Set<number>();
  const maxSym = Math.max(1, ...nodes.map((n) => n.symbols));
  const placed: IsoNode[] = [];
  for (const w of order) {
    let best = -1;
    let bestD = Infinity;
    for (let i = 0; i < cells.length; i++) {
      if (taken.has(i)) continue;
      const d = (cells[i].gx - w.gx) ** 2 + (cells[i].gy - w.gy) ** 2;
      if (d < bestD - 1e-9) {
        bestD = d;
        best = i;
      }
    }
    taken.add(best);
    const cell = cells[best];
    const weight = Math.sqrt(w.n.symbols / maxSym);
    // Footprints cap well under a full tile: neighboring cells must keep a
    // visible lane of ground between blocks or the city fuses into terrain.
    placed.push(
      w.n.kind === "dir"
        ? {
            node: w.n,
            gx: cell.gx,
            gy: cell.gy,
            base: 0.56 + 0.26 * weight,
            floors: Math.max(2, Math.min(5, 1 + Math.floor(Math.log2(Math.max(1, w.n.files))))),
            floorH: 9,
          }
        : {
            node: w.n,
            gx: cell.gx,
            gy: cell.gy,
            base: 0.42 + 0.3 * weight,
            floors: 1,
            floorH: 8 + Math.round(16 * weight),
          },
    );
  }
  // Painter's order: back rows first, so nearer blocks draw over them.
  placed.sort((a, b) => a.gx + a.gy - (b.gx + b.gy) || a.gx - b.gx);
  return placed;
}

export interface Slab {
  top: string;
  left: string;
  right: string;
}

/** The block's slab polygons (bottom to top) and its label anchor. */
export function blockGeometry(iso: IsoNode): {
  slabs: Slab[];
  cx: number;
  cy: number;
  topY: number;
} {
  const { x, y } = proj(iso.gx, iso.gy);
  const hw = (iso.base * TILE_W) / 2;
  const hh = (iso.base * TILE_H) / 2;
  const gap = 3;
  const slabs: Slab[] = [];
  let e0 = 0;
  for (let f = 0; f < iso.floors; f++) {
    const e1 = e0 + iso.floorH;
    slabs.push(slab(x, y, hw, hh, e0, e1));
    e0 = e1 + gap;
  }
  return { slabs, cx: x, cy: y, topY: y - hh - (e0 - gap) };
}

function slab(x: number, y: number, hw: number, hh: number, e0: number, e1: number): Slab {
  const pt = (px: number, py: number) => `${r(px)},${r(py)}`;
  return {
    top: [pt(x, y - hh - e1), pt(x + hw, y - e1), pt(x, y + hh - e1), pt(x - hw, y - e1)].join(" "),
    left: [pt(x - hw, y - e1), pt(x, y + hh - e1), pt(x, y + hh - e0), pt(x - hw, y - e0)].join(" "),
    right: [pt(x + hw, y - e1), pt(x, y + hh - e1), pt(x, y + hh - e0), pt(x + hw, y - e0)].join(" "),
  };
}

/**
 * A road from block a to block b: along the grid's x axis, one rounded
 * corner, then the y axis. The reverse direction takes the other corner, so
 * a two-way pair reads as two separate roads. `dur` paces the traveling
 * data dot at a constant speed regardless of distance.
 */
export function route(a: IsoNode, b: IsoNode): { d: string; arrow: string; dur: number } {
  const p0 = proj(a.gx, a.gy);
  const p2 = proj(b.gx, b.gy);
  const corner = proj(b.gx, a.gy);
  const straight = a.gx === b.gx || a.gy === b.gy;
  if (straight) {
    const len = Math.hypot(p2.x - p0.x, p2.y - p0.y);
    return {
      d: `M ${r(p0.x)} ${r(p0.y)} L ${r(p2.x)} ${r(p2.y)}`,
      arrow: chevron(p0, p2, 0.62),
      dur: dur(len),
    };
  }
  const l1 = Math.hypot(corner.x - p0.x, corner.y - p0.y);
  const l2 = Math.hypot(p2.x - corner.x, p2.y - corner.y);
  const rad = Math.min(10, l1 / 3, l2 / 3);
  const a1 = lerp(corner, p0, rad / l1);
  const b1 = lerp(corner, p2, rad / l2);
  return {
    d:
      `M ${r(p0.x)} ${r(p0.y)} L ${r(a1.x)} ${r(a1.y)} ` +
      `Q ${r(corner.x)} ${r(corner.y)} ${r(b1.x)} ${r(b1.y)} L ${r(p2.x)} ${r(p2.y)}`,
    arrow: chevron(corner, p2, 0.6),
    dur: dur(l1 + l2),
  };
}

function dur(len: number): number {
  return Math.min(10, Math.max(3.5, len / 55));
}

function lerp(from: { x: number; y: number }, to: { x: number; y: number }, t: number) {
  return { x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t };
}

function chevron(from: { x: number; y: number }, to: { x: number; y: number }, t: number): string {
  const qx = from.x + (to.x - from.x) * t;
  const qy = from.y + (to.y - from.y) * t;
  const len = Math.hypot(to.x - from.x, to.y - from.y) || 1;
  const ux = (to.x - from.x) / len;
  const uy = (to.y - from.y) / len;
  const pt = (px: number, py: number) => `${r(px)},${r(py)}`;
  return [
    pt(qx + ux * 5, qy + uy * 5),
    pt(qx - ux * 3 - uy * 4, qy - uy * 3 + ux * 4),
    pt(qx - ux * 3 + uy * 4, qy - uy * 3 - ux * 4),
  ].join(" ");
}

/** The courtyard: a ground diamond under the whole level, plus its grid. */
export function ground(placed: IsoNode[]): {
  outer: string;
  inner: string;
  lines: { x1: number; y1: number; x2: number; y2: number }[];
} {
  let minGx = Infinity;
  let minGy = Infinity;
  let maxGx = -Infinity;
  let maxGy = -Infinity;
  for (const p of placed) {
    minGx = Math.min(minGx, p.gx);
    minGy = Math.min(minGy, p.gy);
    maxGx = Math.max(maxGx, p.gx);
    maxGy = Math.max(maxGy, p.gy);
  }
  const diamond = (m: number) => {
    const corners = [
      proj(minGx - m, minGy - m),
      proj(maxGx + m, minGy - m),
      proj(maxGx + m, maxGy + m),
      proj(minGx - m, maxGy + m),
    ];
    return corners.map((p) => `${r(p.x)},${r(p.y)}`).join(" ");
  };
  const m = 0.85;
  const lines: { x1: number; y1: number; x2: number; y2: number }[] = [];
  for (let gx = Math.ceil(minGx - m) - 0.5; gx <= maxGx + m; gx++) {
    const p1 = proj(gx, minGy - m);
    const p2 = proj(gx, maxGy + m);
    lines.push({ x1: r(p1.x), y1: r(p1.y), x2: r(p2.x), y2: r(p2.y) });
  }
  for (let gy = Math.ceil(minGy - m) - 0.5; gy <= maxGy + m; gy++) {
    const p1 = proj(minGx - m, gy);
    const p2 = proj(maxGx + m, gy);
    lines.push({ x1: r(p1.x), y1: r(p1.y), x2: r(p2.x), y2: r(p2.y) });
  }
  return { outer: diamond(m), inner: diamond(m - 0.28), lines };
}

/** Tight bounds around the city, with headroom for the floating labels. */
export function bounds(placed: IsoNode[], pad = 48): string {
  if (placed.length === 0) return "0 0 100 100";
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const p of placed) {
    const g = blockGeometry(p);
    const hw = (p.base * TILE_W) / 2;
    const hh = (p.base * TILE_H) / 2;
    minX = Math.min(minX, g.cx - hw);
    maxX = Math.max(maxX, g.cx + hw);
    minY = Math.min(minY, g.topY - 30);
    maxY = Math.max(maxY, g.cy + hh);
  }
  return `${r(minX - pad)} ${r(minY - pad)} ${r(maxX - minX + pad * 2)} ${r(maxY - minY + pad * 2)}`;
}

function r(v: number): number {
  return Math.round(v * 10) / 10;
}
