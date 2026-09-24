import { useEffect, useState } from "react";
import { AgentProfile, listProfiles, spawnSyncFixAgent } from "../../api";
import { agentLabel } from "../../agents";
import { useModalKeys } from "../../hooks/useModalKeys";
import { useRuns } from "../../store/runs";
import { toastSuccess } from "../../lib/toast";
import ModalBackdrop from "../ModalBackdrop";

// What a failed Rebase & Sync offers besides the error banner: an agent that
// picks the job up where git left it (AGE-245). A conflicted pull leaves the
// branch mid-rebase, which is the state least worth leaving the user alone in.
// The backend decides where the agent works: a new tab beside the agent already
// in this tree, or a run of its own in a checkout with none.
export default function SyncFixDialog({
  taskId,
  summary,
  output,
  onShowOutput,
  onClose,
}: {
  taskId: string;
  // The banner's one-line reading of the failure, and the full text the agent
  // is handed.
  summary: string;
  output: string;
  onShowOutput?: () => void;
  onClose: () => void;
}) {
  const { runs, refreshRuns, setFocusedRun, setView, setTab, setPendingSession } = useRuns();
  const run = runs.find((r) => r.id === taskId);
  const ownAgent = run?.kind === "agent" ? run.agent : null;
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [agent, setAgent] = useState(ownAgent ?? "claude");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useModalKeys(onClose, !busy);

  useEffect(() => {
    listProfiles()
      .then((ps) => {
        setProfiles(ps);
        const names = ps.map((p) => p.name);
        setAgent((a) => (names.includes(a) || names.length === 0 ? a : names[0]));
      })
      .catch((e) => setError(String(e)));
  }, []);

  async function start() {
    setBusy(true);
    setError("");
    try {
      const { run: host, sessionId, queued } = await spawnSyncFixAgent(taskId, agent, output);
      await refreshRuns();
      if (sessionId) setPendingSession(sessionId);
      setFocusedRun(host.id);
      setTab("agents");
      setView("focus");
      // A CLI that takes no prompt on the command line gets the job through the
      // send queue, which waits for the new agent to reach its prompt.
      toastSuccess(
        queued
          ? `Started ${agentLabel(agent)}; it gets the job once it reaches its prompt`
          : `Started ${agentLabel(agent)} on the sync`,
      );
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <ModalBackdrop onBackdropClick={busy ? undefined : onClose}>
      <div
        className="modal confirm"
        role="dialog"
        aria-modal="true"
        aria-label="Rebase and sync failed"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>Rebase &amp; Sync didn't finish</h3>
          <button className="modal-x" disabled={busy} onClick={onClose}>✕</button>
        </div>
        <div className="modal-body">
          <p className="modal-note">
            Git stopped with: <code>{summary}</code>
            {onShowOutput && (
              <>
                {" "}
                <button className="git-iconbtn" disabled={busy} onClick={onShowOutput}>Output</button>
              </>
            )}
          </p>
          {error && <div className="git-error">{error}</div>}
          <p className="modal-note">
            An agent can pick it up from here: finish the rebase, resolving any conflicts, then
            push. It won't force-push, switch to a merge or throw away uncommitted changes.{" "}
            {ownAgent
              ? "It opens as a new tab in this agent's workspace."
              : "It works in the project checkout, on this branch."}
          </p>
          <div className="merge-methods">
            <label className="gh-pick-agent">
              <span>agent</span>
              <select value={agent} disabled={busy} onChange={(e) => setAgent(e.target.value)}>
                {profiles.map((p) => (
                  <option key={p.name} value={p.name}>{agentLabel(p.name)}</option>
                ))}
              </select>
            </label>
          </div>
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" disabled={busy} onClick={onClose}>Not now</button>
          <button className="btn-primary" disabled={busy || profiles.length === 0} onClick={start}>
            {busy ? "Starting…" : "Fix with an agent"}
          </button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
