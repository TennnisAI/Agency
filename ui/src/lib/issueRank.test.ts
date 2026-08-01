import { describe, expect, it } from "vitest";
import { planReorder } from "./issueRank";

const g = (ranks: (number | null)[]) => ranks.map((rank, i) => ({ id: `i${i}`, rank }));

describe("planReorder", () => {
  it("no-op and out-of-range drops plan nothing", () => {
    expect(planReorder(g([1, 2, 3]), 1, 1)).toEqual([]);
    expect(planReorder(g([1, 2, 3]), -1, 0)).toEqual([]);
    expect(planReorder(g([1, 2, 3]), 0, 3)).toEqual([]);
  });

  it("materializes an unranked group in the dropped order", () => {
    // i2 dragged to the top of an unranked group: everyone gets 1..n.
    expect(planReorder(g([null, null, null]), 2, 0)).toEqual([
      { id: "i2", rank: 1 },
      { id: "i0", rank: 2 },
      { id: "i1", rank: 3 },
    ]);
    // One unranked member is enough to force materialization.
    expect(planReorder(g([1, null, 2]), 2, 0)).toEqual([
      { id: "i2", rank: 1 },
      { id: "i0", rank: 2 },
      { id: "i1", rank: 3 },
    ]);
  });

  it("uses the neighbor midpoint in a fully ranked group", () => {
    expect(planReorder(g([1, 2, 3]), 2, 1)).toEqual([{ id: "i2", rank: 1.5 }]);
  });

  it("head and tail drops extend past the end ranks", () => {
    expect(planReorder(g([1, 2, 3]), 2, 0)).toEqual([{ id: "i2", rank: 0 }]);
    expect(planReorder(g([1, 2, 3]), 0, 2)).toEqual([{ id: "i0", rank: 4 }]);
  });

  it("renormalizes when the gap collapses", () => {
    expect(planReorder(g([1, 1 + 1e-9, 3]), 2, 1)).toEqual([
      { id: "i0", rank: 1 },
      { id: "i2", rank: 2 },
      { id: "i1", rank: 3 },
    ]);
  });
});
