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

/** Hover text for the bar's name: where you are, and that clicking it moves you. */
export function checkoutTooltip(c: Checkout): string {
  const where = c.kind === "worktree"
    ? `You are viewing ${c.name}'s own worktree`
    : `You are viewing the ${c.name} checkout`;
  const to = c.kind === "worktree"
    ? "the project checkout or another agent's worktree"
    : "an agent's worktree";
  return `${where}. Click to switch to ${to}.`;
}

/** Hover text for the bar's branch, which opens Source Control on the same tree. */
export function branchTooltip(c: Checkout, branch: string): string {
  return `On branch ${branch}. Opens Source Control for this ${c.kind}.`;
}

/** One working tree the Docs, Files and Source Control tabs can be pointed at. */
export interface CheckoutChoice {
  /** The run whose worktree this is, or null for the project's own checkout. */
  runId: string | null;
  name: string;
  /** The run's branch as the store has it; empty for the checkout, whose branch
   * the bar reads live. */
  branch: string;
  kind: Checkout["kind"];
}

/**
 * The places a file view can be switched to (AGE-277): the project checkout,
 * then every run with a worktree of its own, in the store's order (pins first).
 * Switching used to mean a trip to the Agents tab and its Grid button, because
 * the grid is what points these views at the checkout, and nothing on the Files
 * tab said so. A run working in the checkout (every terminal, and an agent
 * started without a worktree) is not listed: picking it would show the
 * checkout under another name.
 */
export function checkoutChoices(projectName: string, runs: RunLike[]): CheckoutChoice[] {
  const own = runs
    .filter((r) => r.worktree !== false)
    .map((r): CheckoutChoice => ({ runId: r.id, name: runListLabel(r), branch: r.branch, kind: "worktree" }));
  return [{ runId: null, name: projectName, branch: "", kind: "checkout" }, ...own];
}

/** Which choice a described checkout is, by the same token the bar reads git with. */
export function isCurrentChoice(c: Checkout, choice: CheckoutChoice): boolean {
  return c.kind === "checkout" ? choice.runId === null : choice.runId === c.taskId;
}
