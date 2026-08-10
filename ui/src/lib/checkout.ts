import { FileRoot, RunInfo, projectTarget } from "../api";
import { runListLabel } from "../agents";

/** The fields a run has to expose for the bar to name it. */
export type RunLike = Pick<
  RunInfo, "id" | "kind" | "title" | "prompt" | "branch" | "agent" | "loopConfig" | "raceId" | "worktree"
>;

export interface Checkout {
  /** Git-panel token for this working tree, for reading its branch. */
  taskId: string;
  /** Whose working tree it is: the agent's label, or the project's name. */
  name: string;
  /** An agent's private worktree, or the project's own checkout. */
  kind: "worktree" | "checkout";
  /** Caption for a view that deliberately isn't following the selected agent. */
  note: string | null;
  /** The same, spelled out for the tooltip. */
  noteDetail: string | null;
}

/**
 * Which working tree a file view is actually reading. An agent names a place of
 * its own only when it has a worktree: one started without one (and every
 * terminal) works directly in the project checkout, so it reads as that
 * checkout rather than as a second copy of it.
 */
export function describeCheckout({ root, projectId, projectName, runs, selectedRunId }: {
  root: FileRoot;
  projectId: string;
  projectName: string;
  runs: RunLike[];
  selectedRunId: string | null;
}): Checkout {
  const run = root.kind === "run" ? runs.find((r) => r.id === root.id) ?? null : null;
  // A run the store doesn't know (archived out from under the view) is still a
  // worktree as far as the file root is concerned — only a known `worktree:
  // false` puts the view in the project checkout.
  if (root.kind === "run" && run?.worktree !== false) {
    return {
      taskId: root.id,
      name: run ? runListLabel(run) : "agent worktree",
      kind: "worktree",
      note: null,
      noteDetail: null,
    };
  }
  // The project's own checkout. When an agent with a worktree of its own is
  // selected, say so: this view isn't following that selection (Docs never
  // does, since notes are a project-level artifact).
  const stray = runs.find((r) => r.id === selectedRunId && r.worktree) ?? null;
  return {
    taskId: projectTarget(projectId),
    name: projectName,
    kind: "checkout",
    note: stray ? "not the selected agent's worktree" : null,
    noteDetail: stray
      ? `${runListLabel(stray)} has a worktree of its own; browse it from the Files tab.`
      : null,
  };
}

/** Hover text for the bar: where you are, on which branch, and why. */
export function checkoutTooltip(c: Checkout, branch: string | null): string {
  const where = c.kind === "worktree"
    ? `You are viewing ${c.name}'s own worktree`
    : `You are viewing the ${c.name} checkout`;
  const on = branch ? ` on branch ${branch}` : "";
  return `${where}${on}.${c.noteDetail ? ` ${c.noteDetail}` : ""}`;
}
