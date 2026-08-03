import { useEffect, useState } from "react";
import {
  MergeOutcome,
  MergePreview,
  MergeState,
  abortMergeTask,
  archiveRun,
  discardRun,
  finishMergeTask,
  mergePreview,
  mergeStatus,
  mergeTask,
  sendMergeConflict,
} from "../api";
import PrSection from "./PrSection";
import ConfirmDialog from "./ConfirmDialog";
import { useRuns } from "../store/runs";
import { useModalKeys } from "../hooks/useModalKeys";

// `onRemoved` fires after a post-merge cleanup action (archive or delete) so
// the host view can drop focus and refresh its rail.
export default function MergeModal({
  taskId,
  onClose,
  onRemoved,
  onReviewPr,
}: {
  taskId: string;
  onClose: () => void;
  onRemoved?: () => void;
  // Deep-link handler for "Review in Agency" — opens the PR in the review panel.
  onReviewPr?: (number: number) => void;
}) {
  const [preview, setPreview] = useState<MergePreview | null>(null);
  const [outcome, setOutcome] = useState<MergeOutcome | null>(null);
  const [merging, setMerging] = useState(false);
  const [error, setError] = useState("");
  // "Fix with agent": the prompt is typed into this run's own agent session, so
  // there is nothing to stream here — only whether the hand-off has happened.
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState(false);
  // What git says about the merge after a resolver has had a go at it. Null
  // until the first check.
  const [state, setState] = useState<MergeState | null>(null);
  const [finishing, setFinishing] = useState(false);
  const [archiving, setArchiving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const { runs } = useRuns();
  const me = runs.find((r) => r.id === taskId);
  const projectId = me?.projectId ?? null;
  // Losing race attempts: siblings sharing this run's race_id. Offered for
  // cleanup after the winner merges.
  const losers = me?.raceId
    ? runs.filter((r) => r.raceId === me.raceId && r.id !== taskId && r.kind === "agent")
    : [];
  // Cleanup in flight: archiving or deleting. Both tear the run down, so the
  // modal's cancel affordances stay disabled until they land.
  const busy = archiving || deleting;

  // Escape mirrors the header ✕; disabled while cleaning up (the modal's other
  // cancel affordances are disabled then too).
  useModalKeys(onClose, !busy);

  // Load the preview first so we can explain what a merge would do instead of
  // silently running it the moment the modal opens.
  useEffect(() => {
    mergePreview(taskId).then(setPreview).catch((e) => setError(String(e)));
    // A merge left in progress by an earlier visit (the modal was closed
    // mid-resolution) resumes where it stopped instead of offering a fresh
    // merge that can only fail while the old one is unfinished.
    mergeStatus(taskId)
      .then((s) => {
        setState(s);
        if (s.merging) setOutcome({ kind: "conflicts", files: s.unresolved });
      })
      .catch(() => {});
  }, [taskId]);

  async function archiveWorkspace(discardLosers: boolean) {
    setArchiving(true);
    try {
      if (discardLosers) {
        // The merged branch won; the other attempts' work is unwanted by
        // definition, so a full discard (worktree + branch) is right.
        for (const l of losers) await discardRun(l.id);
      }
      await archiveRun(taskId);
      onRemoved?.();
      onClose();
    } catch (e) {
      setError(String(e));
      setArchiving(false);
    }
  }

  // Delete: the merged work is on the base branch now, so some users want the
  // agent gone for good rather than filed away. Removes the worktree, the
  // agent branch and the run record itself.
  async function deleteWorkspace(discardLosers: boolean) {
    setDeleting(true);
    try {
      if (discardLosers) {
        for (const l of losers) await discardRun(l.id);
      }
      await discardRun(taskId);
      onRemoved?.();
      onClose();
    } catch (e) {
      setError(String(e));
      setDeleting(false);
      // Drop back to the merge modal so the error is the thing on screen.
      setConfirmDelete(false);
    }
  }

  // Failure keeps the modal open with the error visible instead of silently
  // dropping the rejection and leaving the merge half-aborted.
  function abortAndClose() {
    abortMergeTask(taskId)
      .then(onClose)
      .catch((e) => setError(String(e)));
  }

  async function attempt() {
    setError("");
    setMerging(true);
    try {
      setOutcome(await mergeTask(taskId));
    } catch (e) {
      setError(String(e));
    } finally {
      setMerging(false);
    }
  }

  // Hand the conflict to the agent that produced the branch: it already has the
  // context for the change, and it is a session the user can watch and talk to.
  async function fixWithAgent() {
    setError("");
    setSending(true);
    try {
      await sendMergeConflict(taskId);
      setSent(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
    }
  }

  // Ask git where the merge stands. Never re-runs the merge: a merge in
  // progress is a dirty checkout, so a second attempt could only report that
  // as an error (which is exactly what it used to do).
  async function checkState() {
    setError("");
    try {
      setState(await mergeStatus(taskId));
    } catch (e) {
      setError(String(e));
    }
  }

  // Commit the resolved merge (or accept the one the agent committed) and
  // land on the same "done" screen a clean merge reaches.
  async function finish() {
    setError("");
    setFinishing(true);
    try {
      setOutcome(await finishMergeTask(taskId));
    } catch (e) {
      setError(String(e));
      // The failure is usually "still conflicted"; re-read so the file list
      // shown next to the error is the current one.
      await checkState();
    } finally {
      setFinishing(false);
    }
  }

  // While the agent works on the conflict, keep asking git where the merge
  // stands so "Finish merge" lights up on its own instead of waiting for the
  // user to guess when to press "Check again".
  useEffect(() => {
    if (!sent) return;
    const timer = window.setInterval(() => {
      mergeStatus(taskId).then(setState).catch(() => {});
    }, 3000);
    return () => window.clearInterval(timer);
  }, [sent, taskId]);

  const conflicts = outcome?.kind === "conflicts";
  // Finishing is possible once nothing is unmerged and there is a merge to
  // finish: either uncommitted (we commit it) or already committed by the agent
  // (we just record it and restore the branch).
  const canFinish = !!state && state.unresolved.length === 0 && (state.merging || state.merged);
  const step = outcome?.kind === "clean"
    ? "done"
    : conflicts
      ? (sent ? "resolve" : "merge")
      : merging
        ? "merge"
        : "review";
  const steps: { key: string; label: string }[] = [
    { key: "review", label: "Review" },
    { key: "merge", label: "Merge" },
    { key: "resolve", label: "Resolve" },
    { key: "done", label: "Done" },
  ];

  const nothingToMerge = !!preview && preview.commitsAhead === 0;

  return (
    <div className="settings-overlay" onClick={() => { if (!busy) onClose(); }}>
      <div
        className="merge-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Approve and merge"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="settings-head">
          <h2>Approve &amp; merge</h2>
          <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
        </div>
        <div className="merge-timeline">
          {steps.map((s) => (
            <span key={s.key} className={`mt-step ${s.key === step ? "on" : ""}`}>{s.label}</span>
          ))}
        </div>
        {error && <div className="git-error">{error}</div>}

        {/* Review: explain what will happen before merging. */}
        {!outcome && !merging && (
          !preview && !error ? (
            <p>Checking…</p>
          ) : preview ? (
            <div className="merge-review">
              <p className="merge-summary">
                Merge <code>{preview.branch}</code> → <code>{preview.base}</code>
              </p>
              {nothingToMerge ? (
                <p className="merge-warn">
                  Nothing to merge — this agent has no committed changes on top of <code>{preview.base}</code>.
                </p>
              ) : (
                <p className="merge-note">
                  {preview.commitsAhead} commit{preview.commitsAhead === 1 ? "" : "s"} ahead of <code>{preview.base}</code>.
                </p>
              )}
              {!nothingToMerge && preview.commitsBehind > 0 && (
                <p className="merge-warn">
                  <code>{preview.base}</code> has moved ahead by {preview.commitsBehind} commit
                  {preview.commitsBehind === 1 ? "" : "s"} since this agent branched, so the merge may hit conflicts.
                </p>
              )}
              {preview.worktreeDirty && (
                <p className="merge-warn">
                  {preview.dirtyFiles.length} uncommitted change{preview.dirtyFiles.length === 1 ? "" : "s"} in this
                  agent's worktree {nothingToMerge ? "exist but aren't committed" : "won't be included"}. Only committed
                  work is merged. Commit them in Source Control first to include them.
                </p>
              )}
              {!nothingToMerge && (
                <div className="git-actions">
                  <button autoFocus onClick={attempt}>Merge into {preview.base}</button>
                  <button className="ghost" onClick={onClose}>Cancel</button>
                </div>
              )}
              {projectId && (
                <PrSection
                  taskId={taskId}
                  projectId={projectId}
                  canCreate={!nothingToMerge}
                  onLeave={onClose}
                  onReviewPr={onReviewPr}
                />
              )}
            </div>
          ) : null
        )}

        {merging && <p>Merging…</p>}

        {outcome?.kind === "clean" && (
          <div>
            <p className="merge-ok">✓ Merged cleanly into {preview?.base ?? "main"}.</p>
            <code>{outcome.commit.slice(0, 10)}</code>
            <p className="merge-note">
              The worktree and its <code>{preview?.branch ?? "agent"}</code> branch are no longer needed.
              Archiving stops the agent and removes the worktree (the branch is kept, so it can be restored).
              Deleting removes the branch and the run too, which can't be undone.
            </p>
            <div className="git-actions">
              {losers.length > 0 ? (
                <button disabled={busy} onClick={() => archiveWorkspace(true)}>
                  {archiving
                    ? "Cleaning up…"
                    : `Archive + discard ${losers.length} losing attempt${losers.length === 1 ? "" : "s"}`}
                </button>
              ) : (
                <button disabled={busy} onClick={() => archiveWorkspace(false)}>
                  {archiving ? "Archiving…" : "Archive agent"}
                </button>
              )}
              {losers.length > 0 && (
                <button className="ghost" disabled={busy} onClick={() => archiveWorkspace(false)}>
                  Archive, keep losers
                </button>
              )}
              <button className="ghost danger" disabled={busy} onClick={() => setConfirmDelete(true)}>
                {deleting ? "Deleting…" : "Delete agent"}
              </button>
              <button className="ghost" disabled={busy} onClick={onClose}>Keep agent</button>
            </div>
          </div>
        )}

        {outcome?.kind === "conflicts" && (
          <div>
            <p className="merge-warn">
              {outcome.files.length > 0
                ? `Conflicts in ${outcome.files.length} file(s):`
                : "A merge is in progress in the project's checkout."}
            </p>
            {outcome.files.length > 0 && (
              <ul className="profile-list">
                {outcome.files.map((f) => (
                  <li key={f}>
                    <code>{f}</code>
                  </li>
                ))}
              </ul>
            )}
            {sent && (
              <p className="merge-note">
                Sent to this agent's session, with git's status of the merge. Close this window to
                watch it work; the merge is in the project's checkout, not the agent's worktree, so
                the prompt points git there. This panel rechecks git every few seconds.
              </p>
            )}
            {sent && state && state.unresolved.length > 0 && (
              <p className="merge-warn">
                Still conflicted: {state.unresolved.join(", ")}.
              </p>
            )}
            {sent && canFinish && (
              <p className="merge-note">
                {state?.merging
                  ? "Conflicts resolved. Finishing commits the merge."
                  : "The agent committed the merge itself. Finishing records it and puts your checkout back."}
              </p>
            )}
            {sent && state && !state.merging && !state.merged && (
              <p className="merge-warn">
                No merge in progress. It was aborted or undone, so nothing is left to finish.
              </p>
            )}
            <div className="git-actions">
              <button disabled={!canFinish || finishing} onClick={finish}>
                {finishing ? "Finishing…" : "Finish merge"}
              </button>
              {/* The primary move on a fresh conflict; once the merge can be
                  finished (or the prompt is already sent) it steps back. */}
              <button
                className={canFinish || sent ? "ghost" : undefined}
                disabled={sending}
                onClick={fixWithAgent}
              >
                {sending ? "Sending…" : sent ? "Send again" : "Fix with agent"}
              </button>
              <button className="ghost" disabled={finishing} onClick={checkState}>
                Check again
              </button>
              <button className="ghost" disabled={finishing} onClick={abortAndClose}>Abort merge</button>
            </div>
          </div>
        )}
      </div>

      {/* Sibling of the modal card, not a child: the card scrolls its own
          overflow, and the confirm has to cover the whole overlay. Its own
          backdrop click stops propagating, so it never closes the merge modal
          underneath. */}
      {confirmDelete && (
        <ConfirmDialog
          title="Delete agent?"
          body={`Stop this agent, remove its worktree, and delete the "${preview?.branch ?? "agent"}" branch and the run itself. The merged commit stays on ${preview?.base ?? "the base branch"}, so the work isn't lost, but the agent can't be restored.${
            losers.length > 0
              ? ` "Delete all" also discards ${losers.length} losing attempt${losers.length === 1 ? "" : "s"}.`
              : ""
          }`}
          confirmLabel={losers.length > 0 ? `Delete all ${losers.length + 1}` : "Delete"}
          danger
          busy={deleting}
          altLabel={losers.length > 0 ? "Delete this one" : undefined}
          altDanger
          onAlt={losers.length > 0 ? () => deleteWorkspace(false) : undefined}
          onConfirm={() => deleteWorkspace(losers.length > 0)}
          onCancel={() => setConfirmDelete(false)}
        />
      )}
    </div>
  );
}
