import { RepoReadiness } from "../api";

export type RepoSetupView = {
  kind: "init" | "commit" | "dirty" | "ready";
  title: string;
  body: string;
  showGitignore: boolean;
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
      showGitignore: false,
      primaryLabel: "Initialize repository",
      secondaryLabel: null,
    };
  }
  if (readiness.state === "noCommits") {
    return {
      kind: "commit",
      title: "Create an initial commit",
      body: "Agency needs at least one commit — each agent starts from your latest commit. Create the initial commit now?",
      showGitignore: true,
      primaryLabel: "Create initial commit",
      secondaryLabel: null,
    };
  }
  if (readiness.state === "ready" && readiness.dirty) {
    return {
      kind: "dirty",
      title: "Uncommitted changes",
      body: "Agents work from your last commit, so they won't see your current uncommitted changes until you commit them. Commit now?",
      showGitignore: true,
      primaryLabel: "Commit now",
      secondaryLabel: context === "spawn" ? "Spawn anyway" : "Add anyway",
    };
  }
  return { kind: "ready", title: "", body: "", showGitignore: false, primaryLabel: "", secondaryLabel: null };
}
