import { useCallback, useEffect, useState } from "react";
import { RunInfo, listArchivedRuns, restoreRun, discardRun } from "../api";
import { useRuns } from "../store/runs";
import { toastError } from "../lib/toast";
import ConfirmDialog from "./ConfirmDialog";

export default function ArchivedSection() {
  const { selectedProjectId, refreshRuns, setFocusedRun } = useRuns();
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<RunInfo[]>([]);
  // Permanent discard is gated behind a confirm; `busy` covers both restore
  // and discard so a slow op can't be double-fired.
  const [confirmDiscard, setConfirmDiscard] = useState<RunInfo | null>(null);
  const [busy, setBusy] = useState(false);

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
    try {
      await discardRun(r.id);
      await load();
      setConfirmDiscard(null);
    } catch (e) {
      toastError(e, "Discard failed");
      setConfirmDiscard(null);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="archived">
      <button className="archived-head" onClick={() => setOpen((o) => !o)}>
        {open ? "▾" : "▸"} Archived{items.length ? ` (${items.length})` : ""}
      </button>
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
            >✕</button>
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
          onConfirm={() => discard(confirmDiscard)}
          onCancel={() => setConfirmDiscard(null)}
        />
      )}
    </div>
  );
}
