import { describe, expect, it } from "vitest";
import { visibleRange } from "./useVirtualRows";

// rowH=20, viewport=100 (5 rows tall), overscan chosen per-case.
describe("visibleRange", () => {
  it("returns empty for an empty list", () => {
    expect(visibleRange(0, 100, 20, 0, 8)).toEqual({ start: 0, end: 0 });
  });

  it("returns empty when row height is unmeasured", () => {
    expect(visibleRange(0, 100, 0, 50, 8)).toEqual({ start: 0, end: 0 });
  });

  it("windows from the top with overscan clamped at 0", () => {
    // Region flush with viewport top, not yet scrolled.
    expect(visibleRange(0, 100, 20, 1000, 2)).toEqual({ start: 0, end: 7 });
  });

  it("slides the window as the region scrolls above the viewport", () => {
    // Scrolled 200px past the top → first 10 rows are above the fold.
    expect(visibleRange(-200, 100, 20, 1000, 2)).toEqual({ start: 8, end: 17 });
  });

  it("clamps the end to the row count near the bottom", () => {
    // 50 rows total, scrolled so only the last few remain visible.
    expect(visibleRange(-960, 100, 20, 50, 2)).toEqual({ start: 46, end: 50 });
  });

  it("collapses to empty when the region sits entirely below the viewport", () => {
    const r = visibleRange(300, 100, 20, 1000, 2);
    expect(r.end).toBe(r.start);
  });

  it("collapses to empty when the region sits entirely above the viewport", () => {
    // 50 rows * 20px = 1000 tall; scrolled 1200px past → all above.
    const r = visibleRange(-1200, 100, 20, 50, 2);
    expect(r.end).toBe(r.start);
    expect(r.start).toBe(50);
  });
});
