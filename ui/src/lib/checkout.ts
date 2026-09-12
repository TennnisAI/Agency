import { FileRoot, RunInfo, projectTarget } from "../api";
import { runListLabel } from "../agents";

/** The fields a run has to expose for the bar to name it. */
export type RunLike = Pick<
  RunInfo, "id" | "kind" | "title" | "prompt" | "branch" | "agent" | "loopConfig" | "raceId" | "model" | "worktree"
>;

export interface Checkout {
  /** Git-panel token for this working tree, for reading its branch. */
  taskId: string;
  /** Whose working tree it is: the agent's label, or the project's name. */
  name: string;
  /** An agent's private worktree, or the project's own checkout. */
  kind: "worktree" | "checkout";
}

/**
 * Which working tree a file view is actually reading. An agent names a place of
 * its own only when it has a worktree: one started without one (and every
 * terminal) works directly in the project checkout, so it reads as that
 * checkout rather than as a second copy of it.
 */
export function describeCheckout({ root, projectId, projectName, runs }: {
  root: FileRoot;
  projectId: string;
  projectName: string;
  runs: RunLike[];
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
    };
  }
  return { taskId: projectTarget(projectId), name: projectName, kind: "checkout" };
}

/** Hover text for the bar: where you are, on which branch, and where clicking it goes. */
export function checkoutTooltip(c: Checkout, branch: string | null): string {
  const where = c.kind === "worktree"
    ? `You are viewing ${c.name}'s own worktree`
    : `You are viewing the ${c.name} checkout`;
  const on = branch ? ` on branch ${branch}` : "";
  return `${where}${on}. Opens Source Control for this ${c.kind}.`;
}
