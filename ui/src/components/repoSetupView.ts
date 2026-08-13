import { LargeFileScan, RepoReadiness } from "../api";
import { formatSize } from "./git/binary";

export type RepoSetupView = {
  kind: "init" | "commit" | "dirty" | "ready";
  title: string;
  body: string;
  primaryLabel: string;
  secondaryLabel: string | null;
};

// Pure mapping from a folder's git readiness (+ where we're asking) to what the
// setup dialog should show. `ready` means nothing to do (caller treats as no-op).
export function repoSetupView(readiness: RepoReadiness, context: "add" | "spawn"): RepoSetupView {
  if (readiness.state === "notARepo") {
    return {
      kind: "init",
      title: "Set up this folder for agents",
      body: "No git repository found. Agency runs each agent in an isolated git worktree, so this folder needs to be a repository. Initialize one now?",
      primaryLabel: "Initialize repository",
      secondaryLabel: null,
    };
  }
  if (readiness.state === "noCommits") {
    return {
      kind: "commit",
      title: "Create an initial commit",
      body: "Agency needs at least one commit; each agent starts from your latest commit. Create the initial commit now?",
      primaryLabel: "Create initial commit",
      secondaryLabel: null,
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

// Said under the rules when the walk gave up early: the rules cover what it
// found, which is not necessarily everything, and committing is the one choice
// here that can't be quietly undone later.
export function ignoreRulesCaveat(scan: LargeFileScan): string | null {
  if (!scan.truncated) return null;
  return "The scan stopped at its limit, so this folder may hold large files these rules miss.";
}
