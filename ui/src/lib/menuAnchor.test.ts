import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { anchorMenu } from "./menuAnchor";

// Only the four numbers anchorMenu reads off a rect; the node test environment
// has no DOM to produce a real one.
function rect(left: number, top: number, width: number, height: number): DOMRect {
  return { left, top, right: left + width, bottom: top + height, width, height } as DOMRect;
}

describe("anchorMenu", () => {
  beforeEach(() => vi.stubGlobal("window", { innerWidth: 1000, innerHeight: 800 }));
  afterEach(() => vi.unstubAllGlobals());

  it("hangs a menu off the trigger's bottom-left when there is room", () => {
    expect(anchorMenu(rect(100, 200, 120, 24), 220, 200)).toEqual({ top: 228, left: 100 });
  });

  it("opens leftward from a trigger near the right edge (AGE-55)", () => {
    // The expanded issue detail's property rail: a pill at x=900 with a 220px
    // menu would run 120px off a 1000px window, so it right-anchors instead.
    expect(anchorMenu(rect(900, 200, 90, 24), 220, 200)).toEqual({ top: 228, right: 10 });
  });

  it("opens upward from a trigger too low for the menu to clear the bottom", () => {
    expect(anchorMenu(rect(100, 700, 120, 24), 220, 200)).toEqual({ bottom: 104, left: 100 });
  });

  it("flips both ways at once in the bottom-right corner", () => {
    expect(anchorMenu(rect(900, 700, 90, 24), 220, 200)).toEqual({ bottom: 104, right: 10 });
  });
});
