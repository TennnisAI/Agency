import { describe, expect, it } from "vitest";
import { decorate, partition } from "./status";
import type { FileChange } from "../../api";

const fc = (index: string, worktree: string, path = "f"): FileChange => ({ path, index, worktree });

describe("decorate", () => {
  it("modified is M / yellow", () => {
    expect(decorate(" ", "M")).toEqual({ letter: "M", varName: "--yellow" });
  });
  it("added is A / green", () => {
    expect(decorate("A", " ")).toEqual({ letter: "A", varName: "--green" });
  });
  it("untracked is U / green", () => {
    expect(decorate("?", "?")).toEqual({ letter: "U", varName: "--green" });
  });
  it("deleted is D / red", () => {
    expect(decorate(" ", "D")).toEqual({ letter: "D", varName: "--red" });
  });
  it("conflict (UU) is ! / peach", () => {
    expect(decorate("U", "U")).toEqual({ letter: "!", varName: "--peach" });
  });
});

describe("groupOf / partition", () => {
  it("routes conflict to merge, staged to index, modified to workingTree, untracked to untracked", () => {
    const groups = partition([
      fc("U", "U", "conflict"),
      fc("M", " ", "staged"),
      fc(" ", "M", "modified"),
      fc("?", "?", "new"),
    ]);
    expect(groups.merge.map((c) => c.path)).toEqual(["conflict"]);
    expect(groups.index.map((c) => c.path)).toEqual(["staged"]);
    expect(groups.workingTree.map((c) => c.path)).toEqual(["modified"]);
    expect(groups.untracked.map((c) => c.path)).toEqual(["new"]);
  });
  it("a file staged AND modified appears in both index and workingTree", () => {
    const groups = partition([fc("M", "M", "both")]);
    expect(groups.index.map((c) => c.path)).toEqual(["both"]);
    expect(groups.workingTree.map((c) => c.path)).toEqual(["both"]);
  });
});
