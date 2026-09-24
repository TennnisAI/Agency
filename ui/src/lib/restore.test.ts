import { describe, expect, it } from "vitest";
import { RestoreRun, restorable, restoreTitle } from "./restore";

const run = (over: Partial<RestoreRun> = {}): RestoreRun => ({
  worktree: true,
  branch: "agent/foo",
  archived: { branchKept: false, restoreBase: "main", cutBranch: true },
  ...over,
});

describe("restorable", () => {
  it("restores a merged run from the base its work landed on", () => {
    // The archive's own ending: the branch went with it because the commits
    // were on main, and Restore was disabled on exactly those runs — the
    // common case — with "Nothing to restore: this run's branch is gone".
    expect(restorable(run())).toBe(true);
  });

  it("restores an abandoned run onto the branch it kept", () => {
    expect(restorable(run({ archived: { branchKept: true, restoreBase: null, cutBranch: true } }))).toBe(true);
  });

  it("refuses only when the base is gone as well", () => {
    expect(restorable(run({ archived: { branchKept: false, restoreBase: null, cutBranch: true } }))).toBe(false);
  });

  it("restores a run that never had a worktree", () => {
    // Nothing was removed, so nothing can be missing: restoring only puts the
    // row back on the board.
    expect(restorable(run({ worktree: false, archived: null }))).toBe(true);
  });
});

describe("restoreTitle", () => {
  it("names the branch it will cut again, and where from", () => {
    expect(restoreTitle(run())).toContain("cut agent/foo again from main");
    expect(restoreTitle(run())).toContain("resume the conversation");
  });

  it("says the worktree goes back on the kept branch", () => {
    const t = restoreTitle(run({ archived: { branchKept: true, restoreBase: null, cutBranch: true } }));
    expect(t).toContain("put the worktree back on agent/foo");
  });

  it("says what is missing when there is nothing to restore from", () => {
    const t = restoreTitle(run({ archived: { branchKept: false, restoreBase: null, cutBranch: true } }));
    expect(t).toContain("Nothing to restore");
    expect(t).toContain("the branch it was based on");
  });

  it("says a gone branch Agency did not create is not cut again, rather than blaming the base", () => {
    // A PR head deleted while its review run was archived: the base is there,
    // but cutting the PR's name from it brings back none of the PR.
    const pr = run({ branch: "feature/login", archived: { branchKept: false, restoreBase: null, cutBranch: false } });
    expect(restorable(pr)).toBe(false);
    const t = restoreTitle(pr);
    expect(t).toContain("no record of creating it");
    expect(t).not.toContain("the branch it was based on");
  });
});
