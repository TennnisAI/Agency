import { describe, it, expect } from "vitest";
import { computeAnchor } from "./anchor";
import type { DiffRow } from "../git/diffModel";

// Minimal DiffRow factory — only the fields computeAnchor reads matter.
function row(kind: DiffRow["kind"], oldNo: number | null, newNo: number | null): DiffRow {
  return { kind, hunkIndex: 0, lineIndex: 0, oldNo, newNo, oldSpans: null, newSpans: null };
}

describe("computeAnchor", () => {
  it("anchors a single added line to RIGHT/newNo", () => {
    expect(computeAnchor([row("add", null, 42)])).toEqual({ line: 42, side: "RIGHT" });
  });

  it("anchors a single context line to RIGHT/newNo", () => {
    expect(computeAnchor([row("ctx", 40, 42)])).toEqual({ line: 42, side: "RIGHT" });
  });

  it("anchors a pure deletion to LEFT/oldNo", () => {
    expect(computeAnchor([row("del", 40, null)])).toEqual({ line: 40, side: "LEFT" });
  });

  it("anchors a modified (paired) line to RIGHT", () => {
    // buildRows represents a modify as a single row carrying both sides.
    expect(computeAnchor([row("del", 40, 42)])).toEqual({ line: 42, side: "RIGHT" });
  });

  it("builds a same-side RIGHT range from top to bottom", () => {
    const rows = [row("add", null, 10), row("add", null, 11), row("add", null, 12)];
    expect(computeAnchor(rows)).toEqual({ line: 12, side: "RIGHT", startLine: 10, startSide: "RIGHT" });
  });

  it("builds a LEFT range for a multi-line deletion", () => {
    const rows = [row("del", 5, null), row("del", 6, null)];
    expect(computeAnchor(rows)).toEqual({ line: 6, side: "LEFT", startLine: 5, startSide: "LEFT" });
  });

  it("clamps a mixed deletion+addition span to the RIGHT side", () => {
    const rows = [row("del", 5, null), row("add", null, 5), row("add", null, 6)];
    expect(computeAnchor(rows)).toEqual({ line: 6, side: "RIGHT", startLine: 5, startSide: "RIGHT" });
  });

  it("returns null when nothing anchorable is selected", () => {
    expect(computeAnchor([row("spacer", null, null)])).toBeNull();
    expect(computeAnchor([])).toBeNull();
  });
});
