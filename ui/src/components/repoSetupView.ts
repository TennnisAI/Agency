import { RepoReadiness } from "../api";

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
