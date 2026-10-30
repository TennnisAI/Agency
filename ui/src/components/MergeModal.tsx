import { useEffect, useRef, useState } from "react";
import {
  CloneProgress,
  MergeMethod,
  MergeMethods,
  MergeOutcome,
  MergePreview,
  MergeState,
  PrInfo,
  PrMergeResult,
  RunCleanup,
  abortMergeTask,
  archiveRun,
  deleteRunRemoteBranch,
  discardRun,
  finishMergeTask,
  gitPush,
  mergePreview,
  mergeRunPr,
  mergeStatus,
  mergeTask,
  prMergeMethods,
  runCleanup,
} from "../api";
import { mergeTidyCopy, removalCopy } from "../lib/runRemoval";
import { endMergeFlow, handoffNote, mergeFlow, patchMergeFlow, reconcileFlow } from "./git/mergeFlow";
import { useMergeHandoff } from "./git/useMergeHandoff";
import PillSelect from "./PillSelect";
import { prMergeError } from "../lib/prMergeError";
import PrSection from "./PrSection";
import ConfirmDialog from "./ConfirmDialog";
import RemovalSummary, { BranchProbeNote } from "./RemovalSummary";
import ProgressReadout from "./ProgressReadout";
import { useRuns } from "../store/runs";
import { useModalKeys } from "../hooks/useModalKeys";
import { useBackdropDismiss } from "../hooks/useBackdropDismiss";
import { useHushed } from "../lib/hushed";

// Remote-tracking refs as a readable list: `origin/agent/x`, or
// `fork/agent/x and origin/agent/x` for a checkout that publishes to both.
// Every line here that mentions the remote copy names all of them, because
// deleting them is all-or-nothing and a line that named one would be a
// promise about the other.
function refList(refs: string[]) {
  return refs.map((r, i) => (
    <span key={r}>
      {i > 0 && (i === refs.length - 1 ? " and " : ", ")}
      <code>{r}</code>
    </span>
  ));
}

const METHOD_LABEL: Record<MergeMethod, string> = {
  merge: "Merge commit",
  squash: "Squash",
  rebase: "Rebase",
};

// The merge method last used on this project's PRs, so a repo that squashes
// does not have to be told so on every approve. Per viewer and best effort: a
// storage that throws just means the first allowed method.
const methodKey = (projectId: string) => `agency.prMergeMethod.${projectId}`;
function loadMethod(projectId: string): string | null {
  try {
    return localStorage.getItem(methodKey(projectId));
  } catch {
    return null;
  }
}
function saveMethod(projectId: string, method: MergeMethod) {
  try {
    localStorage.setItem(methodKey(projectId), method);
  } catch {
    /* only a preference */
  }
}

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
  // Status is still out on a window reopened over a merge this app knows about.
  // The review step has to wait for it: the section it mounts runs the PR probe
  // again, and the answer kept from before the merge is the better one.
  const [resuming, setResuming] = useState(() => !!mergeFlow(taskId));
  const hand = useMergeHandoff(taskId);
  // Whether the merge takes the branch's remote copy with it. Ticked by
  // default, as GitHub's own delete-branch box is, and offered only for a
  // branch that has actually been published. Before AGE-201 nothing in this
  // window could reach the remote at all: the teardown under it takes the
  // worktree and the local branch, and every locally merged agent branch
  // stayed on the remote for good. With an open PR it is the PR merge's
  // delete-branch option instead, which is the same promise kept by GitHub.
  const [deleteRemote, setDeleteRemote] = useState(() => mergeFlow(taskId)?.deleteRemote ?? true);
  // The branch's PR, probed once by the section below and reported up here.
  // Null both before the probe answers and when there is no PR; only an
  // explicitly open one changes anything above.
  const [pr, setPr] = useState<PrInfo | null>(() => mergeFlow(taskId)?.pr ?? null);
  // Read when the local merge lands. The section reporting `pr` unmounts as
  // the merge starts, but a probe it already had out can still answer after
  // that, and the `tidyRemoteBranch` that runs then is the one from the render
  // the button was clicked on.
  const prRef = useRef(pr);
  prRef.current = pr;
  // With an open PR, approving merges that PR on GitHub. The window used to
  // merge locally and then delete the remote branch, and GitHub closes a PR
  // whose head branch disappears instead of marking it merged, so the tidy
  // option was the one that left every agent PR reading as abandoned.
  const [methods, setMethods] = useState<MergeMethods | null>(null);
  const [methodsError, setMethodsError] = useState(false);
  const [method, setMethod] = useState<MergeMethod>("merge");
  const [prMerged, setPrMerged] = useState<PrMergeResult | null>(null);
  // The push ahead of a PR merge, for the progress line: commits the agent
  // made since the last push are not in the PR until they are on GitHub.
  const [pushing, setPushing] = useState(false);
  // Whether that probe is still out. It is two IPC round-trips, the second of
  // which shells out to `gh`, and until it lands `pr` is null and reads as "no
  // PR". See `prUnknown` below for what waits on it.
  const [prProbing, setPrProbing] = useState(() => !mergeFlow(taskId)?.prSettled);
  const prProbingRef = useRef(prProbing);
  prProbingRef.current = prProbing;
  // The probe has been out long enough that the merge stops waiting for it.
  // `gh` has no timeout of its own, so a stalled network can leave it out for
  // as long as it likes, and holding a purely local merge behind a GitHub API
  // request that may never answer is not a trade this window gets to make.
  // Past the cap the merge goes ahead and the remote copy is left alone, which
  // is the safe half of an answer we never got.
  const [prProbeSlow, setPrProbeSlow] = useState(false);
  // Timed from the preview landing, not from the window opening: the section
  // that runs the probe is inside the preview, so before then the clock would
  // be running on a probe that has not started.
  useEffect(() => {
    if (!prProbing || !preview) return;
    const t = window.setTimeout(() => setPrProbeSlow(true), 5000);
    return () => window.clearTimeout(t);
  }, [prProbing, preview]);
  // Set when a local merge landed with the box still ticked but a PR on the
  // branch either unknown or open, so the panel can say the branch is still
  // out there and why.
  const [remoteSkipped, setRemoteSkipped] = useState(false);
  // What became of that once the merge landed: the refs that went, empty when
  // the remotes turned out not to have the branch any more.
  const [remoteGone, setRemoteGone] = useState<{ refs: string[] } | null>(null);
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
  // Cleanup in flight: archiving or deleting. Both tear the run down, so the
  // modal's cancel affordances stay disabled until they land.
  const busy = archiving || deleting;
  // A git operation is running in the project's shared checkout. Closing the
  // window wouldn't stop it, and leaving mid-merge is how a half-finished merge
  // gets forgotten about, so the exits are shut for the few seconds it takes.
  //
  // The remote deletion counts, though it runs after the merge rather than as
  // part of it. It is the one step here that waits on a network, and its
  // result is the only place a refusal ("origin/agent/x has commits that
  // aren't on main") is ever said. Tearing the run down while `git push
  // --delete` is out drops that message on an unmounted modal, leaving the
  // branch out there with unmerged work on it and nothing on screen about it.
  const gitBusy = merging || finishing || aborting || deletingRemote;

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
    //
    // A merge the agent has already committed resumes too, or it reads as
    // "nothing to merge" with the checkout still left on the base branch and no
    // way to finish. Nothing but the flow can tell that from a branch with no
    // commits of its own, which also reports as merged.
    mergeStatus(taskId)
      .then((s) => {
        setState(s);
        const flow = mergeFlow(taskId);
        const step = reconcileFlow(flow, s);
        if (step === "start") patchMergeFlow(taskId, {});
        if (step === "end") endMergeFlow(taskId);
        if (s.merging || (flow && s.merged)) setOutcome({ kind: "conflicts", files: s.unresolved });
      })
      .catch(() => {})
      .finally(() => setResuming(false));
  }, [taskId]);

  // The merge methods the repo allows, asked once an open PR turns up. The
  // remembered one wins when the repo still allows it; otherwise a plain merge
  // commit, then whatever is left, which is GitHub's own order.
  const prOpen = !!projectId && pr?.state === "OPEN";
  useEffect(() => {
    if (!prOpen || !projectId) return;
    let live = true;
    prMergeMethods(projectId)
      .then((m) => {
        if (!live) return;
        setMethods(m);
        const saved = loadMethod(projectId);
        const ok = (k: string | null): k is MergeMethod =>
          k === "merge" || k === "squash" || k === "rebase" ? m[k] : false;
        setMethod(ok(saved) ? saved : m.merge ? "merge" : m.squash ? "squash" : "rebase");
      })
      .catch((e) => {
        if (!live) return;
        setMethodsError(true);
        setError(String(e));
      });
    return () => { live = false; };
  }, [prOpen, projectId]);

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
      endMergeFlow(taskId);
      onClose();
    } catch (e) {
      setError(String(e));
      setAborting(false);
      setGitStep(null);
    }
  }

  // The branch's remote copies, deleted after the merge has committed and never
  // before: until the commits are on the base, those copies are the only ones
  // off this machine, and the backend refuses to delete them. Best effort by
  // contract — the merge has landed, so a remote that says no is a note under
  // the outcome rather than an error over it.
  async function tidyRemoteBranch() {
    if (!deleteRemote || !preview?.remoteBranches.length) return;
    // The PR probe never answered, and the merge stopped waiting for it (see
    // `prProbeSlow`), or it answered late with an open PR. GitHub closes a PR
    // whose head branch is deleted rather than marking it merged, so a branch
    // whose PR is unknown or open is one this does not touch.
    if (projectId && (prProbingRef.current || prRef.current?.state === "OPEN")) {
      setRemoteSkipped(true);
      return;
    }
    setDeletingRemote(true);
    setRemoteError("");
    try {
      setRemoteGone({ refs: await deleteRunRemoteBranch(taskId) });
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
      // git writes MERGE_HEAD during a clean merge too, for as long as commit-msg
      // runs, so the panel's poll can catch one and start an entry for it. Left
      // behind, that entry read as "Ready to finish" on a merge that was done.
      if (landed) endMergeFlow(taskId);
      // This window closes on the hand-off, and the end of the merge still needs
      // what was decided here. `prRef` over `pr`: see where they are declared.
      if (result.kind === "conflicts") {
        patchMergeFlow(taskId, {
          deleteRemote,
          prSettled: !prProbingRef.current,
          pr: prRef.current,
          handoff: null,
        });
      }
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

  // Merge the branch's open PR on GitHub. Anything the agent committed since
  // the last push goes up first, since a PR merge takes what GitHub has and
  // nothing else; the backend then pins the merge to the branch's local tip,
  // so a push that GitHub has not caught up with fails rather than merging
  // the PR short.
  async function mergeViaPr() {
    if (!preview || !projectId) return;
    setError("");
    setMerging(true);
    setGitStep(null);
    saveMethod(projectId, method);
    try {
      // Read again rather than trusted from when the window opened: an agent
      // still at work can commit while the window sits there, and a commit
      // the preview never counted is one that is not pushed, so the pinned
      // merge would refuse it as a mismatch.
      const fresh = await mergePreview(taskId);
      setPreview(fresh);
      if (fresh.commitsUnpushed > 0) {
        setPushing(true);
        await gitPush(taskId, setGitStep);
        setPushing(false);
        setGitStep(null);
      }
      setPrMerged(await mergeRunPr(taskId, method, deleteRemote));
    } catch (e) {
      setError(prMergeError(e, pr?.baseRefName || preview.base));
      // A push that landed before the merge was refused changes what is left
      // to push, and the button's label with it.
      mergePreview(taskId).then(setPreview).catch(() => {});
    } finally {
      setMerging(false);
      setPushing(false);
      setGitStep(null);
    }
  }

  // Hand the conflict to one of the workspace's agents, then get out of the way:
  // this window has nothing left to do until the conflict is settled, and the
  // agent's tab with the panel beside it says everything it did (AGE-273). A
  // refused hand-off keeps it open, with the reason on screen.
  async function fixWithAgent() {
    setError("");
    if (await hand.send()) onClose();
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
      if (landed) endMergeFlow(taskId);
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
  }, [taskId, outcome?.kind, state?.merged, prMerged]);

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
  // The panel's Finish merge opens this window to land the merge here, where the
  // cleanup choices that follow already are. Waits for the preview, which says
  // whether there is a remote copy to tidy. Taken once at open, so a merge that
  // has gone back to conflicted by then is just shown as it is.
  const autoFinish = useRef(false);
  useEffect(() => {
    if (!mergeFlow(taskId)?.finishOnOpen) return;
    autoFinish.current = true;
    patchMergeFlow(taskId, { finishOnOpen: false });
  }, [taskId]);
  useEffect(() => {
    if (!autoFinish.current || !preview || !state) return;
    autoFinish.current = false;
    if (canFinish) void finish();
  }, [preview, state, canFinish]); // eslint-disable-line react-hooks/exhaustive-deps
  const step = outcome?.kind === "clean" || prMerged
    ? "done"
    : conflicts
      ? "resolve"
      : merging
        ? "merge"
        : "review";
  const nothingToMerge = !!preview && preview.commitsAhead === 0;
  // The PR is the merge: GitHub does it, so there is no local conflict to
  // resolve and no shared checkout to wait on.
  const prMode = prOpen && !nothingToMerge;
  const prBase = pr?.baseRefName || preview?.base || "main";
  const unpushed = preview?.commitsUnpushed ?? 0;
  const allowed: MergeMethod[] = methods
    ? (["merge", "squash", "rebase"] as MergeMethod[]).filter((k) => methods[k])
    : [];
  const steps: { key: string; label: string }[] = [
    { key: "review", label: "Review" },
    { key: "merge", label: "Merge" },
    ...(prMode || prMerged ? [] : [{ key: "resolve", label: "Resolve" }]),
    { key: "done", label: "Done" },
  ];
  // Another run left a merge unfinished in the shared project checkout. Nothing
  // here can proceed until that one is finished or aborted, from its own window.
  const blockedBy = state?.blockedBy ?? null;
  // The PR probe hasn't answered for a branch that has a remote copy, so it
  // could have a PR. The answer decides which merge this window offers: a PR
  // is merged on GitHub, anything else locally. The probe is two IPC
  // round-trips and the button below has `autoFocus`, so Enter beats it
  // comfortably; worse, the section that runs it unmounts the moment merging
  // starts, so a probe that hasn't landed by then never lands. A branch never
  // pushed, or a project with nothing to ask, cannot have a PR, and waits for
  // nothing.
  const prUnknown =
    !!projectId &&
    prProbing &&
    !prProbeSlow &&
    !!preview &&
    preview.remoteBranches.length > 0 &&
    !nothingToMerge;
  const mergeBtn = useRef<HTMLButtonElement>(null);
  // `autoFocus` does nothing to a button that is disabled when it mounts, which
  // is the normal case above. Take the focus when the wait ends so Enter still
  // merges: driving this window from the keyboard is the point of the autofocus.
  useEffect(() => {
    if (!prUnknown) mergeBtn.current?.focus();
  }, [prUnknown]);

  const backdrop = useBackdropDismiss(() => { if (!busy && !gitBusy && !confirmDelete) onClose(); });
  return (
    // Click-outside-to-close is off while the delete confirm is up. The confirm
    // covers this overlay now, so a click can no longer fall through to it, but
    // the pairing is what made the old stacking bug so expensive: a stray click
    // dismissed the whole approve-and-merge flow and put the user back on the
    // agent they had just asked to delete.
    <div
      className="settings-overlay anchor-top"
      {...backdrop}
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
        {(error || hand.error) && <div className="git-error">{error || hand.error}</div>}

        {/* Review: explain what will happen before merging. */}
        {!outcome && !merging && (
          resuming || (!preview && !error) ? (
            <p>Checking…</p>
          ) : preview ? (
            <div className="merge-review">
              <p className="merge-summary">
                {prMode
                  ? <>Merge pull request #{pr.number} → <code>{prBase}</code></>
                  : <>Merge <code>{preview.branch}</code> → <code>{preview.base}</code></>}
              </p>
              {nothingToMerge && preview.alreadyOnBase ? (
                <p className="merge-warn">
                  Already on <code>{preview.base}</code>: every commit on this branch landed there, by
                  cherry-pick, rebase or squash merge. Merging it again adds nothing.
                </p>
              ) : nothingToMerge ? (
                <p className="merge-warn">
                  Nothing to merge: this agent has no committed changes on top of <code>{preview.base}</code>.
                </p>
              ) : (
                <p className="merge-note">
                  {preview.commitsAhead} commit{preview.commitsAhead === 1 ? "" : "s"} ahead of <code>{preview.base}</code>.
                  {/* A PR merge takes only what GitHub has, so anything newer
                      goes up first, and the button says so too. */}
                  {prMode && unpushed > 0 &&
                    ` ${unpushed} ${unpushed === 1 ? "isn't" : "aren't"} on GitHub yet, so ${
                      unpushed === 1 ? "it's" : "they're"
                    } pushed first.`}
                </p>
              )}
              {!prMode && !nothingToMerge && preview.commitsBehind > 0 && (
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
              {!prMode && blockedBy && (
                <p className="merge-warn">
                  <code>{blockedBy}</code> is part-way through a merge in the project's checkout, and every agent
                  merges there. Finish or abort that one from its own Approve window first; this branch is untouched
                  in the meantime.
                </p>
              )}
              {/* The count above is against the run's own merge target, and
                  the PR merges into its own base. Said when the two differ,
                  so the summary and the count are not about different
                  branches without a word. */}
              {prMode && pr.baseRefName && pr.baseRefName !== preview.base && (
                <p className="merge-warn">
                  #{pr.number} merges into <code>{pr.baseRefName}</code>, not <code>{preview.base}</code>,
                  which this agent was set to merge into.
                </p>
              )}
              {prMode && pr.isDraft && (
                <p className="merge-warn">
                  #{pr.number} is a draft. Mark it ready for review on GitHub, then merge it here.
                </p>
              )}
              {prMode && methods && allowed.length === 0 && (
                <p className="merge-warn">This repository has no merge methods enabled for pull requests.</p>
              )}
              {/* Only for a branch that has a remote copy: everything else has
                  nothing to offer deleting, and an unticked box for a branch
                  that was never pushed is one more thing to read past. Held
                  back while the PR probe is out, since its answer changes
                  which promise the box makes. */}
              {!nothingToMerge && !prUnknown && preview.remoteBranches.length > 0 && (
                <label
                  className="merge-option"
                  title={
                    prMode
                      ? "GitHub deletes the branch once the pull request is merged. The agent's worktree and its local branch stay until you archive or delete the agent."
                      // The second sentence is the honest version of what the
                      // backend checks: it proves the commits are on the local
                      // base, which is not the same as proving they are
                      // anywhere else. A merge from this window is unpushed
                      // when the deletion runs, so for that moment this
                      // checkout is the only copy of the work.
                      : "Deletes the branch on the remotes it was pushed to, once the merge has landed. The merge itself is local until you push it, so until then the merged commits live only in this checkout. The agent's worktree and its local branch stay until you archive or delete the agent."
                  }
                >
                  <input
                    type="checkbox"
                    checked={deleteRemote}
                    onChange={(e) => setDeleteRemote(e.target.checked)}
                  />
                  {prMode
                    ? <span>Delete the branch on GitHub</span>
                    : <span>Delete {refList(preview.remoteBranches)} after merging</span>}
                </label>
              )}
              {!nothingToMerge && (
                <div className="git-actions">
                  {/* How GitHub merges it, beside the button that asks it to.
                      Only a choice when the repo allows more than one. */}
                  {prMode && allowed.length > 1 && (
                    <PillSelect
                      value={method}
                      options={allowed.map((k) => ({ value: k, label: METHOD_LABEL[k] }))}
                      onChange={setMethod}
                      title="How GitHub merges the pull request"
                    />
                  )}
                  {prMode ? (
                    <button ref={mergeBtn} autoFocus onClick={mergeViaPr}
                      disabled={pr.isDraft || (!methods && !methodsError) || (!!methods && allowed.length === 0)}>
                      {unpushed > 0 ? "Push and merge" : "Merge pull request"}
                    </button>
                  ) : (
                    <button ref={mergeBtn} autoFocus onClick={attempt}
                      disabled={!!blockedBy || prUnknown}
                      title={
                        blockedBy
                          ? `Blocked: ${blockedBy} is mid-merge`
                          : prUnknown
                            ? "Checking whether this branch has an open pull request, which decides how it is merged."
                            : undefined
                      }>
                      {prUnknown ? "Checking GitHub…" : `Merge into ${preview.base}`}
                    </button>
                  )}
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
                  onProbing={setPrProbing}
                  onReviewPr={onReviewPr}
                />
              )}
            </div>
          ) : null
        )}

        {merging && (
          <ProgressReadout
            progress={gitStep}
            fallback={pushing ? "Pushing…" : prMode ? "Merging on GitHub…" : "Merging…"}
          />
        )}

        {(outcome?.kind === "clean" || prMerged) && (
          <div className="merge-done">
            {prMerged ? (
              <>
                <p className="merge-ok">
                  ✓ Merged #{pr?.number} into {prBase}
                </p>
                {/* GitHub did this merge, so the project's checkout has not
                    seen it, where a local merge would have moved it. */}
                <p className="merge-note">
                  Your local <code>{prBase}</code> doesn't have it yet. Pull <code>{prBase}</code> to bring it in.
                </p>
                {prMerged.branchDeleted && (
                  <p className="merge-note">Deleted <code>{prMerged.branchDeleted}</code> on GitHub.</p>
                )}
                {/* The merge landed; only the branch cleanup didn't. A note,
                    not an error, so it cannot read as a failed merge. */}
                {prMerged.warning && <p className="removal-warn">{prMerged.warning}</p>}
              </>
            ) : outcome?.kind === "clean" && (
              <p className="merge-ok">
                ✓ Merged cleanly into {preview?.base ?? "main"} · <code>{outcome.commit.slice(0, 10)}</code>
              </p>
            )}
            {/* What happened to the remote copy, said here rather than in a
                toast: it is part of what this merge did, and the buttons under
                it are about what is left. */}
            {deletingRemote && (
              <p className="merge-note">
                <span className="spinner" /> Deleting {refList(preview?.remoteBranches ?? [])}…
              </p>
            )}
            {remoteGone && (
              <p className="merge-note">
                {remoteGone.refs.length > 0
                  ? <>Deleted {refList(remoteGone.refs)} from
                      {remoteGone.refs.length > 1 ? " their remotes." : " the remote."}</>
                  : `The remote cop${(preview?.remoteBranches.length ?? 0) > 1 ? "ies" : "y"} of the branch ${
                      (preview?.remoteBranches.length ?? 0) > 1 ? "were" : "was"
                    } already gone.`}
              </p>
            )}
            {remoteSkipped && (
              <p className="removal-warn">
                Kept {refList(preview?.remoteBranches ?? [])} on the remote. This branch may have an
                open pull request, and deleting the branch under one closes it rather than marking
                it merged. Delete it from Source Control once you've looked.
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
            {/* `gitBusy` alongside `busy` here for the remote deletion, which
                is still out when this panel first draws. Every button below
                either tears the run down or closes the window, and doing
                either mid-deletion drops the result, refusal and all. */}
            <div className="git-actions">
              {losers.length > 0 ? (
                <button className="btn-primary" disabled={busy || gitBusy} onClick={() => archiveWorkspace(true)}>
                  {archiving
                    ? "Cleaning up…"
                    : `Archive + delete ${losers.length} losing attempt${losers.length === 1 ? "" : "s"}`}
                </button>
              ) : (
                <button
                  className="btn-primary"
                  disabled={busy || gitBusy}
                  title="Stop the agent, remove its worktree, and keep a record of what it did."
                  onClick={() => archiveWorkspace(false)}
                >
                  {archiving ? "Archiving…" : "Archive agent"}
                </button>
              )}
              {losers.length > 0 && (
                <button className="ghost" disabled={busy || gitBusy} onClick={() => archiveWorkspace(false)}>
                  Archive, keep losers
                </button>
              )}
              <button
                // Red only when something could really be lost. After a merge
                // nothing can, and a warning colour that fires anyway is one
                // nobody reads on the day it means something.
                className={`ghost${deleteCopy.danger ? " danger" : ""}`}
                disabled={busy || gitBusy}
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
              <button className="ghost" disabled={busy || gitBusy} onClick={onClose} title="Leave the agent as it is">
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
                <p className="merge-note">
                  {hand.handoff
                    ? `${handoffNote(hand.handoff)} You can follow it in Source Control, which rechecks git every few seconds.`
                    : "Nothing has been committed. Hand the conflict to one of this workspace's agents, or to a fresh one started for the job. Or resolve it yourself: pick a file under Source Control, choose a side on each conflict, and finish there."}
                </p>
                <div className="git-actions">
                  {/* Which agent, before the button that sends to it. Always
                      offered: the run's own is not usually the one that has
                      been near this branch lately (AGE-184), and even in a
                      workspace with one agent, starting a fresh one is a real
                      answer when that one is busy (AGE-199). */}
                  {hand.target && (
                    <PillSelect
                      value={hand.target}
                      options={hand.targets}
                      onChange={hand.setTarget}
                      title="Which agent in this workspace gets the conflict"
                    />
                  )}
                  <button autoFocus disabled={hand.sending || !hand.target} onClick={fixWithAgent}>
                    {hand.sending ? hand.sendingLabel : hand.sent ? "Send again" : hand.sendLabel}
                  </button>
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
