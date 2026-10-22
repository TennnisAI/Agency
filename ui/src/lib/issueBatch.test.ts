import { describe, expect, it } from "vitest";
import { Issue } from "../api";
import { batchSummary, batchWrites, pruneChecked, rangeChecked, sharedValue, toggleChecked } from "./issueBatch";

let seq = 0;
function issue(over: Partial<Issue>): Issue {
  seq += 1;
  return {
    id: `id${seq}`,
    projectId: "p",
    seq,
    title: "t",
    body: "",
    status: "todo",
    priority: 0,
    due: null,
    scheduled: null,
    rank: null,
    links: [],
    comments: [],
    createdAt: seq,
    updatedAt: seq,
    ...over,
  };
}

const rows = (...ids: string[]) => ids.map((id) => ({ id }));
const sorted = (s: Set<string>) => [...s].sort();

describe("toggleChecked", () => {
  it("adds and removes a row", () => {
    const on = toggleChecked(new Set(), "a", null);
    expect(sorted(on)).toEqual(["a"]);
    expect(sorted(toggleChecked(on, "a", null))).toEqual([]);
  });

  it("takes the open issue along on the first toggle only", () => {
    const first = toggleChecked(new Set(), "b", "a");
    expect(sorted(first)).toEqual(["a", "b"]);
    // Unchecking the open issue later does not bring it back.
    const without = toggleChecked(first, "a", "a");
    expect(sorted(toggleChecked(without, "c", "a"))).toEqual(["b", "c"]);
  });

  it("toggling the open issue itself checks just that one", () => {
    expect(sorted(toggleChecked(new Set(), "a", "a"))).toEqual(["a"]);
  });

  it("never mutates its input", () => {
    const start = new Set(["a"]);
    toggleChecked(start, "b", null);
    expect(sorted(start)).toEqual(["a"]);
  });
});

describe("rangeChecked", () => {
  const visible = rows("a", "b", "c", "d", "e");

  it("checks the span between anchor and target, either direction", () => {
    expect(sorted(rangeChecked(new Set(), visible, "b", "d"))).toEqual(["b", "c", "d"]);
    expect(sorted(rangeChecked(new Set(), visible, "d", "b"))).toEqual(["b", "c", "d"]);
  });

  it("adds to what is already checked", () => {
    expect(sorted(rangeChecked(new Set(["e"]), visible, "a", "b"))).toEqual(["a", "b", "e"]);
  });

  it("checks just the target with no anchor, or one the board no longer shows", () => {
    expect(sorted(rangeChecked(new Set(), visible, null, "c"))).toEqual(["c"]);
    expect(sorted(rangeChecked(new Set(), visible, "gone", "c"))).toEqual(["c"]);
  });

  it("ignores a target that is not on screen", () => {
    const start = new Set(["a"]);
    expect(rangeChecked(start, visible, "a", "gone")).toBe(start);
  });
});

describe("pruneChecked", () => {
  it("drops rows the board no longer shows", () => {
    expect(sorted(pruneChecked(new Set(["a", "b", "x"]), rows("a", "b", "c")))).toEqual(["a", "b"]);
  });

  it("returns the same set when nothing went", () => {
    const start = new Set(["a"]);
    expect(pruneChecked(start, rows("a", "b"))).toBe(start);
    const empty = new Set<string>();
    expect(pruneChecked(empty, [])).toBe(empty);
  });
});

describe("batchWrites", () => {
  it("writes only the issues that would change", () => {
    const todo = issue({ status: "todo" });
    const done = issue({ status: "done" });
    const writes = batchWrites([todo, done], { status: "done" });
    expect(writes).toEqual([{ issue: todo, patch: { status: "done" } }]);
  });

  it("sends each issue only the fields that differ", () => {
    const a = issue({ priority: 3, status: "todo" });
    const writes = batchWrites([a], { status: "in_progress", priority: 3 });
    expect(writes[0].patch).toEqual({ status: "in_progress" });
  });

  it("treats null as a value to clear, and skips issues already clear", () => {
    const dated = issue({ due: "2026-11-01" });
    const undated = issue({ due: null });
    const writes = batchWrites([dated, undated], { due: null });
    expect(writes).toEqual([{ issue: dated, patch: { due: null } }]);
  });

  it("ignores fields outside the batch set", () => {
    const a = issue({ title: "old" });
    // A caller that slips a title in gets nothing written for it.
    expect(batchWrites([a], { title: "new" } as never)).toEqual([]);
  });
});

describe("sharedValue", () => {
  it("names the value every issue has", () => {
    expect(sharedValue([issue({ priority: 2 }), issue({ priority: 2 })], "priority")).toBe(2);
    expect(sharedValue([issue({ due: null }), issue({ due: null })], "due")).toBeNull();
  });

  it("is undefined for a mixed or empty batch", () => {
    expect(sharedValue([issue({ priority: 2 }), issue({ priority: 3 })], "priority")).toBeUndefined();
    expect(sharedValue([], "status")).toBeUndefined();
  });
});

describe("batchSummary", () => {
  it("counts the issues", () => {
    expect(batchSummary("Updated", 1, 1)).toBe("Updated 1 issue");
    expect(batchSummary("Updated", 3, 3)).toBe("Updated 3 issues");
  });

  it("says how many of them landed when some failed", () => {
    expect(batchSummary("Deleted", 2, 3)).toBe("Deleted 2 of 3 issues");
  });
});
