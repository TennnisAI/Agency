import { RunInfo } from "../api";

/**
 * The two ways a run leaves the list: archived (worktree gone, branch kept, the
 * run restorable from the Archived section) or discarded (run and branch gone
 * for good).
 */
export type Removal = "archive" | "discard";

export type RemovalCopy = {
  title: string;
  body: string;
  confirmLabel: string;
  danger: boolean;
  /** Toast title when the call fails. */
  failTitle: string;
};

/** Only the run fields the wording depends on. */
export type RemovalRun = Pick<RunInfo, "kind" | "agent" | "branch" | "worktree">;

// A run can be removed from a tile, from the focus header, or from the agents
// rail, and it should read the same in all three — so the wording lives here,
// once, next to the distinctions it has to make: a terminal owns nothing but a
// shell, and a run without a worktree works in the user's own checkout, where
// nothing may be committed or thrown away on its behalf.
export function removalCopy(run: RemovalRun, action: Removal): RemovalCopy {
  if (action === "archive") {
    // Archive is offered for agents only — a terminal has no branch to keep.
    return {
      title: "Archive agent?",
      body: run.worktree
        ? `Stop "${run.agent}" and remove its worktree. Any uncommitted work is auto-committed to its "${run.branch}" branch first.`
        : `Stop "${run.agent}" and file the run away. Nothing in your checkout is committed or removed.`,
      confirmLabel: "Archive",
      danger: false,
      failTitle: "Archive failed",
    };
  }
  if (run.kind === "terminal") {
    return {
      title: "Close terminal?",
      body: "Stop the shell and remove this terminal session.",
      confirmLabel: "Close",
      danger: true,
      failTitle: "Close failed",
    };
  }
  return {
    title: "Discard agent?",
    body: run.worktree
      ? `Stop "${run.agent}", remove its worktree, and delete the run. This cannot be undone.`
      : `Stop "${run.agent}" and delete the run. Your checkout and its changes are left exactly as they are.`,
    confirmLabel: "Discard",
    danger: true,
    failTitle: "Discard failed",
  };
}

/** Which removals a run offers: a terminal can only be closed. */
export function removalsFor(run: Pick<RunInfo, "kind">): Removal[] {
  return run.kind === "terminal" ? ["discard"] : ["archive", "discard"];
}

/** Menu label for a removal, in the vocabulary of the run it acts on. */
export function removalLabel(run: Pick<RunInfo, "kind">, action: Removal): string {
  if (action === "archive") return "Archive agent";
  return run.kind === "terminal" ? "Close terminal" : "Discard agent";
}
