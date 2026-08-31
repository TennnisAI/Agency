import { describe, expect, it } from "vitest";
import { RestoreRun, restorable, restoreTitle } from "./restore";

const run = (over: Partial<RestoreRun> = {}): RestoreRun => ({
  worktree: true,
  branch: "agent/foo",
  archived: { branchKept: false, restoreBase: "main" },
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
    expect(restorable(run({ archived: { branchKept: true, restoreBase: null } }))).toBe(true);
  });

  it("refuses only when the base is gone as well", () => {
    expect(restorable(run({ archived: { branchKept: false, restoreBase: null } }))).toBe(false);
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
    const t = restoreTitle(run({ archived: { branchKept: true, restoreBase: null } }));
    expect(t).toContain("put the worktree back on agent/foo");
  });

  it("says what is missing when there is nothing to restore from", () => {
    const t = restoreTitle(run({ archived: { branchKept: false, restoreBase: null } }));
    expect(t).toContain("Nothing to restore");
    expect(t).toContain("the branch it was based on");
  });
});
