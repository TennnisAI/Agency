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
});
