import { describe, expect, it } from "vitest";
import { inGitlessFolder } from "./runstate";

describe("inGitlessFolder", () => {
  it("is true only for a worktree-less run with no branch", () => {
    expect(inGitlessFolder({ worktree: false, branch: "" })).toBe(true);
  });

  it("is false for a run in a real checkout, which reports the live branch", () => {
    expect(inGitlessFolder({ worktree: false, branch: "main" })).toBe(false);
  });

  it("is false for a worktree run, which always has a branch of its own", () => {
    expect(inGitlessFolder({ worktree: true, branch: "agent/foo" })).toBe(false);
  });
});
