import { RunInfo } from "../api";

/**
 * Whether an archived run can be brought back, and what bringing it back means.
 *
 * The archive used to offer Restore only when the run's own branch survived,
 * which is the *abandoned* ending: archiving a merged run deletes its branch,
 * because by then the branch is a second name for commits that are already on
 * the base. So the button was disabled on every run that ended the normal way,
 * with "Nothing to restore: this run's branch is gone" — true of the branch,
 * and misleading about the run, whose work is on the base and whose
 * conversation was rescued into the archive.
 *
 * `restoreBase` is the backend's answer to "what would we cut it from instead",
 * so there are three cases and only the last one disables anything.
 */
export type RestoreRun = Pick<RunInfo, "worktree" | "branch"> & {
  archived?: Pick<NonNullable<RunInfo["archived"]>, "branchKept" | "restoreBase"> | null;
};

export function restorable(run: RestoreRun): boolean {
  // A run in the project's own checkout had no worktree removed, so there is
  // nothing to cut and nothing that can be missing: restoring only makes the
  // row live again.
  if (!run.worktree) return true;
  return !!run.archived?.branchKept || !!run.archived?.restoreBase;
}

/** The button's tooltip, which is also where the two endings are told apart. */
export function restoreTitle(run: RestoreRun): string {
  if (!run.worktree) return "Restore: bring this agent back in your own checkout";
  if (run.archived?.branchKept) {
    return `Restore: put the worktree back on ${run.branch}, where this run's work still is`;
  }
  const base = run.archived?.restoreBase;
  if (base) {
    return `Restore: cut ${run.branch} again from ${base}, where this run's work landed, and resume the conversation in it`;
  }
  return `Nothing to restore: ${run.branch} is gone, and so is the branch it was based on`;
}
