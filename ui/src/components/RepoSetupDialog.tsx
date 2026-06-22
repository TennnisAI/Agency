import { useEffect, useState } from "react";
import { RepoReadiness, initRepo, commitRepo, inspectRepo } from "../api";
import { repoSetupView } from "./repoSetupView";

type Props = {
  readiness: RepoReadiness;
  context: "add" | "spawn";
  repoPath: string;
  onResolved: () => void;
  onCancel: () => void;
};

export default function RepoSetupDialog({ readiness, context, repoPath, onResolved, onCancel }: Props) {
  const [current, setCurrent] = useState<RepoReadiness>(readiness);
  const [addGitignore, setAddGitignore] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const view = repoSetupView(current, context);

  // Defensive: if we're already ready+clean (caller normally avoids opening then), resolve.
  useEffect(() => {
    if (view.kind === "ready") onResolved();
  }, [view.kind]);
  if (view.kind === "ready") return null;

  async function doInit() {
    setBusy(true); setError("");
    try {
      await initRepo(repoPath);
      // After init the folder always has no commits — advance to the commit step.
      setCurrent(await inspectRepo(repoPath));
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  async function doCommit() {
    setBusy(true); setError("");
    try {
      await commitRepo(repoPath, addGitignore);
      onResolved();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  const onPrimary = view.kind === "init" ? doInit : doCommit;

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal confirm" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{view.title}</h3>
          <button className="modal-x" onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          {view.body}
          {view.showGitignore && (
            <label className="setup-gitignore">
              <input type="checkbox" checked={addGitignore} onChange={(e) => setAddGitignore(e.target.checked)} />
              Add a .gitignore (node_modules, .env, dist, target, .DS_Store)
            </label>
          )}
          {error && <div className="git-error">{error}</div>}
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" disabled={busy} onClick={onCancel}>Cancel</button>
          {view.secondaryLabel && (
            <button className="btn-secondary" disabled={busy} onClick={onResolved}>{view.secondaryLabel}</button>
          )}
          <button className="btn-primary" disabled={busy} onClick={onPrimary}>{view.primaryLabel}</button>
        </div>
      </div>
    </div>
  );
}
