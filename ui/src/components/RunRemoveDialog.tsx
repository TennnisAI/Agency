import { useState } from "react";
import { RunInfo, archiveRun, discardRun } from "../api";
import { useRuns } from "../store/runs";
import { toastError } from "../lib/toast";
import { Removal, removalCopy } from "../lib/runRemoval";
import ConfirmDialog from "./ConfirmDialog";

/**
 * Confirm-and-perform for archiving or discarding a run. Every place a run can
 * be removed from (the grid tile, the focus header, the agents rail) shares
 * this, so the wording and the aftermath are the same everywhere: the run that
 * just went away stops being the focused one, and the list refreshes.
 */
export default function RunRemoveDialog({
  run,
  action,
  onClose,
}: {
  run: RunInfo;
  action: Removal;
  onClose: () => void;
}) {
  const { focusedRunId, setFocusedRun, refreshRuns } = useRuns();
  const [busy, setBusy] = useState(false);
  const copy = removalCopy(run, action);
  return (
    <ConfirmDialog
      title={copy.title}
      body={copy.body}
      confirmLabel={copy.confirmLabel}
      danger={copy.danger}
      busy={busy}
      onConfirm={async () => {
        setBusy(true);
        try {
          await (action === "archive" ? archiveRun(run.id) : discardRun(run.id));
          if (focusedRunId === run.id) setFocusedRun(null);
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
