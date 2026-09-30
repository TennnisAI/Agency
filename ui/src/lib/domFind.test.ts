import { describe, expect, it } from "vitest";
import { matchAtOrAfter, matchSegments } from "./domFind";

describe("matchSegments", () => {
  it("keeps a match inside one node in that node", () => {
    // "hello world" as one node.
    expect(matchSegments([11], [{ from: 6, to: 11 }])).toEqual([{ node: 0, from: 6, to: 11, match: 0 }]);
  });

  it("splits a match across the nodes it covers", () => {
    // "a **bold** phrase" renders as "a ", "bold", " phrase".
    expect(matchSegments([2, 4, 7], [{ from: 0, to: 9 }])).toEqual([
      { node: 0, from: 0, to: 2, match: 0 },
      { node: 1, from: 0, to: 4, match: 0 },
      { node: 2, from: 0, to: 3, match: 0 },
    ]);
  });

  it("draws nothing in a node a match only touches at its edge", () => {
    expect(matchSegments([3, 3], [{ from: 0, to: 3 }, { from: 3, to: 4 }])).toEqual([
      { node: 0, from: 0, to: 3, match: 0 },
      { node: 1, from: 0, to: 1, match: 1 },
    ]);
  });

  it("skips empty nodes and finds later matches in later nodes", () => {
    expect(matchSegments([2, 0, 5], [{ from: 0, to: 1 }, { from: 4, to: 6 }])).toEqual([
      { node: 0, from: 0, to: 1, match: 0 },
      { node: 2, from: 2, to: 4, match: 1 },
    ]);
  });
});

describe("matchAtOrAfter", () => {
  const ms = [{ from: 2, to: 3 }, { from: 8, to: 9 }];
  it("lands on the first match at or after the anchor, wrapping past the end", () => {
    expect(matchAtOrAfter(ms, 0)).toBe(0);
    expect(matchAtOrAfter(ms, 2)).toBe(0);
    expect(matchAtOrAfter(ms, 3)).toBe(1);
    expect(matchAtOrAfter(ms, 9)).toBe(0);
    expect(matchAtOrAfter([], 0)).toBe(-1);
  });
});
