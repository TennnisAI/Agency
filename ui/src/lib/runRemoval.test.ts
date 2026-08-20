import { describe, expect, it } from "vitest";
import { CleanupPlan, RunCleanup } from "../api";
import { removalCopy, removalLabel, removalsFor, RemovalRun } from "./runRemoval";

const agent: RemovalRun = { kind: "agent", agent: "claude", branch: "agent/foo", worktree: true };
const inCheckout: RemovalRun = { ...agent, worktree: false };
const terminal: RemovalRun = { kind: "terminal", agent: "shell", branch: "main", worktree: false };

const plan = (over: Partial<CleanupPlan>): CleanupPlan => ({
  removesWorktree: true,
  deletesBranch: false,
  keepsBranch: true,
  keepsRecord: true,
  restorable: true,
  commitsAtRisk: 0,
  losesUncommitted: false,
  safeBecause: null,
  ...over,
});

/** The two plans a run offers, as the backend would return them. */
const cleanup = (
  archive: Partial<CleanupPlan>,
  del: Partial<CleanupPlan>,
  commitsAhead = 0,
): RunCleanup => ({
  facts: {
    ownsBranch: true,
    commitsAhead,
    commitsKnown: true,
    merged: false,
    pushed: false,
    gone: false,
    dirty: false,
  },
  archive: plan(archive),
  delete: plan({ keepsRecord: false, restorable: false, ...del }),
  branch: "agent/foo",
  base: "main",
});

const merged = cleanup(
  { deletesBranch: true, keepsBranch: false, restorable: false, safeBecause: "merged" },
  { deletesBranch: true, keepsBranch: false, safeBecause: "merged" },
  2,
);
const unmerged = cleanup({}, { deletesBranch: true, keepsBranch: false, commitsAtRisk: 3 }, 3);

describe("removalCopy", () => {
  it("says a merged branch goes, and why that costs nothing", () => {
    const c = removalCopy(agent, "archive", merged);
    expect(c.goes.join(" ")).toContain("agent/foo branch, which is already on main");
    expect(c.stays.join(" ")).toContain("A record");
    expect(c.stays.join(" ")).not.toContain("branch");
    expect(c.danger).toBe(false);
    expect(c.warning).toBeNull();
  });

  it("keeps an unmerged branch and counts what it is holding", () => {
    const c = removalCopy(agent, "archive", unmerged);
    expect(c.stays.join(" ")).toContain("carrying 3 commits that exist nowhere else");
    expect(c.stays.join(" ")).toContain("restorable");
    expect(c.danger).toBe(false);
  });

  it("is not a red button when deleting after a merge", () => {
    // The complaint behind AGE-149: "delete" reading as catastrophic when the
    // work is already on main.
    const c = removalCopy(agent, "delete", merged);
    expect(c.danger).toBe(false);
    expect(c.warning).toBeNull();
    expect(c.goes.join(" ")).toContain("The run and its record.");
  });

  it("warns, and turns red, only when commits would really be lost", () => {
    const c = removalCopy(agent, "delete", unmerged);
    expect(c.danger).toBe(true);
    expect(c.warning).toContain("3 commits on agent/foo");
    expect(c.warning).toContain("not on main and not on any remote");
  });

  it("claims nothing about the branch before the plan has been read", () => {
    const c = removalCopy(agent, "archive", null);
    expect([...c.goes, ...c.stays].join(" ")).not.toContain("branch,");
    expect(c.danger).toBe(false);
  });

  it("promises nothing is committed or removed when the run has no worktree", () => {
    expect(removalCopy(inCheckout, "archive", null).stays.join(" ")).toContain(
      "Nothing is committed or removed",
    );
    expect(removalCopy(inCheckout, "delete", null).stays.join(" ")).toContain(
      "exactly as they are",
    );
  });

  it("talks about the shell, not an agent, for a terminal", () => {
    const c = removalCopy(terminal, "delete", null);
    expect(c.title).toBe("Close terminal?");
    expect(c.confirmLabel).toBe("Close");
    expect(c.failTitle).toBe("Close failed");
    expect(c.danger).toBe(false);
  });

  it("says so when a delete would also take uncommitted work", () => {
    // A merged branch does not make this safe: the uncommitted changes are on
    // no branch and no remote.
    const dirty = cleanup(
      { deletesBranch: true, keepsBranch: false, safeBecause: "merged" },
      { deletesBranch: true, keepsBranch: false, losesUncommitted: true, safeBecause: "merged" },
      0,
    );
    const c = removalCopy(agent, "delete", dirty);
    expect(c.danger).toBe(true);
    expect(c.warning).toContain("Uncommitted changes in its worktree");
    // Archiving the same run commits them instead, so it stays calm.
    expect(removalCopy(agent, "archive", dirty).danger).toBe(false);
  });
});

describe("removalsFor", () => {
  it("offers archive for agents only", () => {
    expect(removalsFor(agent)).toEqual(["archive", "delete"]);
    expect(removalsFor(terminal)).toEqual(["delete"]);
  });
});

describe("removalLabel", () => {
  it("matches the run's vocabulary", () => {
    expect(removalLabel(agent, "archive")).toBe("Archive agent");
    expect(removalLabel(agent, "delete")).toBe("Delete agent");
    expect(removalLabel(terminal, "delete")).toBe("Close terminal");
  });
});
