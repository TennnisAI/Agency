import { describe, expect, it } from "vitest";
import { canRestore, checkpointLabel, newestFirst, restoreLines } from "./checkpoints";
import type { Checkpoint, CheckpointPreview } from "../../api";

const cp = (seq: number): Checkpoint => ({
  seq, commit: `c${seq}`, tree: `t${seq}`, at: seq, kind: "turnEnded", head: null,
});
const preview = (p: Partial<CheckpointPreview>): CheckpointPreview => ({
  write: 0, remove: 0, headMoved: false, unsaved: [], ...p,
});

describe("checkpointLabel", () => {
  it("names every kind, and something for a kind it does not know", () => {
    expect(checkpointLabel("turnEnded")).toBe("Turn ended");
    expect(checkpointLabel("beforeRestore")).toBe("Before a restore");
    expect(checkpointLabel("unknown")).toBe("Checkpoint");
  });
});

describe("newestFirst", () => {
  it("pairs each checkpoint with the one before it, by sequence", () => {
    const rows = newestFirst([cp(1), cp(10), cp(2)]);
    expect(rows.map((r) => [r.cp.seq, r.prev?.seq ?? null])).toEqual([[10, 2], [2, 1], [1, null]]);
  });
  it("is empty for no checkpoints", () => {
    expect(newestFirst([])).toEqual([]);
  });
});

describe("restoreLines", () => {
  it("counts what goes back and what goes", () => {
    const lines = restoreLines(preview({ write: 1, remove: 2 }), false);
    expect(lines[0]).toBe("1 file goes back to that version.");
    expect(lines[1]).toBe("2 files created since are deleted.");
    expect(lines).toContain("Files git ignores are left as they are.");
  });
  it("warns when the branch has moved on, and when the files are the user's checkout", () => {
    const lines = restoreLines(preview({ write: 3, headMoved: true }), true);
    expect(lines.some((l) => l.startsWith("The branch has commits made after this point"))).toBe(true);
    expect(lines.some((l) => l.includes("your project checkout"))).toBe(true);
  });
  it("says so when there is nothing to do", () => {
    expect(restoreLines(preview({}), false)).toEqual(["The files already match this checkpoint."]);
    expect(canRestore(preview({}))).toBe(false);
  });
  it("refuses while a restore would overwrite a file it could not save", () => {
    const p = preview({ write: 2, unsaved: ["data.bin"] });
    expect(restoreLines(p, false)[0]).toContain("data.bin");
    expect(canRestore(p)).toBe(false);
  });
  it("never uses an em dash", () => {
    const all = restoreLines(preview({ write: 2, remove: 1, headMoved: true }), true).join(" ");
    expect(all).not.toContain("—");
  });
});
