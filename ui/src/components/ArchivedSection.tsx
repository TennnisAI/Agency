import { useCallback, useEffect, useState } from "react";
import { CloneProgress, RunInfo, listArchivedRuns, restoreRun, discardRun, discardArchivedRuns } from "../api";
import { useRuns } from "../store/runs";
import { toastError, toastSuccess } from "../lib/toast";
import ConfirmDialog from "./ConfirmDialog";
import { TrashIcon } from "./icons";

export default function ArchivedSection() {
  const { selectedProjectId, refreshRuns, setFocusedRun } = useRuns();
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<RunInfo[]>([]);
  // Permanent discard is gated behind a confirm; `busy` covers restore,
  // discard, and the bulk cleanup so a slow op can't be double-fired.
  const [confirmDiscard, setConfirmDiscard] = useState<RunInfo | null>(null);
  const [confirmCleanup, setConfirmCleanup] = useState(false);
  const [busy, setBusy] = useState(false);
  // Teardown step of the discard or sweep in flight. A whole project's worth of
  // archived worktrees is the slowest removal in the app, and it used to show
  // nothing at all until it finished.
  const [progress, setProgress] = useState<CloneProgress | null>(null);

  const load = useCallback(async () => {
    if (!selectedProjectId) return setItems([]);
    try {
      setItems(await listArchivedRuns(selectedProjectId));
    } catch {
      /* ignore */
    }
  }, [selectedProjectId]);

  useEffect(() => {
    if (open) load();
  }, [open, load]);

  if (!selectedProjectId) return null;

  const restore = async (r: RunInfo) => {
    if (busy) return;
    setBusy(true);
    try {
      const restored = await restoreRun(r.id);
      await refreshRuns();
      await load();
      setFocusedRun(restored.id);
    } catch (e) {
      toastError(e, "Restore failed");
    } finally {
      setBusy(false);
    }
  };

  const discard = async (r: RunInfo) => {
    setBusy(true);
    setProgress(null);
    try {
      await discardRun(r.id, setProgress);
      await load();
      setConfirmDiscard(null);
    } catch (e) {
      toastError(e, "Discard failed");
      setConfirmDiscard(null);
    } finally {
      setBusy(false);
    }
  };

  // Discard every archived run at once. Partially successful sweeps are
  // reported rather than swallowed: some runs can go while another's branch
  // refuses to delete.
  const cleanUp = async () => {
    setBusy(true);
    setProgress(null);
    try {
      const { discarded, failed } = await discardArchivedRuns(selectedProjectId, setProgress);
      await load();
      setConfirmCleanup(false);
      if (discarded) toastSuccess(`Cleaned up ${discarded} archived agent${discarded === 1 ? "" : "s"}`);
      if (failed.length) toastError(failed.join("; "), "Some agents could not be cleaned up");
    } catch (e) {
      toastError(e, "Cleanup failed");
      setConfirmCleanup(false);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="archived">
      <div className="archived-head">
        <button className="archived-toggle" onClick={() => setOpen((o) => !o)}>
          {open ? "▾" : "▸"} Archived{items.length ? ` (${items.length})` : ""}
        </button>
        {/* Bulk cleanup only while the list is expanded, so what the button is
            about to delete is on screen next to it. */}
        {open && items.length > 0 && (
          <button
            className="archived-cleanup"
            title="Discard every archived agent and its worktree"
            disabled={busy}
            onClick={() => setConfirmCleanup(true)}
          >Clean up</button>
        )}
      </div>
      {open &&
        items.map((r) => (
          <div key={r.id} className="archived-row">
            <span className="archived-name">{r.agent}: {r.prompt || r.branch}</span>
            <button
              className="icon-btn"
              title="Restore"
              disabled={busy}
              onClick={() => restore(r)}
            >↺</button>
            <button
              className="icon-btn danger"
              title="Discard permanently"
              disabled={busy}
              onClick={() => setConfirmDiscard(r)}
            ><TrashIcon /></button>
          </div>
        ))}
      {open && items.length === 0 && <div className="archived-empty">Nothing archived.</div>}
      {confirmDiscard && (
        <ConfirmDialog
          title="Discard archived agent?"
          body={`Permanently delete this run and its "${confirmDiscard.branch}" branch. Any unmerged work on that branch is lost. This cannot be undone.`}
          confirmLabel="Discard"
          danger
          busy={busy}
          progress={progress}
          progressLabel="Discarding…"
          onConfirm={() => discard(confirmDiscard)}
          onCancel={() => setConfirmDiscard(null)}
        />
      )}
      {confirmCleanup && (
        <ConfirmDialog
          title="Clean up all archived agents?"
          body={`Permanently delete ${items.length} archived agent${items.length === 1 ? "" : "s"}, along with ${items.length === 1 ? "its worktree and branch" : "their worktrees and branches"}. Any unmerged work on those branches is lost. This cannot be undone.`}
          confirmLabel="Clean up"
          danger
          busy={busy}
          progress={progress}
          progressLabel="Cleaning up…"
          onConfirm={cleanUp}
          onCancel={() => setConfirmCleanup(false)}
        />
      )}
    </div>
  );
}
