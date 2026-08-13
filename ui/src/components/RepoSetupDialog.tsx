import { useEffect, useRef, useState } from "react";
import {
  RepoReadiness,
  CloneProgress,
  LargeFileScan,
  cancelRepoSetup,
  initRepo,
  commitRepo,
  inspectRepo,
  scanLargeFiles,
} from "../api";
import { repoSetupView, largeFileSummary } from "./repoSetupView";
import { formatSize } from "./git/binary";
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
  // Large files in the folder, once the background scan finds any.
  const [scan, setScan] = useState<LargeFileScan | null>(null);
  const [ignoreLarge, setIgnoreLarge] = useState(true);
  // Set by Cancel so a git that finishes anyway can't resolve the dialog the
  // user has already closed.
  const cancelled = useRef(false);

  const view = repoSetupView(current, context);

  // Escape matches the Cancel button, which now works mid-op too.
  useModalKeys(cancel);

  // Defensive: if we're already ready+clean (caller normally avoids opening then), resolve.
  useEffect(() => {
    if (view.kind === "ready") onResolved();
  }, [view.kind]);

  // Look for files big enough to make the first commit an hours-long mistake.
  // Only for folders that still need one, so adding a normal project never
  // waits on it, and off the button's path: it lands when it lands.
  useEffect(() => {
    if (readiness.state === "ready") return;
    let live = true;
    scanLargeFiles(repoPath)
      .then((s) => {
        if (live && s.count > 0) setScan(s);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [repoPath]);

  if (view.kind === "ready") return null;

  async function doInit() {
    setBusy(true); setError(""); setProgress(null);
    try {
      await initRepo(repoPath);
      // After init the folder always has no commits — advance to the commit step.
      const next = await inspectRepo(repoPath);
      if (!cancelled.current) setCurrent(next);
    } catch (e) { if (!cancelled.current) setError(String(e)); }
    finally { setBusy(false); setProgress(null); }
  }

  async function doCommit() {
    setBusy(true); setError(""); setProgress(null);
    try {
      // Only write a default .gitignore when creating the very first commit of a
      // brand-new repo — there it stops node_modules/target getting committed and
      // never overwrites an existing file. For an already-established repo with
      // uncommitted changes we don't touch it.
      await commitRepo(repoPath, view.kind === "commit", ignorePaths(), setProgress);
      if (!cancelled.current) onResolved();
    } catch (e) { if (!cancelled.current) setError(String(e)); }
    finally { setBusy(false); setProgress(null); }
  }

  // The large files the user chose to leave out, if any.
  function ignorePaths(): string[] {
    return scan && ignoreLarge ? scan.ignorePaths : [];
  }

  // Cancel means cancel, mid-op included: staging a folder of model weights can
  // run for many minutes, so the backend is told to kill the git it's waiting on
  // and the dialog closes now rather than when git gets around to it.
  function cancel() {
    cancelled.current = true;
    // Unconditional: it's a no-op when nothing is running, and this must not
    // depend on `busy` having reached this closure.
    void cancelRepoSetup(repoPath).catch(() => {});
    onCancel();
  }

  const onPrimary = view.kind === "init" ? doInit : doCommit;
  // Shown until the backend's first progress message lands. `init` streams
  // nothing — it's a fast `git init` plus a rescan of the folder.
  const startingLabel = view.kind === "init" ? "Setting up repository…" : "Staging files…";
  const unlisted = scan ? scan.count - scan.files.length : 0;

  return (
    // A stray backdrop click still can't abort a running op; Cancel, X and
    // Escape are the deliberate ways out.
    <div className="modal-backdrop" onClick={(e) => { e.stopPropagation(); if (!busy) cancel(); }}>
      <div className="modal confirm" role="dialog" aria-modal="true" aria-label={view.title} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{view.title}</h3>
          <button className="modal-x" onClick={cancel}>✕</button>
        </div>
        <div className="modal-body">
          {view.body}
          {scan && (
            <div className="repo-large">
              <div className="repo-large-head">Large files found</div>
              <p>
                {largeFileSummary(scan)}. Committing them copies them into the repository's
                history, which takes a long time and roughly doubles the disk space they use.
              </p>
              <ul className="repo-large-list">
                {scan.files.map((f) => (
                  <li key={f.path}>
                    <span className="repo-large-path">{f.path}</span>
                    <span className="repo-large-size">{formatSize(f.bytes)}</span>
                  </li>
                ))}
                {unlisted > 0 && <li className="repo-large-more">and {unlisted} more</li>}
              </ul>
              <label className="repo-large-check">
                <input
                  type="checkbox"
                  checked={ignoreLarge}
                  disabled={busy}
                  onChange={(e) => setIgnoreLarge(e.target.checked)}
                />
                <span>Leave them out (adds them to .gitignore)</span>
              </label>
              {ignoreLarge && (
                <div className="repo-large-rules">.gitignore: {scan.ignorePaths.join(", ")}</div>
              )}
            </div>
          )}
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
          <button className="btn-secondary" onClick={cancel}>Cancel</button>
          {view.secondaryLabel && (
            <button className="btn-secondary" disabled={busy} onClick={onResolved}>{view.secondaryLabel}</button>
          )}
          <button className="btn-primary" disabled={busy} onClick={onPrimary}>{view.primaryLabel}</button>
        </div>
      </div>
    </div>
  );
}
