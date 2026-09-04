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
import { restorable, restoreTitle } from "../lib/restore";
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
 * second name for commits that are on the base.
 *
 * Both endings restore. Where the branch survived, the worktree goes back onto
 * it; where it did not, the branch is cut again from the base its work landed
 * on and the rescued conversation is reinstated, so the agent resumes on top of
 * what it merged. Which of the two is about to happen is in the Restore
 * tooltip; see `lib/restore.ts`. Only a run whose base is gone as well has
 * nothing to come back from.
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
        items.map((r) => (
            <div key={r.id} className="archived-row">
              {/* The row used to carry a "branch"/"record" chip here. On a
                  sidebar this narrow it cost more of the title than it was
                  worth, and every row after a merge said the same word —
                  "record" — so it distinguished nothing. What it was for now
                  lives in the Restore button's own tooltip, which is where the
                  distinction is acted on.

                  The name itself opens the record, for the same reason: the ≡
                  button that used to sit in the actions spent width on an
                  affordance the row already had. A run with neither a record
                  nor a rescued conversation has nothing to open, so it stays
                  plain text. */}
              {r.archived?.hasRecord || r.archived?.hasConversation ? (
                <button
                  className="archived-name archived-name-open"
                  title="Read the record and the conversation"
                  disabled={busy}
                  onClick={() => setReading(r)}
                >
                  {r.agent}: {r.prompt || r.branch}
                </button>
              ) : (
                <span className="archived-name" title={`${r.agent}: ${r.prompt || r.branch}`}>
                  {r.agent}: {r.prompt || r.branch}
                </span>
              )}
              <div className="archived-acts">
                <button
                  className="icon-btn"
                  title={restoreTitle(r)}
                  disabled={busy || !restorable(r)}
                  onClick={() => restore(r)}
                >↺</button>
                <button
                  className="icon-btn danger"
                  title="Delete permanently"
                  disabled={busy}
                  onClick={() => setConfirmDelete(r)}
                ><TrashIcon /></button>
              </div>
            </div>
        ))}
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
