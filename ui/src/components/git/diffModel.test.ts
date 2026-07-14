import { describe, expect, it } from "vitest";
import { buildRows, wordSpans } from "./diffModel";
import type { FileDiff } from "../../api";

const fd = (lines: string[]): FileDiff => ({
  header: "diff --git a/f b/f\n--- a/f\n+++ b/f\n",
  hunks: [{ header: "@@ -1,2 +1,2 @@", lines }],
});

describe("buildRows", () => {
  it("pairs a delete with the following add on one row", () => {
    const rows = buildRows(fd([" ctx", "-old", "+new"]));
    // ctx row, then a paired modify row
    expect(rows[0].kind).toBe("ctx");
    const mod = rows[1];
    expect(mod.oldSpans?.map((s) => s.text).join("")).toBe("old");
    expect(mod.newSpans?.map((s) => s.text).join("")).toBe("new");
    expect(mod.oldNo).toBe(2);
    expect(mod.newNo).toBe(2);
  });
  it("emits spacer on the side without content for pure add", () => {
    const rows = buildRows(fd([" ctx", "+added"]));
    const add = rows[1];
    expect(add.kind).toBe("add");
    expect(add.oldSpans).toBeNull();
    expect(add.newSpans?.map((s) => s.text).join("")).toBe("added");
  });
  it("terminates on git's '\\ No newline at end of file' marker (no infinite loop)", () => {
    // A file with no trailing newline (common in a root commit's full-file
    // diff) ends with this marker; it must not spin buildRows forever.
    const rows = buildRows(fd(["+line one", "+line two", "\\ No newline at end of file"]));
    // The marker itself produces no row; only the two added lines do.
    expect(rows.map((r) => r.kind)).toEqual(["add", "add"]);
    expect(rows.map((r) => r.newSpans?.map((s) => s.text).join(""))).toEqual(["line one", "line two"]);
  });
});

describe("wordSpans", () => {
  it("marks only the changed word", () => {
    const { old, new: nw } = wordSpans("the cat sat", "the dog sat");
    expect(old.filter((s) => s.changed).map((s) => s.text).join("")).toContain("cat");
    expect(nw.filter((s) => s.changed).map((s) => s.text).join("")).toContain("dog");
  });
});
