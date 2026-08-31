import { describe, expect, it } from "vitest";
import { CleanupPlan, RunCleanup } from "../api";
import {
  mergeTidyCopy,
  removalCopy,
  removalLabel,
  removalsFor,
  RemovalRun,
} from "./runRemoval";

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
    baseExists: true,
  },
  archive: plan(archive),
  delete: plan({ keepsRecord: false, restorable: false, ...del }),
  branch: "agent/foo",
  base: "main",
  // The fixture is a claude worktree run: one of the two agents whose
  // transcript layout Agency reads, so it is the case where the conversation
  // travels with the teardown. `unmanaged` below is everyone else.
  managesTranscript: true,
});

/** The same run under an agent whose transcript Agency cannot find. */
const unmanaged = (c: RunCleanup): RunCleanup => ({ ...c, managesTranscript: false });

// The ordinary ending: the branch goes because its commits are on main, and
// the archive is still restorable — main is what a restore cuts it from again.
const merged = cleanup(
  { deletesBranch: true, keepsBranch: false, safeBecause: "merged" },
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

  it("only claims the conversation for an agent Agency can find one for", () => {
    // Deleting a claude or pi worktree run removes that agent's session
    // directory for the worktree; for every other agent Agency has no idea
    // where the conversation is and touches nothing, so the delete list has to
    // say the history stays rather than imply it goes.
    const del = removalCopy(agent, "delete", merged);
    expect(del.goes.join(" ")).toContain("Its conversation, from the agent's own session store.");
    expect(del.stays.join(" ")).not.toContain("session history");

    const other = removalCopy(agent, "delete", unmanaged(merged));
    expect(other.goes.join(" ")).not.toContain("conversation");
    expect(other.stays.join(" ")).toContain("This agent's own session history");

    // And the archive list mirrors it: it only offers a conversation to read
    // when there was one to rescue.
    expect(removalCopy(agent, "archive", merged).stays.join(" ")).toContain("and its conversation");
    expect(removalCopy(agent, "archive", unmanaged(merged)).stays.join(" ")).not.toContain(
      "conversation",
    );
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

describe("mergeTidyCopy", () => {
  /** One line per button, as they read on screen. */
  const lines = (c: ReturnType<typeof mergeTidyCopy>) =>
    c.choices.map((o) => `${o.verb} ${o.text}.`);

  it("names every button under it, in the buttons' own words", () => {
    // AGE-164: the step explained archiving under a heading, "Tidy up", that
    // matched no button, and said nothing at all about the other two.
    const c = mergeTidyCopy(merged);
    expect(c.choices.map((o) => o.verb)).toEqual(["Archive", "Delete", "Keep"]);
    expect(lines(c)).toEqual([
      "Archive removes the worktree and the agent/foo branch, and keeps the transcript and a record under Archived.",
      "Delete removes all of it, the transcript included.",
      "Keep leaves the agent as it is.",
    ]);
  });

  it("says the transcript is what the two verbs actually differ over", () => {
    // The one asymmetry left now that both endings restore, and the one a
    // vaguer word ("its record") let the user walk past.
    const c = mergeTidyCopy(merged);
    expect(lines(c)[0]).toContain("keeps the transcript and a record under Archived");
    expect(lines(c)[1]).toContain("the transcript included");
  });

  it("says why the branch is safe to let go, and that archiving is reversible", () => {
    const c = mergeTidyCopy(merged);
    expect(c.detail).toBe(
      "The agent/foo branch goes either way, since it is already on main. Neither touches " +
        "what you just merged. Archiving can be undone: Restore brings the worktree and the " +
        "conversation back.",
    );
    expect(c.caveat).toBeNull();
  });

  it("does not claim a transcript it cannot see, for the agents it cannot read", () => {
    // Most agents: Agency has no idea where their conversation lives, so it
    // neither rescues nor removes one, and the user who deletes an agent and
    // then resumes it from the agent itself is not being contradicted.
    const c = mergeTidyCopy(unmanaged(merged));
    expect(lines(c)[0]).toBe(
      "Archive removes the worktree and the agent/foo branch, and keeps a record of what it did under Archived.",
    );
    expect(lines(c)[1]).toBe("Delete removes all of it.");
    expect(c.detail).toContain("This agent keeps its own session history, which neither one touches.");
  });

  it("does not claim both verbs take a branch only one of them takes", () => {
    // A dirty worktree keeps the branch through an archive (the leftovers are
    // committed to it) and loses it to a delete, so neither the Archive line
    // nor the shared detail may swallow the branch.
    const dirty = cleanup(
      {},
      { deletesBranch: true, keepsBranch: false, losesUncommitted: true },
      0,
    );
    const c = mergeTidyCopy(dirty);
    expect(lines(c)[0]).toBe("Archive removes the worktree, and keeps the transcript and a record under Archived.");
    expect(c.detail).toContain("Only deleting takes the agent/foo branch with it.");
    expect(c.caveat).toContain("Archiving commits what is uncommitted in the worktree");
    expect(c.caveat).toContain("deleting discards it");
  });

  it("carries the commits-at-risk warning into the caveat", () => {
    const c = mergeTidyCopy(unmerged);
    expect(c.caveat).toContain("3 commits on agent/foo");
    expect(c.caveat).toContain("not on main and not on any remote");
  });

  it("does not promise a restore the archive cannot deliver", () => {
    // Branch gone and no base to cut it again from: the plan says so, and the
    // line about undoing has to go with it.
    const stranded = cleanup({ deletesBranch: true, keepsBranch: false, restorable: false }, {});
    expect(mergeTidyCopy(stranded).detail).not.toContain("can be undone");
  });

  it("claims nothing about the branch or the transcript before the plan is read", () => {
    const c = mergeTidyCopy(null);
    expect(c.detail).toBeNull();
    expect(c.caveat).toBeNull();
    // The choices are true of any run, so they are still said — minus the
    // branch and the transcript, the two parts that need the plan. The
    // transcript defaults to unclaimed rather than promised, so the window
    // never says "the transcript included" about an agent it turns out not to
    // hold one for.
    expect(c.choices).toHaveLength(3);
    expect(lines(c)[0]).toBe(
      "Archive removes the worktree, and keeps a record of what it did under Archived.",
    );
    expect(lines(c)[1]).toBe("Delete removes all of it.");
  });
});
