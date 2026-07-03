import { GhReadiness, createInstallTerminal } from "../api";
import { useRuns } from "../store/runs";
import { toastError } from "../lib/toast";

// Guided fix for a non-ready gh state. Install and sign-in run in an in-app
// terminal (interactive `gh auth login` works there, reusing the agent-install
// flow); the missing-remote gap points at Source Control. `onLeave` lets the
// host modal close itself after we navigate away to the terminal.
export default function GhSetupHint({
  readiness,
  onLeave,
}: {
  readiness: Exclude<GhReadiness, "ready">;
  onLeave: () => void;
}) {
  const { selectedProjectId, refreshRuns, setFocusedRun, setView } = useRuns();

  async function setupInTerminal(label: string, command: string) {
    if (!selectedProjectId) return;
    try {
      const run = await createInstallTerminal(selectedProjectId, label, command);
      await refreshRuns();
      setFocusedRun(run.id);
      setView("focus");
      onLeave();
    } catch (e) {
      toastError(e, "Couldn't open setup terminal");
    }
  }

  if (readiness === "notInstalled") {
    return (
      <div className="pr-setup">
        <p className="merge-note">
          This uses the GitHub CLI (<code>gh</code>), which isn't installed.
        </p>
        <button onClick={() => setupInTerminal("gh", "brew install gh")}>Install GitHub CLI…</button>
      </div>
    );
  }
  if (readiness === "notAuthenticated") {
    return (
      <div className="pr-setup">
        <p className="merge-note">
          The GitHub CLI isn't signed in yet. <code>gh auth login</code> walks you through it.
        </p>
        <button onClick={() => setupInTerminal("gh auth", "gh auth login")}>Sign in to GitHub…</button>
      </div>
    );
  }
  return (
    <p className="merge-note">
      This repo has no GitHub remote — add one in Source Control (Publish) first.
    </p>
  );
}
