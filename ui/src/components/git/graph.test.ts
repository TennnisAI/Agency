import { describe, expect, it } from "vitest";
import { computeGraph } from "./graph";

describe("computeGraph", () => {
  it("linear history uses one lane", () => {
    const rows = computeGraph([
      { hash: "c", parents: ["b"] },
      { hash: "b", parents: ["a"] },
      { hash: "a", parents: [] },
    ]);
    expect(rows[0].circleIndex).toBe(0);
    expect(rows[0].output).toEqual([{ target: "b", color: rows[0].color }]);
    expect(rows[1].output).toEqual([{ target: "a", color: rows[1].color }]);
    expect(rows[2].output).toEqual([]); // root: no parent lane
  });

  it("a merge commit adds a second lane for its second parent", () => {
    const rows = computeGraph([
      { hash: "m", parents: ["a", "b"] },
      { hash: "b", parents: ["root"] },
      { hash: "a", parents: ["root"] },
    ]);
    expect(rows[0].output.map((l) => l.target)).toEqual(["a", "b"]);
    // by the time we reach 'a', its lane is consumed and continues to root
    expect(rows[2].circleIndex).toBe(0);
  });

  it("lanes converging on the same commit collapse to one", () => {
    const rows = computeGraph([
      { hash: "a", parents: ["root"] },
      { hash: "root", parents: [] },
    ]);
    // 'a' is a tip -> appends lane to root; root consumes it and has no parents
    expect(rows[0].output).toEqual([{ target: "root", color: rows[0].color }]);
    expect(rows[1].output).toEqual([]);
  });

  it("emits enter/exit segments that join rails to the dot", () => {
    const rows = computeGraph([
      { hash: "c", parents: ["b"] },
      { hash: "b", parents: ["a"] },
      { hash: "a", parents: [] },
    ]);
    // Tip: only an exit rail leaving the dot toward its parent.
    expect(rows[0].segments).toEqual([
      { kind: "exit", from: 0, to: 0, color: rows[0].color },
    ]);
    // Middle commit: rail enters the dot from above and exits below.
    expect(rows[1].segments).toEqual([
      { kind: "enter", from: 0, to: 0, color: rows[1].color },
      { kind: "exit", from: 0, to: 0, color: rows[1].color },
    ]);
    // Root: rail enters, nothing leaves.
    expect(rows[2].segments).toEqual([
      { kind: "enter", from: 0, to: 0, color: rows[2].color },
    ]);
  });

  it("merge commit: second parent exits diagonally to a new lane", () => {
    const rows = computeGraph([
      { hash: "m", parents: ["a", "b"] },
      { hash: "b", parents: ["root"] },
      { hash: "a", parents: ["root"] },
      { hash: "root", parents: [] },
    ]);
    expect(rows[0].isMerge).toBe(true);
    // Merge dot at column 0 exits to column 0 (first parent) and column 1 (second).
    expect(rows[0].segments).toContainEqual({ kind: "exit", from: 0, to: 0, color: rows[0].color });
    expect(rows[0].segments.filter((s) => s.kind === "exit")).toHaveLength(2);
    expect(rows[0].segments.find((s) => s.kind === "exit" && s.to === 1)).toBeTruthy();
    // Row for 'b' (dot in lane 1): lane 0 passes straight through it.
    expect(rows[1].circleIndex).toBe(1);
    expect(rows[1].segments).toContainEqual({ kind: "pass", from: 0, to: 0, color: rows[1].input[0].color });
    // Second lane bends into 'root' when it collapses: an enter from column 1 to column 0.
    expect(rows[3].segments).toContainEqual({ kind: "enter", from: 1, to: 0, color: rows[3].input[1].color });
    // Column counts: the merge row and both middle rows occupy 2 lanes.
    expect(rows[0].lanes).toBe(2);
    expect(rows[1].lanes).toBe(2);
  });

  it("each row's lane count covers its own rails and meets its neighbours'", () => {
    // Each row's gutter is sized from its own `lanes`, so a row must (a) fit
    // every column its rails touch and (b) hand its bottom-edge columns to the
    // next row unchanged — otherwise rails break where the widths differ.
    const rows = computeGraph([
      { hash: "m", parents: ["a", "b"] },
      { hash: "a", parents: ["c", "d"] },
      { hash: "b", parents: ["e"] },
      { hash: "c", parents: ["e"] },
      { hash: "d", parents: ["e"] },
      { hash: "e", parents: [] },
    ]);
    // A row is narrower than the widest row, yet still fits its own rails.
    expect(Math.min(...rows.map((r) => r.lanes))).toBeLessThan(Math.max(...rows.map((r) => r.lanes)));
    for (const r of rows) {
      for (const s of r.segments) {
        expect(Math.max(s.from, s.to)).toBeLessThan(r.lanes);
      }
    }
    const bottomCols = (r: (typeof rows)[number]) =>
      r.segments.filter((s) => s.kind !== "enter").map((s) => s.to).sort();
    const topCols = (r: (typeof rows)[number]) =>
      r.segments.filter((s) => s.kind !== "exit").map((s) => s.from).sort();
    for (let i = 0; i < rows.length - 1; i++) {
      expect(bottomCols(rows[i])).toEqual(topCols(rows[i + 1]));
    }
  });

  it("a lane right of a collapsed lane shifts left via a pass segment", () => {
    // x is a second root tip so a third lane exists while lanes 0+1 converge.
    const rows = computeGraph([
      { hash: "m", parents: ["a", "b"] },
      { hash: "x", parents: [] },
      { hash: "a", parents: ["root"] },
      { hash: "b", parents: ["root"] },
      { hash: "root", parents: [] },
    ]);
    // 'b' row: lane 0 (targeting root) passes, lane 1 is the dot continuing to root.
    // 'root' row: two lanes both target root -> first is the dot, second enters it.
    const rootRow = rows[4];
    expect(rootRow.circleIndex).toBe(0);
    expect(rootRow.segments).toContainEqual({ kind: "enter", from: 1, to: 0, color: rootRow.input[1].color });
  });
});
