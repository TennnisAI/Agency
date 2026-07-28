import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Project, createWorkspace, defaultWorkspaceLocation } from "../api";
import { useModalKeys } from "../hooks/useModalKeys";
import Toggle from "./Toggle";

/**
 * First-use setup for the pinned workspace: pick where it lives (default
 * `~/Agency`) and whether to keep history with git. Git is default-on — it's
 * what lets agents work on notes in worktrees — but never required.
 */
export default function WorkspaceCreateDialog({
  onCreated,
  onCancel,
}: {
  onCreated: (ws: Project) => void;
  onCancel: () => void;
}) {
  const [location, setLocation] = useState("");
  const [useGit, setUseGit] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useModalKeys(onCancel, !busy);

  useEffect(() => {
    defaultWorkspaceLocation().then(setLocation).catch(() => {});
  }, []);

  async function choose() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel === "string") setLocation(sel);
  }

  async function create() {
    setBusy(true);
    setError("");
    try {
      onCreated(await createWorkspace(location, useGit));
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={(e) => { e.stopPropagation(); if (!busy) onCancel(); }}>
      <div className="modal confirm" role="dialog" aria-modal="true" aria-label="Create workspace" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>Create your workspace</h3>
          <button className="modal-x" disabled={busy} onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          <p>
            The workspace is your home for journaling, planning, and notes across
            projects — plain markdown files, editable here, by agents, or by any
            other tool.
          </p>
          <div className="ws-create-row">
            <code className="ws-create-path" title={location}>{location || "…"}</code>
            <button className="btn-secondary" disabled={busy} onClick={choose}>Choose…</button>
          </div>
          <label className="ws-create-git">
            <Toggle checked={useGit} onChange={setUseGit} />
            <span>
              Keep history with git
              <span className="ws-create-hint">
                Recommended — this is what lets agents draft and edit notes on
                their own branches. Nothing is ever pushed anywhere.
              </span>
            </span>
          </label>
          {error && <div className="git-error">{error}</div>}
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" disabled={busy} onClick={onCancel}>Cancel</button>
          <button className="btn-primary" disabled={busy || !location} onClick={create}>
            {busy ? "Creating…" : "Create workspace"}
          </button>
        </div>
      </div>
    </div>
  );
}
