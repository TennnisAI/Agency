import { useState } from "react";
import { CloneProgress, RunInfo, archiveRun, discardRun } from "../api";
import { useRuns } from "../store/runs";
import { toastError } from "../lib/toast";
import { Removal, removalCopy } from "../lib/runRemoval";
import ConfirmDialog from "./ConfirmDialog";

/**
 * Confirm-and-perform for archiving or discarding a run. Every place a run can
 * be removed from (the grid tile, the focus header, the agents rail, the Agent
 * menu) shares this, so the wording and the aftermath are the same everywhere:
 * the run that just went away stops being the focused one, and the list
 * refreshes. `onRemoved` is for whatever else the caller has to put back after
 * the run is gone; it runs only when the removal actually succeeded.
 */
export default function RunRemoveDialog({
  run,
  action,
  onClose,
  onRemoved,
}: {
  run: RunInfo;
  action: Removal;
  onClose: () => void;
  onRemoved?: () => void;
}) {
  const { focusedRunId, setFocusedRun, refreshRuns } = useRuns();
  const [busy, setBusy] = useState(false);
  // Which teardown step the backend is on. Both actions stop a session and
  // hand git a worktree to unlink, so on a big repo they run for seconds.
  const [progress, setProgress] = useState<CloneProgress | null>(null);
  const copy = removalCopy(run, action);
  return (
    <ConfirmDialog
      title={copy.title}
      body={copy.body}
      confirmLabel={copy.confirmLabel}
      danger={copy.danger}
      busy={busy}
      progress={progress}
      progressLabel={action === "archive" ? "Archiving…" : "Deleting…"}
      onConfirm={async () => {
        setBusy(true);
        setProgress(null);
        try {
          await (action === "archive"
            ? archiveRun(run.id, setProgress)
            : discardRun(run.id, setProgress));
          if (focusedRunId === run.id) setFocusedRun(null);
          onRemoved?.();
          await refreshRuns();
        } catch (e) {
          toastError(e, copy.failTitle);
        } finally {
          setBusy(false);
          onClose();
        }
      }}
      onCancel={onClose}
    />
  );
}
