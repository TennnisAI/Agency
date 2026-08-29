import { useEffect, useState } from "react";
import { AgentProfile, PrConflicts, createPrConflictRun, listProfiles } from "../../api";
import { agentLabel } from "../../agents";
import { useModalKeys } from "../../hooks/useModalKeys";
import { useAgentModels } from "../../hooks/useAgentModels";
import { useRuns } from "../../store/runs";
import ModalBackdrop from "../ModalBackdrop";
import ModelSelect from "../ModelSelect";

// Why the merge is blocked, and the one thing that unblocks it: an agent on the
// PR's branch that merges the base in, resolves, and pushes. This is what the
// Merge button opens on a conflicted PR, so it has to answer "why did nothing
// happen?" before it offers anything (AGE-172).
export default function ConflictDialog({
  projectId,
  number,
  title,
  conflicts,
  base,
  head,
  onStarted,
  onCancel,
}: {
  projectId: string;
  number: number;
  title: string;
  // Null while the probe is still running, or if it couldn't run at all.
  conflicts: PrConflicts | null;
  base: string;
  head: string;
  onStarted: () => void;
  onCancel: () => void;
}) {
  const { refreshRuns, setFocusedRun, setView, setTab, setPendingSession } = useRuns();
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [agent, setAgent] = useState("claude");
  // Re-seeded whenever the agent changes, until the user picks for themselves.
  const { models, remembered } = useAgentModels();
  const [model, setModel] = useState<string | null>(null);
  const [modelTouched, setModelTouched] = useState(false);
  useEffect(() => {
    if (modelTouched) return;
    setModel(remembered(agent));
  }, [agent, remembered, modelTouched]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useModalKeys(onCancel, !busy);

  useEffect(() => {
    listProfiles()
      .then((ps) => {
        setProfiles(ps);
        const names = ps.map((p) => p.name);
        if (!names.includes("claude") && names.length > 0) setAgent(names[0]);
      })
      .catch((e) => setError(String(e)));
  }, []);

  async function start() {
    setBusy(true);
    setError("");
    try {
      const { run, sessionId } = await createPrConflictRun(projectId, number, agent, model);
      await refreshRuns();
      // When the branch was already checked out by another run, the work goes
      // into a new tab there; open that tab, not the agent that wrote the code.
      if (sessionId) setPendingSession(sessionId);
      setFocusedRun(run.id);
      setTab("agents");
      setView("focus");
      onStarted();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  const files = conflicts?.files ?? [];

  return (
    <ModalBackdrop onBackdropClick={busy ? undefined : onCancel}>
      <div
        className="modal confirm"
        role="dialog"
        aria-modal="true"
        aria-label="Resolve merge conflicts"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>This PR has merge conflicts</h3>
          <button className="modal-x" disabled={busy} onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          <p className="modal-note">
            GitHub won't merge #{number} “{title}” while <code>{head}</code> conflicts with{" "}
            <code>{base}</code>. The conflicts have to be resolved on the branch first.
          </p>
          {files.length > 0 && (
            <ul className="pr-conflict-files">
              {files.map((f) => (
                <li key={f}><code>{f}</code></li>
              ))}
            </ul>
          )}
          {conflicts === null && (
            <p className="modal-note"><span className="spinner" /> Checking which files collide…</p>
          )}
          {conflicts !== null && !conflicts.probed && (
            <p className="modal-note">
              Agency couldn't work out which files collide from here. The agent will find them.
            </p>
          )}
          {error && <div className="git-error">{error}</div>}
          <p className="modal-note">
            An agent can merge <code>{base}</code> into <code>{head}</code>, resolve the conflicts
            and push, which updates this PR. It won't rebase, force-push or merge the PR itself.
          </p>
          <div className="merge-methods">
            <label className="gh-pick-agent">
              <span>agent</span>
              <select
                value={agent}
                disabled={busy}
                onChange={(e) => { setAgent(e.target.value); setModelTouched(false); }}
              >
                {profiles.map((p) => (
                  <option key={p.name} value={p.name}>{agentLabel(p.name)}</option>
                ))}
              </select>
              {models[agent]?.supported && (
                <>
                  <span>model</span>
                  <ModelSelect
                    info={models[agent]}
                    projectId={projectId}
                    value={model}
                    onChange={(m) => { setModel(m); setModelTouched(true); }}
                  />
                </>
              )}
            </label>
          </div>
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" disabled={busy} onClick={onCancel}>Cancel</button>
          <button className="btn-primary" disabled={busy || profiles.length === 0} onClick={start}>
            {busy ? "Starting…" : "Fix with an agent"}
          </button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
