import { describe, it, expect } from "vitest";
import { shouldSwallowWheel, appTracksWheel, createPageScroller, wheelPixels, PAGE_PX } from "./termScroll";

const PGUP = "\x1b[5~";
const PGDN = "\x1b[6~";

describe("appTracksWheel", () => {
  it("is true for the protocols that carry wheel events", () => {
    expect(appTracksWheel("vt200")).toBe(true);
    expect(appTracksWheel("drag")).toBe(true);
    expect(appTracksWheel("any")).toBe(true);
  });

  it("is false for X10, which reports button presses only", () => {
    expect(appTracksWheel("x10")).toBe(false);
  });

  it("is false when nothing is tracking the mouse", () => {
    expect(appTracksWheel("none")).toBe(false);
  });
});

describe("shouldSwallowWheel", () => {
  it("takes over on the alternate screen when arrows would mean history", () => {
    expect(shouldSwallowWheel("alternate", "none", false)).toBe(true);
  });

  it("leaves the wheel to an agent that is tracking it, which scrolls itself", () => {
    // A live agent pane: alternate screen, all-motion tracking, SGR encoding.
    // Paging it by hand is what made scrolling sluggish (AGE-26).
    expect(shouldSwallowWheel("alternate", "any", false)).toBe(false);
    expect(shouldSwallowWheel("alternate", "drag", false)).toBe(false);
    expect(shouldSwallowWheel("alternate", "vt200", false)).toBe(false);
  });

  it("still guards X10 tracking, which never gets the wheel", () => {
    expect(shouldSwallowWheel("alternate", "x10", false)).toBe(true);
  });

  it("leaves the normal screen alone so the viewport still scrolls", () => {
    expect(shouldSwallowWheel("normal", "none", false)).toBe(false);
  });

  it("keeps xterm's alternate-scroll arrows for panes that want them", () => {
    expect(shouldSwallowWheel("alternate", "none", true)).toBe(false);
    expect(shouldSwallowWheel("normal", "none", true)).toBe(false);
  });
});

describe("wheelPixels", () => {
  it("passes pixel deltas through", () => {
    expect(wheelPixels(120, 0)).toBe(120);
  });

  it("scales line and page deltas", () => {
    expect(wheelPixels(3, 1)).toBe(48);
    expect(wheelPixels(-1, 2)).toBe(-400);
  });
});

describe("createPageScroller", () => {
  it("emits nothing until a page's worth has accumulated", () => {
    const scroll = createPageScroller();
    expect(scroll(-100)).toBe("");
    expect(scroll(-100)).toBe("");
    expect(scroll(-100)).toBe(PGUP);
  });

  it("scrolls down on a positive delta", () => {
    const scroll = createPageScroller();
    expect(scroll(PAGE_PX)).toBe(PGDN);
  });

  it("emits several pages for one big delta", () => {
    const scroll = createPageScroller();
    expect(scroll(-PAGE_PX * 2)).toBe(PGUP + PGUP);
  });

  it("keeps the remainder so slow trackpad scrolling still pages", () => {
    const scroll = createPageScroller();
    scroll(-PAGE_PX * 1.5); // one page out, half a page left over
    expect(scroll(-PAGE_PX / 2)).toBe(PGUP);
  });

  it("drops leftovers when the direction reverses, so a flick back needs a full page", () => {
    const scroll = createPageScroller();
    scroll(-PAGE_PX * 0.9); // nearly a page up, unpaid
    expect(scroll(PAGE_PX * 0.5)).toBe("");
    // 0.5 + 0.4 of a page: only a full page down pays out, the up-leftover is gone.
    expect(scroll(PAGE_PX * 0.4)).toBe("");
    expect(scroll(PAGE_PX * 0.1)).toBe(PGDN);
  });

  it("ignores zero deltas from horizontal scrolling", () => {
    const scroll = createPageScroller();
    expect(scroll(0)).toBe("");
  });
});
