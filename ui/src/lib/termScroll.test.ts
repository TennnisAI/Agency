import { describe, it, expect } from "vitest";
import { shouldSwallowWheel } from "./termScroll";

describe("shouldSwallowWheel", () => {
  it("swallows the notch on the alternate screen when arrows mean history", () => {
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
