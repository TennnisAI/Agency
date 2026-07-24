import { useEffect, useState } from "react";
import { RepoReadiness, CloneProgress, initRepo, commitRepo, inspectRepo } from "../api";
import { repoSetupView } from "./repoSetupView";
import { useModalKeys } from "../hooks/useModalKeys";

type Props = {
  readiness: RepoReadiness;
  context: "add" | "spawn";
  repoPath: string;
  onResolved: () => void;
  onCancel: () => void;
};

export default function RepoSetupDialog({ readiness, context, repoPath, onResolved, onCancel }: Props) {
  const [current, setCurrent] = useState<RepoReadiness>(readiness);
  const [busy, setBusy] = useState(false);
  // Latest progress update while an op runs (null before the first arrives).
  const [progress, setProgress] = useState<CloneProgress | null>(null);
  const [error, setError] = useState("");

  const view = repoSetupView(current, context);

  // Escape matches the Cancel button, which is disabled while an op runs.
  useModalKeys(onCancel, !busy);

  // Defensive: if we're already ready+clean (caller normally avoids opening then), resolve.
  useEffect(() => {
    if (view.kind === "ready") onResolved();
  }, [view.kind]);
  if (view.kind === "ready") return null;

  async function doInit() {
    setBusy(true); setError(""); setProgress(null);
    try {
      await initRepo(repoPath);
      // After init the folder always has no commits — advance to the commit step.
      setCurrent(await inspectRepo(repoPath));
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); setProgress(null); }
  }

  async function doCommit() {
    setBusy(true); setError(""); setProgress(null);
    try {
      // Only write a default .gitignore when creating the very first commit of a
      // brand-new repo — there it stops node_modules/target getting committed and
      // never overwrites an existing file. For an already-established repo with
      // uncommitted changes we don't touch it.
      await commitRepo(repoPath, view.kind === "commit", setProgress);
      onResolved();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); setProgress(null); }
  }

  const onPrimary = view.kind === "init" ? doInit : doCommit;
  // Shown until the backend's first progress message lands. `init` streams
  // nothing — it's a fast `git init` plus a rescan of the folder.
  const startingLabel = view.kind === "init" ? "Setting up repository…" : "Staging files…";

  return (
    <div className="modal-backdrop" onClick={(e) => { e.stopPropagation(); if (!busy) onCancel(); }}>
      <div className="modal confirm" role="dialog" aria-modal="true" aria-label={view.title} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{view.title}</h3>
          <button className="modal-x" disabled={busy} onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          {view.body}
          {busy && (
            // Always indeterminate: git reports no percentage for add/commit, so
            // the detail line carries the staged-file count instead.
            <div className="clone-progress" role="status" aria-live="polite">
              <div className="clone-progress-head">
                <span className="clone-progress-phase">{progress ? progress.phase : startingLabel}</span>
              </div>
              <div className="clone-progress-track">
                <div className="clone-progress-bar indeterminate" />
              </div>
              {progress?.detail && <div className="clone-progress-detail">{progress.detail}</div>}
            </div>
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
