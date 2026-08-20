import { useCallback, useEffect, useState } from "react";
import {
  CloneProgress,
  RunInfo,
  listArchivedRuns,
  restoreRun,
  discardRun,
  discardArchivedRuns,
} from "../api";
import { useRuns } from "../store/runs";
import { toastError, toastSuccess } from "../lib/toast";
import ConfirmDialog from "./ConfirmDialog";
import RunRecordDialog from "./RunRecordDialog";
import { TrashIcon } from "./icons";

/**
 * The archive: runs that are finished with, and what is left of each one.
 *
 * An archived run never has a worktree — archiving removes it, and always did.
 * What it may still have is its branch, and only when that branch is the last
 * copy of some work; a merged run's branch goes with the archive, since it is a
 * second name for commits that are on the base. So the two endings look
 * different here and are labelled differently: one can be restored, the other
 * is a record to read. Saying "worktrees and branches" over both, as this
 * section used to, is what made archiving sound expensive.
 */
export default function ArchivedSection() {
  const { selectedProjectId, refreshRuns, setFocusedRun } = useRuns();
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<RunInfo[]>([]);
  // Permanent deletion is gated behind a confirm; `busy` covers restore,
  // delete, and the bulk cleanup so a slow op can't be double-fired.
  const [confirmDelete, setConfirmDelete] = useState<RunInfo | null>(null);
  const [confirmCleanup, setConfirmCleanup] = useState(false);
  const [reading, setReading] = useState<RunInfo | null>(null);
  const [busy, setBusy] = useState(false);
  // Teardown step of the delete or sweep in flight. A whole project's worth of
  // archived runs is the slowest removal in the app, and it used to show
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

  const remove = async (r: RunInfo) => {
    setBusy(true);
    setProgress(null);
    try {
      await discardRun(r.id, setProgress);
      await load();
      setConfirmDelete(null);
    } catch (e) {
      toastError(e, "Delete failed");
      setConfirmDelete(null);
    } finally {
      setBusy(false);
    }
  };

  // Delete every archived run at once. Partially successful sweeps are
  // reported rather than swallowed: some runs can go while another's branch
  // refuses to delete.
  const cleanUp = async () => {
    setBusy(true);
    setProgress(null);
    try {
      const { discarded, failed } = await discardArchivedRuns(selectedProjectId, setProgress);
      await load();
      setConfirmCleanup(false);
      if (discarded) toastSuccess(`Cleared ${discarded} archived agent${discarded === 1 ? "" : "s"}`);
      if (failed.length) toastError(failed.join("; "), "Some agents could not be cleared");
    } catch (e) {
      toastError(e, "Cleanup failed");
      setConfirmCleanup(false);
    } finally {
      setBusy(false);
    }
  };

  // How many of the archived runs are still holding a branch. That is the only
  // part of a sweep that can lose anything, so it is the part the confirm
  // counts rather than talking about all of them at once.
  const holding = items.filter((r) => r.archived?.branchKept).length;

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
            title="Delete every archived agent, its record, and any branch it still holds"
            disabled={busy}
            onClick={() => setConfirmCleanup(true)}
          >Clear</button>
        )}
      </div>
      {open &&
        items.map((r) => {
          const kept = r.archived?.branchKept ?? false;
          return (
            <div key={r.id} className="archived-row">
              <span className="archived-name">{r.agent}: {r.prompt || r.branch}</span>
              {/* What is left of this one, in one word. "Record" is not a
                  lesser archive — it is the normal ending for merged work. */}
              <span className="archived-held" title={kept
                ? `Its ${r.branch} branch is still here, so the agent can be restored onto it`
                : "Its work is elsewhere, so only the record was kept"}>
                {kept ? "branch" : "record"}
              </span>
              {r.archived?.hasRecord && (
                <button
                  className="icon-btn"
                  title="Read the record"
                  disabled={busy}
                  onClick={() => setReading(r)}
                >≡</button>
              )}
              <button
                className="icon-btn"
                title={kept ? "Restore" : "Nothing to restore: this run's branch is gone"}
                disabled={busy || !kept}
                onClick={() => restore(r)}
              >↺</button>
              <button
                className="icon-btn danger"
                title="Delete permanently"
                disabled={busy}
                onClick={() => setConfirmDelete(r)}
              ><TrashIcon /></button>
            </div>
          );
        })}
      {open && items.length === 0 && <div className="archived-empty">Nothing archived.</div>}
      {reading && <RunRecordDialog run={reading} onClose={() => setReading(null)} />}
      {confirmDelete && (
        <ConfirmDialog
          title="Delete archived agent?"
          body={
            confirmDelete.archived?.branchKept
              ? `Permanently delete this run, its record, and its ${confirmDelete.branch} branch, which is the only copy of the work on it.`
              : "Permanently delete this run and its record. It has no branch left to lose, so nothing else goes with it."
          }
          confirmLabel="Delete"
          danger={confirmDelete.archived?.branchKept ?? true}
          busy={busy}
          progress={progress}
          progressLabel="Deleting…"
          onConfirm={() => remove(confirmDelete)}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
      {confirmCleanup && (
        <ConfirmDialog
          title="Clear the archive?"
          body={
            <div className="removal-body">
              <p>
                {`Permanently delete ${items.length} archived agent${
                  items.length === 1 ? "" : "s"
                } and ${items.length === 1 ? "its record" : "their records"}.`}
              </p>
              {holding > 0 && (
                <p className="removal-warn">
                  {`${holding} of them still ${holding === 1 ? "holds a branch" : "hold branches"} that ${
                    holding === 1 ? "is" : "are"
                  } the only copy of unmerged work. That goes too.`}
                </p>
              )}
            </div>
          }
          confirmLabel="Clear"
          danger={holding > 0}
          busy={busy}
          progress={progress}
          progressLabel="Clearing…"
          onConfirm={cleanUp}
          onCancel={() => setConfirmCleanup(false)}
        />
      )}
    </div>
  );
}
