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

/**
 * The actions the "nothing staged" offer can answer. The "All" variants already
 * stage everything, and an amend has no stage-everything counterpart: folding
 * the whole working tree into the previous commit is not what its button says.
 */
export type StageableAction = "commit" | "commitPush";

/** What the guard reads: the working tree, and whether a merge is unconcluded. */
export type CommitState = { changes: FileChange[]; merging: boolean };

export type CommitGuard =
  /** Nothing in the way: run the commit. */
  | { kind: "ok" }
  /** Unresolved conflicts. git refuses any commit while the index has them. */
  | { kind: "conflicts"; count: number }
  /** Nothing staged, but this many files and untracked folders could be. */
  | { kind: "unstaged"; files: number; folders: number; action: StageableAction }
  /** A clean working tree: no commit of any kind has anything to include. */
  | { kind: "empty" };

/** The stage-everything counterpart of an action, for the "nothing staged" offer. */
export function stagingAll(action: StageableAction): CommitAction {
  return action === "commitPush" ? "commitAllPush" : "commitAll";
}

/** `state` is null when the status read failed, i.e. when nothing is known. */
export function commitGuard(action: CommitAction, state: CommitState | null): CommitGuard {
  // Nothing known, so nothing to refuse. Guarding on an unread status turned a
  // failed `git status` into "this working tree has no changes", and that
  // sticky lie then displaced the real error in the panel's banner.
  if (!state) return { kind: "ok" };
  const g = partition(state.changes);
  if (g.merge.length > 0) return { kind: "conflicts", count: g.merge.length };
  // Amend rewrites the last commit, so it works on an empty index too: that is
  // how you reword a message.
  if (action === "amend") return { kind: "ok" };
  if (g.index.length > 0) return { kind: "ok" };
  // Nothing is staged, so no file is in `workingTree` and `index` at once;
  // `untracked` never overlaps either. Each file below is counted once.
  const unstaged = [...g.workingTree, ...g.untracked];
  if (unstaged.length === 0) {
    // A merge resolved to what HEAD already has (every conflict kept "current")
    // leaves the tree clean with the merge still unconcluded: `git commit` is
    // required, and succeeds. Reading that as "nothing to commit" left the
    // panel with no way to finish the merge it had just resolved.
    return state.merging ? { kind: "ok" } : { kind: "empty" };
  }
  if (action === "commitAll" || action === "commitAllPush") return { kind: "ok" };
  // An untracked folder above the expansion cap (git.rs UNTRACKED_DIR_CAP)
  // arrives as one row standing for every file inside it, so counting it as a
  // file would offer to commit 40,000 of them under the words "1 file".
  const folders = unstaged.filter((c) => c.path.endsWith("/")).length;
  return { kind: "unstaged", files: unstaged.length - folders, folders, action };
}
