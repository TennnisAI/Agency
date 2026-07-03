import { useEffect, useState } from "react";
import { RunInfo, createInstallTerminal, listProfiles } from "../api";
import { INSTALL_COMMANDS, agentLabel } from "../agents";

// Shown when the user picks an agent whose CLI isn't on PATH. For the
// preconfigured agents we know the install one-liner and offer to run it in a
// new Agency terminal (so the install is visible and inspectable); for unknown
// profiles we point at Settings instead of failing silently.
export default function InstallAgentDialog({
  agent,
  projectId,
  onInstalling,
  onCancel,
}: {
  agent: string;
  projectId: string;
  onInstalling: (run: RunInfo) => void;
  onCancel: () => void;
}) {
  const [command, setCommand] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const install = INSTALL_COMMANDS[agent];

  useEffect(() => {
    listProfiles()
      .then((ps) => setCommand(ps.find((p) => p.name === agent)?.command ?? agent))
      .catch(() => setCommand(agent));
  }, [agent]);

  async function runInstall() {
    if (!install || busy) return;
    setBusy(true);
    setError("");
    try {
      const run = await createInstallTerminal(projectId, agent, install);
      onInstalling(run);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal confirm" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{agentLabel(agent)} isn't installed</h3>
          <button className="modal-x" onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          <p className="install-lede">
            The <code>{command ?? agent}</code> command wasn't found on your PATH.
          </p>
          {install ? (
            <>
              <p>Agency can install it for you in a new terminal:</p>
              <pre className="install-cmd">{install}</pre>
            </>
          ) : (
            <p>
              Install it manually, or point this agent's profile at the right
              command in Settings.
            </p>
          )}
          {error && <div className="git-error">{error}</div>}
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" onClick={onCancel}>Cancel</button>
          {install && (
            <button className="btn-primary" disabled={busy} onClick={runInstall}>
              {busy ? "Opening terminal…" : "Install in terminal"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
