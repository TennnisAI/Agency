import { openUrl } from "@tauri-apps/plugin-opener";
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
    // We can't cheaply probe whether Homebrew is on PATH (the agent_installed
    // IPC only checks registered agent profiles, not arbitrary binaries like
    // `brew`), so offer both paths: `brew install gh` for Homebrew users and a
    // manual download for everyone else — never brew-only.
    return (
      <div className="pr-setup">
        <p className="merge-note">
          This uses the GitHub CLI (<code>gh</code>), which isn't installed.
        </p>
        <button onClick={() => setupInTerminal("gh", "brew install gh")}>Install with Homebrew…</button>
        <button className="ghost" onClick={() => openUrl("https://cli.github.com").catch(() => {})}>
          No Homebrew? Download from cli.github.com ↗
        </button>
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
