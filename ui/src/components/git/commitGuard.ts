import type { FileChange } from "../../api";
import { partition } from "./status";

/**
 * Why a commit would fail, worked out before git is asked.
 *
 * `git commit` with nothing staged exits 1 and prints "no changes added to
 * commit" on stdout, which the panel showed as `git ["commit", "-m", "Fix
 * review findings"] failed:` and nothing else (AGE-205). The reason is
 * knowable from the status the panel already polls, and so is the way out:
 * stage the changes, or stage them and commit in one go.
 */

/** Which button was pressed. The "All" variants stage everything themselves. */
export type CommitAction = "commit" | "commitAll" | "commitPush" | "commitAllPush" | "amend";

export type CommitGuard =
  /** Nothing in the way: run the commit. */
  | { kind: "ok" }
  /** Unresolved conflicts. git refuses any commit while the index has them. */
  | { kind: "conflicts"; count: number }
  /** Nothing staged, but `count` files have changes that could be. */
  | { kind: "unstaged"; count: number }
  /** A clean working tree: no commit of any kind has anything to include. */
  | { kind: "empty" };

/** The stage-everything counterpart of an action, for the "nothing staged" offer. */
export function stagingAll(action: CommitAction): CommitAction {
  return action === "commitPush" ? "commitAllPush" : "commitAll";
}

export function commitGuard(action: CommitAction, changes: FileChange[]): CommitGuard {
  const g = partition(changes);
  if (g.merge.length > 0) return { kind: "conflicts", count: g.merge.length };
  // A file with both staged and unstaged edits is in `index` and `workingTree`
  // at once; `untracked` never overlaps either, so this counts each file once.
  const unstaged = g.workingTree.length + g.untracked.length;
  // Amend rewrites the last commit, so it works on an empty index too: that is
  // how you reword a message.
  if (action === "amend") return { kind: "ok" };
  if (g.index.length > 0) return { kind: "ok" };
  if (unstaged === 0) return { kind: "empty" };
  if (action === "commitAll" || action === "commitAllPush") return { kind: "ok" };
  return { kind: "unstaged", count: unstaged };
}
