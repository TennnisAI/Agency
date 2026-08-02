import { describe, it, expect } from "vitest";
import { shouldSwallowWheel, createPageScroller, wheelPixels, PAGE_PX } from "./termScroll";

const PGUP = "\x1b[5~";
const PGDN = "\x1b[6~";

describe("shouldSwallowWheel", () => {
  it("takes over on the alternate screen when arrows would mean history", () => {
    expect(shouldSwallowWheel("alternate", false)).toBe(true);
  });

  it("leaves the normal screen alone so the viewport still scrolls", () => {
    expect(shouldSwallowWheel("normal", false)).toBe(false);
  });

  it("keeps xterm's alternate-scroll arrows for panes that want them", () => {
    expect(shouldSwallowWheel("alternate", true)).toBe(false);
    expect(shouldSwallowWheel("normal", true)).toBe(false);
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
