import { useEffect, useState } from "react";
import { Checkpoint, CheckpointPreview, checkpointPreview, checkpointRestore } from "../../api";
import { toastSuccess } from "../../lib/toast";
import { useRuns } from "../../store/runs";
import ConfirmDialog from "../ConfirmDialog";
import { canRestore, checkpointLabel, restoreLines } from "./checkpoints";

/**
 * Confirm, then put a run's files back to a checkpoint. What would change is
 * read before the button is live, so the dialog says what it is about to do
 * rather than what it might do.
 *
 * The one thing it must never let a reader assume is that the agent goes back
 * too: a checkpoint holds files, and the conversation stays where it is across
 * every harness we launch. So that is said in the dialog every time, not left
 * to a help page.
 */
export default function CheckpointRestoreDialog({ runId, cp, checkout, onAct, onClose }: {
  runId: string;
  cp: Checkpoint;
  /** The run works in the project checkout rather than a worktree of its own. */
  checkout: boolean;
  onAct: (fn: () => Promise<unknown>, label?: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const [preview, setPreview] = useState<CheckpointPreview | null>(null);
  const [error, setError] = useState("");
  // The backend refuses too, and covers every agent tab; this is the run's own
  // agent, so the dialog can say so before the button is pressed.
  const working = useRuns().runs.find((r) => r.id === runId)?.activity?.state === "working";

  useEffect(() => {
    let cancelled = false;
    checkpointPreview(runId, cp.seq)
      .then((p) => { if (!cancelled) setPreview(p); })
      .catch((e) => { if (!cancelled) setError(String(e)); });
    return () => { cancelled = true; };
  }, [runId, cp.seq]);

  const restore = () => {
    onClose();
    void onAct(async () => {
      const done = await checkpointRestore(runId, cp.seq);
      toastSuccess(
        done.saved != null
          ? `Files restored. What you had is saved as checkpoint ${done.saved}.`
          : "Files restored.",
      );
    });
  };

  const when = new Date(cp.at * 1000).toLocaleString();
  const body = (
    <div className="checkpoint-restore">
      <p>Put this workspace's files back the way they were at {when} ({checkpointLabel(cp.kind).toLowerCase()}).</p>
      {working && (
        <p className="checkpoint-restore-error">
          The agent is working. Wait for it to finish its turn, or stop it, before restoring.
        </p>
      )}
      {error ? (
        <p className="checkpoint-restore-error">{error}</p>
      ) : !preview ? (
        <p className="checkpoint-restore-muted">Reading what would change…</p>
      ) : (
        <ul>{restoreLines(preview, checkout).map((l) => <li key={l}>{l}</li>)}</ul>
      )}
      <p>
        <strong>The agent's memory does not go back.</strong> It still remembers everything after
        this point, so tell it what you undid.
      </p>
      <p className="checkpoint-restore-muted">
        Your files as they are now are saved as a checkpoint first, so you can restore them again.
      </p>
    </div>
  );

  return (
    <ConfirmDialog
      title="Restore files"
      body={body}
      confirmLabel="Restore files"
      danger
      confirmDisabled={working || !preview || !canRestore(preview)}
      onConfirm={restore}
      onCancel={onClose}
    />
  );
}
