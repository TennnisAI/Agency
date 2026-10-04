import { describe, expect, it } from "vitest";
import { ScrollMemory, isMeasurable } from "./scrollMemory";

describe("ScrollMemory", () => {
  it("recalls what was remembered, and keeps it for the next remount", () => {
    const m = new ScrollMemory<number>(4);
    m.remember("project:p1:docs/a.md", 640);
    expect(m.recall("project:p1:docs/a.md")).toBe(640);
    // A peek, not a take: every return to the file lands on the same place.
    expect(m.recall("project:p1:docs/a.md")).toBe(640);
  });

  it("is null for a file never scrolled", () => {
    expect(new ScrollMemory<number>(4).recall("nope")).toBeNull();
  });

  it("keeps a recorded top distinct from nothing recorded", () => {
    const m = new ScrollMemory<number>(4);
    m.remember("k", 0);
    expect(m.recall("k")).toBe(0);
  });

  it("the latest position wins", () => {
    const m = new ScrollMemory<number>(4);
    m.remember("k", 100);
    m.remember("k", 900);
    expect(m.recall("k")).toBe(900);
  });

  it("evicts the least recently remembered past the cap", () => {
    const m = new ScrollMemory<number>(2);
    m.remember("a", 1);
    m.remember("b", 2);
    m.remember("a", 3); // a is newest again
    m.remember("c", 4);
    expect(m.recall("b")).toBeNull();
    expect(m.recall("a")).toBe(3);
    expect(m.recall("c")).toBe(4);
  });
});

describe("isMeasurable", () => {
  it("refuses a scroller under display: none, which reads as the top", () => {
    expect(isMeasurable({ clientHeight: 0 })).toBe(false);
    expect(isMeasurable({ clientHeight: 480 })).toBe(true);
  });
});
