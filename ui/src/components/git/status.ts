import type { FileChange } from "../../api";

export type Decoration = { letter: string; varName: string };
export type GitGroup = "merge" | "index" | "workingTree" | "untracked";

const CONFLICT = new Set(["DD", "AU", "UD", "UA", "DU", "AA", "UU"]);

export function isStaged(c: FileChange): boolean {
  return c.index !== " " && c.index !== "?";
}

function isConflict(c: FileChange): boolean {
  return CONFLICT.has(`${c.index}${c.worktree}`);
}

/** Map a status code pair to a single decoration. `index` wins when staged. */
export function decorate(index: string, worktree: string): Decoration {
  if (isConflict({ path: "", index, worktree })) return { letter: "!", varName: "--peach" };
  if (index === "?" || worktree === "?") return { letter: "U", varName: "--green" };
  const code = index !== " " ? index : worktree;
  switch (code) {
    case "A": return { letter: "A", varName: "--green" };
    case "D": return { letter: "D", varName: "--red" };
    case "R": return { letter: "R", varName: "--teal" };
    case "C": return { letter: "C", varName: "--teal" };
    case "!": return { letter: "I", varName: "--o0" };
    case "M": default: return { letter: "M", varName: "--yellow" };
  }
}

/**
 * How a change reads in the group it's rendered under: the staged group shows
 * the index code, every other group the worktree code. A path with both staged
 * and unstaged edits appears in two groups and decorates differently in each.
 */
export function decorateIn(c: FileChange, group: GitGroup): Decoration {
  return decorate(group === "index" ? c.index : " ", group === "index" ? " " : c.worktree);
}

export function groupOf(c: FileChange): GitGroup {
  if (isConflict(c)) return "merge";
  if (c.index === "?" || c.worktree === "?") return "untracked";
  return isStaged(c) ? "index" : "workingTree";
}

export function partition(changes: FileChange[]): Record<GitGroup, FileChange[]> {
  const groups: Record<GitGroup, FileChange[]> = { merge: [], index: [], workingTree: [], untracked: [] };
  for (const c of changes) {
    if (isConflict(c)) { groups.merge.push(c); continue; }
    if (c.index === "?" || c.worktree === "?") { groups.untracked.push(c); continue; }
    if (isStaged(c)) groups.index.push(c);
    if (c.worktree !== " " && c.worktree !== "?") groups.workingTree.push(c);
  }
  return groups;
}
