import { useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { cancelClone, cloneRepo, ghAuthReadiness, GhReadiness, CloneProgress } from "../api";
import { useModalKeys } from "../hooks/useModalKeys";
import ModalBackdrop from "./ModalBackdrop";
import ProgressReadout from "./ProgressReadout";

type Props = {
  // Called with the freshly cloned repo's absolute path once the clone succeeds.
  onCloned: (path: string) => void;
  onCancel: () => void;
};

// Mirror the backend's repo_name_from_url so the dialog can preview the folder
// name that will be created — handles https, scp-style, and trailing slashes.
function repoNameFromUrl(url: string): string {
  const trimmed = url.trim().replace(/\/+$/, "");
  const last = trimmed.split(/[/:]/).pop() ?? trimmed;
  return last.replace(/\.git$/, "");
}

export default function CloneDialog({ onCloned, onCancel }: Props) {
  const [url, setUrl] = useState("");
  const [parentDir, setParentDir] = useState("");
  const [busy, setBusy] = useState(false);
  // Latest git progress update while a clone is running (null before the first).
  const [progress, setProgress] = useState<CloneProgress | null>(null);
  const [error, setError] = useState("");
  // When a clone fails on authentication, we surface a guided sign-in step keyed
  // to how far the user is from being ready (gh missing vs. not signed in).
  const [authHelp, setAuthHelp] = useState<GhReadiness | null>(null);
  // Set by Cancel so a clone that finishes anyway can't resolve the dialog the
  // user has already closed, or report its own killing as an error.
  const cancelled = useRef(false);

  // Escape matches the Cancel button, which now works mid-clone too.
  useModalKeys(cancel);

  const name = repoNameFromUrl(url);
  const canClone = !!url.trim() && !!parentDir && !busy;

  async function chooseLocation() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel === "string") setParentDir(sel);
  }

  async function doClone() {
    if (!canClone) return;
    setBusy(true); setError(""); setAuthHelp(null); setProgress(null);
    try {
      const path = await cloneRepo(url.trim(), parentDir, setProgress);
      if (!cancelled.current) onCloned(path);
    } catch (e) {
      if (cancelled.current) return;
      const msg = String(e);
      setError(msg);
      // The backend's auth guidance mentions signing in to GitHub; when it does,
      // fetch the gh state so we can offer the matching fix (install / sign in).
      if (/sign in to GitHub|gh auth login/i.test(msg)) {
        ghAuthReadiness().then(setAuthHelp).catch(() => {});
      }
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  // Cancel means cancel, mid-clone included: downloading a large repository runs
  // for many minutes, so the backend is told to kill the git it's waiting on
  // (which also deletes the half-downloaded folder) and the dialog closes now
  // rather than when git gets around to it.
  function cancel() {
    cancelled.current = true;
    // Unconditional: it's a no-op when no clone is running, and this must not
    // depend on `busy` having reached this closure.
    void cancelClone(url.trim(), parentDir).catch(() => {});
    onCancel();
  }

  return (
    // A stray backdrop click still can't abort a running clone; Cancel, X and
    // Escape are the deliberate ways out.
    <ModalBackdrop onBackdropClick={busy ? undefined : cancel}>
      <div className="modal confirm" role="dialog" aria-modal="true" aria-label="Clone repository" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>Clone a repository</h3>
          <button className="modal-x" onClick={cancel}>✕</button>
        </div>
        <div className="modal-body">
          <p className="modal-note">Clone an existing Git repository, then add it as a project.</p>
          <label className="clone-field">
            <span className="clone-label">Repository URL</span>
            <input
              className="modal-input"
              autoFocus
              placeholder="https://github.com/owner/repo.git"
              value={url}
              disabled={busy}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && canClone) doClone(); }}
            />
          </label>
          <label className="clone-field">
            <span className="clone-label">Destination folder</span>
            <div className="clone-dest">
              <input
                className="modal-input"
                readOnly
                placeholder="Choose a location…"
                value={parentDir}
                onClick={chooseLocation}
              />
              <button className="btn-secondary" disabled={busy} onClick={chooseLocation}>Choose…</button>
            </div>
          </label>
          {parentDir && name && !busy && (
            <p className="modal-note">Clones into <code>{parentDir}/{name}</code></p>
          )}
          {busy && <ProgressReadout progress={progress} fallback="Starting clone…" />}
          {error && <div className="git-error">{error}</div>}
          {authHelp && authHelp !== "ready" && (
            <div className="clone-auth-help">
              {authHelp === "notInstalled" ? (
                <>
                  <p className="modal-note">
                    Cloning a private repo needs the GitHub CLI (<code>gh</code>), which isn't installed.
                  </p>
                  <button className="btn-secondary" onClick={() => openUrl("https://cli.github.com").catch(() => {})}>
                    Get GitHub CLI ↗
                  </button>
                  <p className="modal-note">After installing, run <code>gh auth login</code>, then retry.</p>
                </>
              ) : (
                <>
                  <p className="modal-note">
                    Sign in first: run <code>gh auth login</code> in a terminal, then retry.
                  </p>
                </>
              )}
              <button className="btn-primary" disabled={!canClone} onClick={doClone}>Retry</button>
            </div>
          )}
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" onClick={cancel}>Cancel</button>
          <button className="btn-primary" disabled={!canClone} onClick={doClone}>
            {busy ? "Cloning…" : "Clone repository"}
          </button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
