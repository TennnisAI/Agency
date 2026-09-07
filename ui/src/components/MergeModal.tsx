import { useEffect, useState } from "react";
import {
  CloneProgress,
  MergeOutcome,
  MergePreview,
  MergeState,
  PrInfo,
  RunCleanup,
  RunSessionInfo,
  abortMergeTask,
  archiveRun,
  deleteRunRemoteBranch,
  discardRun,
  finishMergeTask,
  listRunSessions,
  mergePreview,
  mergeStatus,
  mergeTask,
  runCleanup,
  sendMergeConflict,
} from "../api";
import { mergeTidyCopy, removalCopy } from "../lib/runRemoval";
import { loadFocusTab, PRIMARY_TAB } from "../lib/focusTab";
import { agentLabel } from "../agents";
import PillSelect from "./PillSelect";
import PrSection from "./PrSection";
import ConfirmDialog from "./ConfirmDialog";
import RemovalSummary, { BranchProbeNote } from "./RemovalSummary";
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
  // Deep-link handler for "View PR" — opens the PR in the review panel.
  // Required: without it the button falls out of Agency to github.com.
  onReviewPr: (number: number) => void;
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
  const [queued, setQueued] = useState(false);
  // The agent tabs sharing this run's worktree, so the conflict can be handed
  // to the one whose context has something to do with it (AGE-184). This used
  // to go to the run's own agent every time, which after a day's work is
  // routinely the one that has been doing something else entirely.
  const [tabs, setTabs] = useState<RunSessionInfo[]>([]);
  // The tabs have been asked for and answered. The default below has to wait
  // for this: the run itself is in the store already, so without it the
  // default would settle on the run's own agent one render before the tabs
  // that might hold the remembered one arrived.
  const [tabsLoaded, setTabsLoaded] = useState(false);
  // Null until the tabs load, then the tab this run was last looked at on —
  // "the one I was just using" is what the user means by "this agent", and the
  // focus view already remembers it per run.
  const [target, setTarget] = useState<string | null>(null);
  // Whether the merge takes the branch's remote copy with it. Ticked by
  // default, as the PR merge dialog's own delete-branch box is, and offered
  // only for a branch that has actually been published. Before AGE-201 nothing
  // in this window could reach the remote at all: the teardown under it takes
  // the worktree and the local branch, and every locally merged agent branch
  // stayed on the remote for good.
  const [deleteRemote, setDeleteRemote] = useState(true);
  // The branch's PR, probed once by the section below and reported up here.
  // Null both before the probe answers and when there is no PR; only an
  // explicitly open one changes anything above.
  const [pr, setPr] = useState<PrInfo | null>(null);
  // Whether the user has had an opinion about the checkbox yet. An open PR
  // arrives after the modal has already drawn the box ticked, and unticking it
  // under a user who ticked it themselves would be the app overruling them.
  const [remoteTouched, setRemoteTouched] = useState(false);
  // What became of that once the merge landed: the ref that went, or null when
  // the remote turned out not to have the branch any more.
  const [remoteGone, setRemoteGone] = useState<{ ref: string | null } | null>(null);
  const [deletingRemote, setDeletingRemote] = useState(false);
  // Kept apart from the modal's own error line: the merge has already
  // succeeded by the time this can fail, and a refused branch deletion must
  // not read as a failed merge.
  const [remoteError, setRemoteError] = useState("");
  // What git says about the merge after a resolver has had a go at it. Null
  // until the first check; `probeError` is why, when it stays null.
  const [state, setState] = useState<MergeState | null>(null);
  const [probeError, setProbeError] = useState("");
  const [finishing, setFinishing] = useState(false);
  const [aborting, setAborting] = useState(false);
  // Latest step of whichever git operation is in flight (merge, finish, abort).
  // All three run in the project's shared checkout and move it between
  // branches, which on a large repo is seconds of nothing to look at.
  const [gitStep, setGitStep] = useState<CloneProgress | null>(null);
  // What tidying up would remove, re-read once the merge lands: before it the
  // branch is the only copy of the work, after it the branch is a duplicate of
  // commits on the base, and the buttons below have to say the second thing.
  const [plan, setPlan] = useState<RunCleanup | null>(null);
  // The probe failed rather than being slow. Kept apart so the panel says it
  // could not read the branch instead of claiming to still be reading it.
  const [planFailed, setPlanFailed] = useState(false);
  const [archiving, setArchiving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  // Latest teardown step of the archive/delete in flight. Both stop a session
  // and hand git a worktree to unlink, which on a large repo runs long enough
  // that a bare "Deleting…" label reads as a frozen app.
  const [cleanup, setCleanup] = useState<CloneProgress | null>(null);
  // Both post-merge explanations can be switched off once the workflow is
  // habit, and switched back on from Settings ▸ Messages.
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
  // Who "Fix with agent" can hand the conflict to: the same tabs the run's
  // strip draws, in the same order. A shell tab is left out — it is a login
  // shell, not something that reads a prompt.
  const targets: { value: string; label: string }[] = [
    ...(me && !me.primaryClosed ? [{ value: me.id, label: agentLabel(me.agent) }] : []),
    ...tabs
      .filter((t) => t.status.state !== "gone" && t.agent !== "shell")
      .map((t) => ({ value: t.id, label: `${agentLabel(t.agent)} · ${t.id.split("--").pop()}` })),
  ];
  // Default to the tab this run was last open on, which is the app's best
  // record of "the agent I was just working with". Falls back to the leftmost
  // tab when that one is gone (or when the run has never been opened).
  useEffect(() => {
    if (target !== null || !tabsLoaded || targets.length === 0) return;
    const remembered = typeof localStorage !== "undefined"
      ? loadFocusTab(localStorage, taskId)
      : PRIMARY_TAB;
    const wanted = remembered === PRIMARY_TAB ? taskId : remembered;
    setTarget(targets.some((t) => t.value === wanted) ? wanted : targets[0].value);
  }, [target, targets, tabsLoaded, taskId]);
  // Cleanup in flight: archiving or deleting. Both tear the run down, so the
  // modal's cancel affordances stay disabled until they land.
  const busy = archiving || deleting;
  // A git operation is running in the project's shared checkout. Closing the
  // window wouldn't stop it, and leaving mid-merge is how a half-finished merge
  // gets forgotten about, so the exits are shut for the few seconds it takes.
  const gitBusy = merging || finishing || aborting;

  // Escape mirrors the header ✕; disabled while cleaning up or mid-git (the
  // modal's other cancel affordances are disabled then too).
  useModalKeys(onClose, !busy && !gitBusy);

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

  // GitHub closes a pull request whose head branch is deleted; it only marks it
  // merged once the commits reach the base branch on the remote, which a local
  // merge hasn't done yet. So an open PR turns the default off, with the reason
  // beside the box: the branch is better deleted by merging that PR from Source
  // Control, which is what leaves the PR saying "merged".
  useEffect(() => {
    if (pr?.state === "OPEN" && !remoteTouched) setDeleteRemote(false);
  }, [pr, remoteTouched]);

  // The run's agent tabs, loaded once: "Fix with agent" needs to know whether
  // there is a choice to offer before it offers one.
  useEffect(() => {
    let live = true;
    listRunSessions(taskId)
      .then((s) => { if (live) setTabs(s); })
      .catch(() => {})
      .finally(() => { if (live) setTabsLoaded(true); });
    return () => { live = false; };
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
  async function abortAndClose() {
    setError("");
    setAborting(true);
    setGitStep(null);
    try {
      await abortMergeTask(taskId, setGitStep);
      onClose();
    } catch (e) {
      setError(String(e));
      setAborting(false);
      setGitStep(null);
    }
  }

  // The branch's remote copy, deleted after the merge has committed and never
  // before: until the commits are on the base, that copy is the only one off
  // this machine, and the backend refuses to delete it. Best effort by
  // contract — the merge has landed, so a remote that says no is a note under
  // the outcome rather than an error over it.
  async function tidyRemoteBranch() {
    if (!deleteRemote || !preview?.remoteBranch) return;
    setDeletingRemote(true);
    setRemoteError("");
    try {
      setRemoteGone({ ref: await deleteRunRemoteBranch(taskId) });
    } catch (e) {
      setRemoteError(String(e));
    } finally {
      setDeletingRemote(false);
    }
  }

  async function attempt() {
    setError("");
    setMerging(true);
    setGitStep(null);
    // Whatever an earlier check said predates this merge. Clearing it keeps the
    // conflict panel from rendering a stale "nothing in progress" for the frame
    // between the merge landing and the first poll answering.
    setState(null);
    let landed = false;
    try {
      const result = await mergeTask(taskId, setGitStep);
      setOutcome(result);
      landed = result.kind === "clean";
    } catch (e) {
      setError(String(e));
      // Re-read rather than leave the failure as a bare string: if another run
      // is mid-merge, this is what turns it into the explanation above.
      mergeStatus(taskId).then(setState).catch(() => {});
    } finally {
      setMerging(false);
      setGitStep(null);
    }
    if (landed) await tidyRemoteBranch();
  }

  // Hand the conflict to the agent that produced the branch: it already has the
  // context for the change, and it is a session the user can watch and talk to.
  // The backend submits the prompt, so this really does start the agent working
  // rather than leaving a composed message sitting in its prompt box.
  async function fixWithAgent() {
    setError("");
    setSending(true);
    try {
      // False means the agent is mid-turn and the prompt is queued behind it;
      // saying "sent" then would have the user watching for work that has not
      // started yet.
      setQueued(!(await sendMergeConflict(taskId, target ?? undefined)));
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
    setGitStep(null);
    let landed = false;
    try {
      const result = await finishMergeTask(taskId, setGitStep);
      setOutcome(result);
      landed = result.kind === "clean";
    } catch (e) {
      setError(String(e));
      // The failure is usually "still conflicted"; re-read so the file list
      // shown next to the error is the current one.
      mergeStatus(taskId).then(setState).catch(() => {});
    } finally {
      setFinishing(false);
      setGitStep(null);
    }
    // A merge that came through conflicts ends where a clean one does, so it
    // tidies the same way. Skipping it here is how the resolved half of the
    // flow quietly keeps a remote branch the clean half deletes.
    if (landed) await tidyRemoteBranch();
  }

  // Read when the modal opens and again whenever the merge state changes, so
  // the tidy-up wording is about the branch as it is now rather than as it was
  // before the merge.
  useEffect(() => {
    let live = true;
    runCleanup(taskId)
      .then((c) => {
        if (!live) return;
        setPlan(c);
        setPlanFailed(false);
      })
      .catch(() => live && setPlanFailed(true));
    return () => {
      live = false;
    };
  }, [taskId, outcome?.kind, state?.merged]);

  const conflicts = outcome?.kind === "conflicts";
  // The same sentences the tile and rail teardown dialogs use, so the merge
  // window and the ✕ menu describe one action rather than two that sound
  // different. `me` is missing only for the frame between a teardown landing
  // and the run list refreshing, where the fallback is never rendered.
  const removalRun = me ?? {
    kind: "agent" as const,
    agent: "this agent",
    branch: preview?.branch ?? "",
    worktree: true,
  };
  const tidy = mergeTidyCopy(plan);
  const deleteCopy = removalCopy(removalRun, "delete", plan);

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
    // Click-outside-to-close is off while the delete confirm is up. The confirm
    // covers this overlay now, so a click can no longer fall through to it, but
    // the pairing is what made the old stacking bug so expensive: a stray click
    // dismissed the whole approve-and-merge flow and put the user back on the
    // agent they had just asked to delete.
    <div
      className="settings-overlay anchor-top"
      onClick={() => { if (!busy && !gitBusy && !confirmDelete) onClose(); }}
    >
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
              {/* Only for a branch that has a remote copy: everything else has
                  nothing to offer deleting, and an unticked box for a branch
                  that was never pushed is one more thing to read past. */}
              {!nothingToMerge && preview.remoteBranch && (
                <label
                  className="merge-option"
                  title="Deletes the branch on the remote once the merge has landed. The agent's worktree and its local branch stay until you archive or delete the agent."
                >
                  <input
                    type="checkbox"
                    checked={deleteRemote}
                    onChange={(e) => { setRemoteTouched(true); setDeleteRemote(e.target.checked); }}
                  />
                  <span>Delete <code>{preview.remoteBranch}</code> after merging</span>
                </label>
              )}
              {!nothingToMerge && preview.remoteBranch && pr?.state === "OPEN" && (
                <p className="merge-warn">
                  #{pr.number} is open on this branch. Deleting the branch on the remote closes
                  that PR rather than marking it merged, so merging the PR from Source Control is
                  the tidier ending; it deletes the branch for you.
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
                  onPr={setPr}
                  onReviewPr={onReviewPr}
                />
              )}
            </div>
          ) : null
        )}

        {merging && <ProgressReadout progress={gitStep} fallback="Merging…" />}

        {outcome?.kind === "clean" && (
          <div className="merge-done">
            <p className="merge-ok">
              ✓ Merged cleanly into {preview?.base ?? "main"} · <code>{outcome.commit.slice(0, 10)}</code>
            </p>
            {/* What happened to the remote copy, said here rather than in a
                toast: it is part of what this merge did, and the buttons under
                it are about what is left. */}
            {deletingRemote && (
              <p className="merge-note">
                <span className="spinner" /> Deleting <code>{preview?.remoteBranch}</code>…
              </p>
            )}
            {remoteGone && (
              <p className="merge-note">
                {remoteGone.ref
                  ? <>Deleted <code>{remoteGone.ref}</code> from the remote.</>
                  : "The remote copy of the branch was already gone."}
              </p>
            )}
            {remoteError && <p className="removal-warn">{remoteError}</p>}
            {/* The three buttons under this are the whole decision, so the
                copy is those three verbs and nothing else. It used to be the
                archive teardown's own Removes/Keeps lists under a heading,
                "Tidy up", that named no button on screen and explained one
                option of the three (AGE-164).

                Under the three lines is the mechanics — the agent branch a
                post-merge archive takes because it is now a second name for
                commits on the base, which is also why the delete beside it is
                not a red button, and that archiving is reversible. That much
                is hushable. The caveat is not: it is the only line that can
                change which button you press. */}
            <div className="merge-cleanup">
              {/* One line per button, in the buttons' own order. Run together
                  as a single comma-spliced sentence, as this was, the three
                  verbs had to be picked back out of it before any of them
                  could be compared — which is the whole job of this step. */}
              <ul className="merge-choices">
                {tidy.choices.map((c) => (
                  <li key={c.verb}><b>{c.verb}</b> {c.text}.</li>
                ))}
              </ul>
              {!hushNote && (
                <>
                  {tidy.detail && (
                    <p className="merge-note">
                      {tidy.detail}{" "}
                      <button className="hush-link" onClick={() => setHushNote(true)}>Don't show this again</button>
                    </p>
                  )}
                  <BranchProbeNote checking={!plan && !planFailed} probeFailed={planFailed && !plan} />
                </>
              )}
              {tidy.caveat && <p className="removal-warn">{tidy.caveat}</p>}
            </div>
            <div className="git-actions">
              {losers.length > 0 ? (
                <button disabled={busy} onClick={() => archiveWorkspace(true)}>
                  {archiving
                    ? "Cleaning up…"
                    : `Archive + delete ${losers.length} losing attempt${losers.length === 1 ? "" : "s"}`}
                </button>
              ) : (
                <button
                  disabled={busy}
                  title="Stop the agent, remove its worktree, and keep a record of what it did."
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
                // Red only when something could really be lost. After a merge
                // nothing can, and a warning colour that fires anyway is one
                // nobody reads on the day it means something.
                className={`ghost${deleteCopy.danger ? " danger" : ""}`}
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
                    {queued
                      ? `Queued for ${targets.find((t) => t.value === target)?.label ?? "this agent"}, with git's status of the merge. It is part-way through a turn, so the prompt goes in as soon as that finishes. `
                      : `Sent to ${targets.find((t) => t.value === target)?.label ?? "this agent"}, with git's status of the merge. `}
                    Close this window to watch it work; the merge is in the project's checkout, not
                    the agent's worktree, so the prompt points git there. Reopen this window when
                    it's done, or leave it open: it rechecks git every few seconds either way.
                  </p>
                ) : (
                  <p className="merge-note">
                    Nothing has been committed. Hand the conflict to
                    {targets.length > 1 ? " one of this workspace's agents" : " this agent"}, or
                    resolve it yourself in the project's checkout and come back here to finish.
                  </p>
                )}
                <div className="git-actions">
                  {/* Which agent, before the button that sends to it. Only
                      when there is a choice: one agent in the worktree and
                      this is noise; several and picking is the whole point,
                      since the run's own is not usually the one that has been
                      near this branch lately (AGE-184). */}
                  {targets.length > 1 && target && (
                    <PillSelect
                      value={target}
                      options={targets}
                      onChange={(v) => { setTarget(v); setSent(false); }}
                      title="Which agent in this workspace gets the conflict"
                    />
                  )}
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
                    disabled={aborting}
                    onClick={abortAndClose}
                    title="Undo the merge and put the project's checkout back"
                  >
                    {aborting ? "Aborting…" : "Abort merge"}
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
                  <button autoFocus disabled={finishing || aborting} onClick={finish}>
                    {finishing ? "Finishing…" : "Finish merge"}
                  </button>
                  <button className="ghost" disabled={finishing || aborting} onClick={onClose}>Close</button>
                  <button
                    className="ghost"
                    disabled={finishing || aborting}
                    onClick={abortAndClose}
                    title="Undo the merge, and with it the resolution that was just done"
                  >
                    {aborting ? "Aborting…" : "Abort merge"}
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
            {/* Both of these move the shared checkout between branches, so
                they say where they are rather than sitting on a button label. */}
            {(finishing || aborting) && (
              <ProgressReadout
                progress={gitStep}
                fallback={aborting ? "Undoing the merge…" : "Finishing the merge…"}
              />
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
          body={
            <RemovalSummary
              copy={
                // The losing attempts are part of what this button removes, so
                // they belong in the same list rather than in a sentence under
                // it that the eye skips.
                losers.length > 0
                  ? {
                      ...deleteCopy,
                      goes: [
                        ...deleteCopy.goes,
                        `"Delete all" takes ${losers.length} losing attempt${
                          losers.length === 1 ? "" : "s"
                        } with it.`,
                      ],
                    }
                  : deleteCopy
              }
            />
          }
          confirmLabel={losers.length > 0 ? `Delete all ${losers.length + 1}` : "Delete"}
          danger={deleteCopy.danger}
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
