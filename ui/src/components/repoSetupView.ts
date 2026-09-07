import { LargeFileScan, RepoReadiness } from "../api";
import { formatSize } from "./git/binary";

export type RepoSetupView = {
  kind: "missing" | "init" | "commit" | "dirty" | "ready";
  title: string;
  body: string;
  /** Empty for a view with nothing this dialog can do about it. */
  primaryLabel: string;
  secondaryLabel: string | null;
};

// Pure mapping from a folder's git readiness (+ where we're asking) to what the
// setup dialog should show. `ready` means nothing to do (caller treats as no-op).
export function repoSetupView(readiness: RepoReadiness, context: "add" | "spawn"): RepoSetupView {
  // The folder is gone (AGE-203). Every caller's pre-flight now catches this
  // before opening the dialog, so this is the backstop rather than the path:
  // both of the dialog's actions shell out to git inside the folder, so on a
  // folder that is not there "Initialize repository" is a button that can only
  // fail. Say what is wrong and leave Cancel as the way out; Locate folder and
  // Remove project live on the project's own screen, which is where the user
  // is being sent.
  if (readiness.state === "missing") {
    return {
      kind: "missing",
      title: "This folder is missing",
      body: "Agency can't find this folder. It may have been moved or renamed, it may have been deleted, or it may be on a disk that isn't connected. Open the project to reconnect it to its folder, or to remove it.",
      primaryLabel: "",
      secondaryLabel: null,
    };
  }
  if (readiness.state === "notARepo") {
    return {
      kind: "init",
      title: "Set up this folder for agents",
      body: "No git repository found. With one, each agent gets an isolated worktree and branch to work on. Without, agents still work here, but directly in the folder: no branches, no Source Control, nothing to merge. Initialize a repository now?",
      primaryLabel: "Initialize repository",
      // Adding a scratch folder is a legitimate choice, so it gets a way
      // through. A spawn never lands here: a folder with no repo needs no
      // setup, the agent just works in it.
      secondaryLabel: context === "add" ? "Add without git" : null,
    };
  }
  if (readiness.state === "noCommits") {
    return {
      kind: "commit",
      title: "Create an initial commit",
      body: "Agents branch from your latest commit, so this repository needs at least one before they can get their own worktrees. Create the initial commit now?",
      primaryLabel: "Create initial commit",
      // Adding gets a way through too, or "Initialize repository" above becomes
      // a trap: it leaves a commit-less repo behind, so backing out of the
      // commit would strand a folder that can never be added again. A spawn
      // keeps only Cancel, since a worktree really does need a commit to cut
      // from and there is nothing to fall back to.
      secondaryLabel: context === "add" ? "Add anyway" : null,
    };
  }
  if (readiness.state === "ready" && readiness.dirty) {
    return {
      kind: "dirty",
      title: "Uncommitted changes",
      body: "Agents work from your last commit, so they won't see your current uncommitted changes until you commit them. Commit now?",
      primaryLabel: "Commit now",
      secondaryLabel: context === "spawn" ? "Spawn anyway" : "Add anyway",
    };
  }
  return { kind: "ready", title: "", body: "", primaryLabel: "", secondaryLabel: null };
}

// One line for what the folder is carrying: how many oversized files and how
// much they weigh. `truncated` means the scan stopped at its budget, so the
// count is a floor, not a total.
export function largeFileSummary(scan: LargeFileScan): string {
  const one = scan.count === 1;
  const at = scan.truncated ? "At least " : "";
  const size = `${formatSize(scan.bytes)}${one ? "" : " in total"}`;
  const over = formatSize(scan.thresholdBytes);
  return `${at}${scan.count} file${one ? " here is" : "s here are"} over ${over} (${size})`;
}

// How many .gitignore rules the dialog spells out before it summarises the rest.
const MAX_SHOWN_RULES = 6;

// The rules the dialog shows under the checkbox. Capped, because a folder whose
// large files sit in separate directories produces one rule each: the modal has
// no scroll of its own, so an unbounded list grows it until its own buttons are
// off the bottom of the screen.
export function ignoreRulesLine(scan: LargeFileScan): string {
  const shown = scan.ignorePaths.slice(0, MAX_SHOWN_RULES).join(", ");
  const rest = scan.ignorePaths.length - MAX_SHOWN_RULES;
  return rest > 0 ? `${shown}, and ${rest} more` : shown;
}

// Said under the rules when any of them is a whole folder. The rules are picked
// from the large files alone, so a folder holding two of them is excluded whole,
// source files and all. That is usually what the user wants for a models/ or
// data/ folder and quietly wrong otherwise, so it gets one line rather than a
// choice per rule.
export function folderRulesNote(scan: LargeFileScan): string | null {
  if (!scan.ignorePaths.some((p) => p.endsWith("/"))) return null;
  return "Rules ending in / leave out the whole folder, not only the large files in it.";
}

// Said under the rules when the walk gave up early: the rules cover what it
// found, which is not necessarily everything, and committing is the one choice
// here that can't be quietly undone later.
export function ignoreRulesCaveat(scan: LargeFileScan): string | null {
  if (!scan.truncated) return null;
  return "The scan stopped at its limit, so this folder may hold large files these rules miss.";
}
