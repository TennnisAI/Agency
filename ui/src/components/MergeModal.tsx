import { useEffect, useState } from "react";
import {
  CloneProgress,
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
import ProgressReadout from "./ProgressReadout";
import { useRuns } from "../store/runs";
import { useModalKeys } from "../hooks/useModalKeys";
import { useHushed } from "../lib/hushed";

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
  // "Fix with agent": the prompt is typed into this run's own agent session and
  // submitted there, so there is nothing to stream here, only whether the
  // hand-off has happened.
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState(false);
  // What git says about the merge after a resolver has had a go at it. Null
  // until the first check; `probeError` is why, when it stays null.
  const [state, setState] = useState<MergeState | null>(null);
  const [probeError, setProbeError] = useState("");
  const [finishing, setFinishing] = useState(false);
  const [archiving, setArchiving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  // Latest teardown step of the archive/delete in flight. Both stop a session
  // and hand git a worktree to unlink, which on a large repo runs long enough
  // that a bare "Deleting…" label reads as a frozen app.
  const [cleanup, setCleanup] = useState<CloneProgress | null>(null);
  // Both post-merge explanations can be switched off once the workflow is
  // habit, and switched back on from Settings ▸ Hidden messages.
  const [hushNote, setHushNote] = useHushed("merge-cleanup");
  const [hushConfirm, setHushConfirm] = useHushed("merge-delete");
  // Ticked inside the confirm; only applied if the delete goes ahead.
  const [hushNext, setHushNext] = useState(false);
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

  // Losers are torn down one at a time before the winner, so their progress is
  // labelled with their position: otherwise a five-attempt race looks like one
  // teardown that restarts its phases over and over.
  async function discardLosers() {
    for (const [i, l] of losers.entries()) {
      await discardRun(l.id, (p) =>
        setCleanup({ ...p, detail: `losing attempt ${i + 1} of ${losers.length}` }));
    }
  }

  async function archiveWorkspace(withLosers: boolean) {
    setArchiving(true);
    setCleanup(null);
    try {
      // The merged branch won; the other attempts' work is unwanted by
      // definition, so a full discard (worktree + branch) is right.
      if (withLosers) await discardLosers();
      await archiveRun(taskId, setCleanup);
      onRemoved?.();
      onClose();
    } catch (e) {
      setError(String(e));
      setArchiving(false);
      setCleanup(null);
    }
  }

  // Delete: the merged work is on the base branch now, so some users want the
  // agent gone for good rather than filed away. Removes the worktree, the
  // agent branch and the run record itself.
  async function deleteWorkspace(withLosers: boolean) {
    setDeleting(true);
    setCleanup(null);
    if (hushNext) setHushConfirm(true);
    try {
      if (withLosers) await discardLosers();
      await discardRun(taskId, setCleanup);
      onRemoved?.();
      onClose();
    } catch (e) {
      setError(String(e));
      setDeleting(false);
      setCleanup(null);
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
    // Whatever an earlier check said predates this merge. Clearing it keeps the
    // conflict panel from rendering a stale "nothing in progress" for the frame
    // between the merge landing and the first poll answering.
    setState(null);
    try {
      setOutcome(await mergeTask(taskId));
    } catch (e) {
      setError(String(e));
      // Re-read rather than leave the failure as a bare string: if another run
      // is mid-merge, this is what turns it into the explanation above.
      mergeStatus(taskId).then(setState).catch(() => {});
    } finally {
      setMerging(false);
    }
  }

  // Hand the conflict to the agent that produced the branch: it already has the
  // context for the change, and it is a session the user can watch and talk to.
  // The backend submits the prompt, so this really does start the agent working
  // rather than leaving a composed message sitting in its prompt box.
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
      mergeStatus(taskId).then(setState).catch(() => {});
    } finally {
      setFinishing(false);
    }
  }

  const conflicts = outcome?.kind === "conflicts";

  // Once a merge is in progress, keep asking git where it stands: the resolver
  // is the agent (or the user, by hand, in the project's checkout), so this
  // panel only learns of a resolution by looking. Polling unconditionally, not
  // just after a hand-off, is what makes a reopened window show the merge as it
  // is now rather than as it was when the conflict first appeared.
  useEffect(() => {
    if (!conflicts) return;
    const poll = () =>
      mergeStatus(taskId)
        .then((s) => {
          setProbeError("");
          setState(s);
        })
        // Kept apart from the modal's own error line: a blip here (the index
        // locked while the agent commits, say) clears itself on the next tick,
        // and it must not sit on top of a real failure from Finish or Abort.
        // Shown only while there is no answer at all to show instead.
        .catch((e) => setProbeError(String(e)));
    poll();
    const timer = window.setInterval(poll, 3000);
    return () => window.clearInterval(timer);
  }, [conflicts, taskId]);

  // git's answer wins once it has one; the merge attempt's own file list stands
  // in only for the moment before the first poll lands.
  const unresolved = state ? state.unresolved : outcome?.kind === "conflicts" ? outcome.files : [];
  // Finishing is possible once nothing is unmerged and there is a merge to
  // finish: either uncommitted (we commit it) or already committed by the agent
  // (we just record it and restore the branch).
  const canFinish = !!state && state.unresolved.length === 0 && (state.merging || state.merged);
  const step = outcome?.kind === "clean"
    ? "done"
    : conflicts
      ? "resolve"
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
  // Another run left a merge unfinished in the shared project checkout. Nothing
  // here can proceed until that one is finished or aborted, from its own window.
  const blockedBy = state?.blockedBy ?? null;

  return (
    <div className="settings-overlay anchor-top" onClick={() => { if (!busy) onClose(); }}>
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
              {blockedBy && (
                <p className="merge-warn">
                  <code>{blockedBy}</code> is part-way through a merge in the project's checkout, and every agent
                  merges there. Finish or abort that one from its own Approve window first; this branch is untouched
                  in the meantime.
                </p>
              )}
              {!nothingToMerge && (
                <div className="git-actions">
                  <button autoFocus onClick={attempt} disabled={!!blockedBy}
                    title={blockedBy ? `Blocked: ${blockedBy} is mid-merge` : undefined}>
                    Merge into {preview.base}
                  </button>
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
            <p className="merge-ok">
              ✓ Merged cleanly into {preview?.base ?? "main"} · <code>{outcome.commit.slice(0, 10)}</code>
            </p>
            {!hushNote && (
              <p className="merge-note">
                Tidy up the agent? Archiving keeps its branch, so you can bring it back later. Deleting clears
                the branch and the run out for good. Either way the merged commit stays put.{" "}
                <button className="hush-link" onClick={() => setHushNote(true)}>Don't show this again</button>
              </p>
            )}
            <div className="git-actions">
              {losers.length > 0 ? (
                <button disabled={busy} onClick={() => archiveWorkspace(true)}>
                  {archiving
                    ? "Cleaning up…"
                    : `Archive + discard ${losers.length} losing attempt${losers.length === 1 ? "" : "s"}`}
                </button>
              ) : (
                <button
                  disabled={busy}
                  title="Stop the agent and remove its worktree. The branch is kept, so the agent can be restored."
                  onClick={() => archiveWorkspace(false)}
                >
                  {archiving ? "Archiving…" : "Archive agent"}
                </button>
              )}
              {losers.length > 0 && (
                <button className="ghost" disabled={busy} onClick={() => archiveWorkspace(false)}>
                  Archive, keep losers
                </button>
              )}
              <button
                className="ghost danger"
                disabled={busy}
                title="Remove the worktree, the branch and the run. The merged commit stays on the base branch."
                // Hushed and nothing else to decide: go straight to the delete.
                // With losing attempts around, the confirm carries a real
                // choice (all of them, or just this one), so it still opens.
                onClick={() => {
                  if (hushConfirm && losers.length === 0) deleteWorkspace(false);
                  else { setHushNext(false); setConfirmDelete(true); }
                }}
              >
                {deleting ? "Deleting…" : "Delete agent"}
              </button>
              <button className="ghost" disabled={busy} onClick={onClose} title="Leave the agent as it is">
                Keep agent
              </button>
            </div>
            {/* Archiving has no confirm step of its own, and a hushed delete
                skips its own; either way the teardown says where it is. When
                the confirm is up it shows the same readout, so not twice. */}
            {busy && !confirmDelete && (
              <ProgressReadout progress={cleanup} fallback={archiving ? "Archiving…" : "Deleting…"} />
            )}
          </div>
        )}

        {/* Where the merge actually stands, and only the buttons that state can
            act on. Offering all four moves at once (finish / hand off / recheck
            / abort) made the ones that were dead weight in any given state the
            loudest thing on screen, and read as advice about what to do. */}
        {conflicts && (
          <div>
            {unresolved.length > 0 ? (
              <>
                <p className="merge-warn">Conflicts in {unresolved.length} file(s):</p>
                <ul className="profile-list">
                  {unresolved.map((f) => (
                    <li key={f}>
                      <code>{f}</code>
                    </li>
                  ))}
                </ul>
                {sent ? (
                  <p className="merge-note">
                    Sent to this agent's session, with git's status of the merge. Close this window
                    to watch it work; the merge is in the project's checkout, not the agent's
                    worktree, so the prompt points git there. Reopen this window when it's done, or
                    leave it open: it rechecks git every few seconds either way.
                  </p>
                ) : (
                  <p className="merge-note">
                    Nothing has been committed. Hand the conflict to this agent, or resolve it
                    yourself in the project's checkout and come back here to finish.
                  </p>
                )}
                <div className="git-actions">
                  {sent ? (
                    <button autoFocus onClick={onClose}>Close and watch</button>
                  ) : (
                    <button autoFocus disabled={sending} onClick={fixWithAgent}>
                      {sending ? "Sending…" : "Fix with agent"}
                    </button>
                  )}
                  {sent && (
                    <button className="ghost" disabled={sending} onClick={fixWithAgent}>
                      {sending ? "Sending…" : "Send again"}
                    </button>
                  )}
                  <button
                    className="ghost"
                    onClick={abortAndClose}
                    title="Undo the merge and put the project's checkout back"
                  >
                    Abort merge
                  </button>
                </div>
              </>
            ) : !state ? (
              <>
                {probeError ? (
                  <p className="merge-warn">Couldn't read the merge from git: {probeError}</p>
                ) : (
                  <p>Checking the merge…</p>
                )}
                <div className="git-actions">
                  <button className="ghost" onClick={onClose}>Close</button>
                </div>
              </>
            ) : canFinish ? (
              <>
                <p className="merge-ok">✓ No conflicts remain.</p>
                <p className="merge-note">
                  {state.merging ? (
                    <>
                      {preview && (
                        <>
                          {preview.commitsAhead} commit{preview.commitsAhead === 1 ? "" : "s"} ahead
                          of <code>{preview.base}</code>, resolved and staged.{" "}
                        </>
                      )}
                      Finishing commits the merge and puts your checkout back.
                    </>
                  ) : (
                    "The agent committed the merge itself. Finishing records it and puts your checkout back."
                  )}
                </p>
                <div className="git-actions">
                  <button autoFocus disabled={finishing} onClick={finish}>
                    {finishing ? "Finishing…" : "Finish merge"}
                  </button>
                  <button className="ghost" disabled={finishing} onClick={onClose}>Close</button>
                  <button
                    className="ghost"
                    disabled={finishing}
                    onClick={abortAndClose}
                    title="Undo the merge, and with it the resolution that was just done"
                  >
                    Abort merge
                  </button>
                </div>
              </>
            ) : (
              <>
                <p className="merge-warn">
                  {state.blockedBy
                    ? `The merge in the project's checkout is ${state.blockedBy}'s now, not this agent's. Finish or abort it from its own Approve window.`
                    : "No merge in progress. It was aborted or undone, so nothing is left to finish."}
                </p>
                <div className="git-actions">
                  <button autoFocus onClick={onClose}>Close</button>
                  {!state.blockedBy && (
                    <button className="ghost" onClick={() => { setOutcome(null); setError(""); }}>
                      Back to review
                    </button>
                  )}
                </div>
              </>
            )}
          </div>
        )}
      </div>

      {/* Sibling of the modal card, not a child: the card scrolls its own
          overflow, and the confirm has to cover the whole overlay. Its own
          backdrop click stops propagating, so it never closes the merge modal
          underneath. "Don't ask again" is offered only when this dialog is the
          whole question: with losing attempts to decide about, there is more
          here than a yes/no worth skipping. */}
      {confirmDelete && (
        <ConfirmDialog
          title="Delete agent?"
          body={`The merged work stays on ${preview?.base ?? "the base branch"}. This clears out the worktree, the "${preview?.branch ?? "agent"}" branch and the run.${
            losers.length > 0
              ? ` "Delete all" clears ${losers.length} losing attempt${losers.length === 1 ? "" : "s"} too.`
              : ""
          }`}
          confirmLabel={losers.length > 0 ? `Delete all ${losers.length + 1}` : "Delete"}
          danger
          busy={deleting}
          progress={cleanup}
          progressLabel="Deleting…"
          altLabel={losers.length > 0 ? "Delete this one" : undefined}
          altDanger
          onAlt={losers.length > 0 ? () => deleteWorkspace(false) : undefined}
          hushLabel={losers.length === 0 ? "Don't ask again" : undefined}
          hushed={hushNext}
          onHush={losers.length === 0 ? setHushNext : undefined}
          onConfirm={() => deleteWorkspace(losers.length > 0)}
          onCancel={() => setConfirmDelete(false)}
        />
      )}
    </div>
  );
}
