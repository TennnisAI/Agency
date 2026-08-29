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

  it("opens upward from a trigger with room for neither, then hugs the top", () => {
    // A menu taller than the window fits above or below nothing; `.agent-menu`
    // caps its height so this stays hypothetical, but the placement still has
    // to land the menu on screen rather than off the top.
    expect(anchorMenu(rect(100, 400, 120, 24), 220, 500)).toEqual({ bottom: 404, left: 100 });
  });

  it("opens leftward from a trigger that asks for it, wherever it sits", () => {
    // The "+ Agent" button sits at the right of its own header, and a pane to
    // its right leaves room the menu should not take.
    expect(anchorMenu(rect(500, 100, 90, 24), 220, 200, "left")).toEqual({ top: 128, right: 410 });
  });

  it("still opens rightward when a left-opening menu would run off the left edge", () => {
    expect(anchorMenu(rect(40, 100, 90, 24), 220, 200, "left")).toEqual({ top: 128, left: 40 });
  });
});
