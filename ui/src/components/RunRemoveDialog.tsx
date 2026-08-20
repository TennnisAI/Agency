import { useEffect, useState } from "react";
import { CloneProgress, RunCleanup, RunInfo, archiveRun, discardRun, runCleanup } from "../api";
import { useRuns } from "../store/runs";
import { toastError } from "../lib/toast";
import { Removal, removalCopy } from "../lib/runRemoval";
import ConfirmDialog from "./ConfirmDialog";
import RemovalSummary from "./RemovalSummary";

/**
 * Confirm-and-perform for archiving or deleting a run. Every place a run can be
 * removed from (the grid tile, the focus header, the agents rail, the Agent
 * menu) shares this, so the wording and the aftermath are the same everywhere:
 * the run that just went away stops being the focused one, and the list
 * refreshes. Removing the run you were working in also steps back out to the
 * grid — there is no worktree left to stand in — which points source control at
 * your own checkout; for an agent the panel is opened too, so a merge that has
 * just been archived is visibly sitting there unpushed rather than forgotten.
 * `onRemoved` is for whatever else the caller has to put back after the run is
 * gone; it runs only when the removal actually succeeded.
 *
 * The dialog asks the backend what this teardown would remove before it says
 * anything about it. That answer is about *this* branch — merged, pushed, or
 * the only copy of three commits — and it is what decides both the wording and
 * whether the confirm is a red button.
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
  const { focusedRunId, setFocusedRun, setView, setSourcePanelOpen, refreshRuns } = useRuns();
  const [busy, setBusy] = useState(false);
  // Which teardown step the backend is on. Both actions stop a session and
  // hand git a worktree to unlink, so on a big repo they run for seconds.
  const [progress, setProgress] = useState<CloneProgress | null>(null);
  const [cleanup, setCleanup] = useState<RunCleanup | null>(null);
  // Null while the probe is in flight; set if it fails, so the dialog says it
  // is working from less than it wanted rather than silently guessing.
  const [probeFailed, setProbeFailed] = useState(false);

  // A terminal owns no branch, so there is nothing to ask git about.
  useEffect(() => {
    if (run.kind === "terminal") return;
    let live = true;
    runCleanup(run.id)
      .then((c) => live && setCleanup(c))
      .catch(() => live && setProbeFailed(true));
    return () => {
      live = false;
    };
  }, [run.id, run.kind]);

  const copy = removalCopy(run, action, cleanup);
  const checking = !cleanup && !probeFailed && run.kind === "agent";
  return (
    <ConfirmDialog
      title={copy.title}
      body={<RemovalSummary copy={copy} checking={checking} probeFailed={probeFailed} />}
      confirmLabel={copy.confirmLabel}
      danger={copy.danger}
      busy={busy}
      // Confirming before the branch has been read would go ahead with the
      // right teardown but the wrong explanation, which is the failure this
      // whole dialog exists to prevent. Cancel stays live throughout.
      confirmDisabled={checking}
      progress={progress}
      progressLabel={action === "archive" ? "Archiving…" : "Deleting…"}
      onConfirm={async () => {
        setBusy(true);
        setProgress(null);
        try {
          await (action === "archive"
            ? archiveRun(run.id, setProgress)
            : discardRun(run.id, setProgress));
          if (focusedRunId === run.id) {
            setFocusedRun(null);
            setView("grid");
            // Only for agents: closing a terminal ends nothing that could be
            // waiting to be pushed, and the panel would just be in the way.
            if (run.kind === "agent") setSourcePanelOpen(true);
          }
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
