import { useCallback, useEffect, useRef, useState } from "react";
import {
  FileChange, BranchInfo, HistoryItem, StashEntry, CloneProgress,
  gitStatus, gitBranchInfo, gitStashList, gitUndoLastCommit, gitPush, gitSync, gitPullRebase,
} from "../../api";
import { toastSuccess } from "../../lib/toast";
import ConfirmDialog from "../ConfirmDialog";
import ChangesPanel from "./ChangesPanel";
import HistoryPanel from "./HistoryPanel";
import CommitDetail from "./CommitDetail";
import DiffViewer from "./DiffViewer";
import ReviewComments from "./ReviewComments";
import BranchBar from "./BranchBar";
import GitSections from "./GitSections";
import GitOutputModal from "./GitOutputModal";
import Resizer from "../Resizer";
import { usePaneWidth } from "../../hooks/usePaneWidth";
import { useGitOp, setGitOp } from "./ops";

export type GitSelection =
  | { kind: "file"; path: string; group: "index" | "workingTree" | "merge" | "untracked" }
  | { kind: "commit"; item: HistoryItem }
  | null;

type Props = {
  taskId: string;
  layout: "compact" | "full";
  selection: GitSelection;
  onSelect: (sel: GitSelection) => void;
  width?: number;
  allowComments?: boolean;
};

// Source control is per repo, but the panel sits at a fixed spot in the tree —
// switching projects (or focusing another agent) only swaps `taskId`, so React
// would otherwise keep the old repo's file list, errors and dialogs on screen
// until the next poll. Keying on the repo remounts instead, and the in-flight
// op lives in the per-repo store (./ops) so a running push isn't lost with it.
export default function GitPanel(props: Props) {
  return <GitRepoPanel key={props.taskId} {...props} />;
}

function GitRepoPanel({
  taskId,
  layout,
  selection,
  onSelect,
  width,
  allowComments = true,
}: Props) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [branch, setBranch] = useState<BranchInfo | null>(null);
  const [stashes, setStashes] = useState<StashEntry[]>([]);
  // `error` is the transient status-refresh error (re-evaluated every poll).
  // The op's `error` is sticky: it survives the follow-up refresh so a failed
  // commit/push/publish stays readable until dismissed or the next action.
  const [error, setError] = useState("");
  const [commentsKey, setCommentsKey] = useState(0);
  // When set, the full git output (a long push error) is shown in a modal.
  const [outputText, setOutputText] = useState<string | null>(null);
  // The running op, from the per-repo store: `busy` disables action buttons and
  // shows the progress bar, `progress` carries streamed push phase/percent.
  const { busy, progress: pushProgress, error: actionError } = useGitOp(taskId);
  const setPushProgress = useCallback(
    (p: CloneProgress | null) => setGitOp(taskId, { progress: p }),
    [taskId],
  );
  // `historyKey` bumps after every action so the commit graph reloads.
  const [historyKey, setHistoryKey] = useState(0);
  // When a Sync finds the branch diverged from upstream, prompt the user to
  // rebase-and-sync rather than silently merging or failing with a raw error.
  const [divergedPrompt, setDivergedPrompt] = useState(false);
  // After Undo Last Commit, the undone message is restored into the commit box.
  const [restoreMessage, setRestoreMessage] = useState<{ text: string; nonce: number } | null>(null);
  const leftPane = usePaneWidth("git-full-left", 360, 300, 720);

  const refresh = useCallback(async () => {
    try {
      // Fetch in parallel: three sequential IPC round-trips needlessly tripled
      // the per-poll latency, and the status payload is the slow one to deserialize.
      const [st, br, sh] = await Promise.all([
        gitStatus(taskId), gitBranchInfo(taskId), gitStashList(taskId),
      ]);
      setChanges(st);
      setBranch(br);
      setStashes(sh);
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId]);

  useEffect(() => { refresh(); }, [refresh]);
  // Poll for changes, but back off on huge changesets: re-deserializing a
  // multi-megabyte status payload across IPC every 2s re-hitches the main
  // thread, so once there are thousands of changes we poll far less often.
  const changeCount = useRef(0);
  changeCount.current = changes.length;
  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const tick = async () => {
      await refresh();
      if (stopped) return;
      const delay = changeCount.current > 2000 ? 10000 : 2000;
      timer = setTimeout(tick, delay);
    };
    timer = setTimeout(tick, 2000);
    return () => { stopped = true; clearTimeout(timer); };
  }, [refresh]);

  // `label`, when given, raises a success toast once the op resolves.
  // Resolves true on success so callers can react (e.g. CommitBox only clears
  // the message once the commit actually landed); never rejects.
  const act = useCallback(async (fn: () => Promise<unknown>, label?: string): Promise<boolean> => {
    setGitOp(taskId, { busy: true, error: "" });
    let ok = false;
    try {
      await fn();
      ok = true;
      if (label) toastSuccess(label);
    } catch (e) {
      setGitOp(taskId, { error: String(e) });
    } finally {
      setGitOp(taskId, { busy: false });
    }
    await refresh();
    // New/changed commits: reload the history graph (it doesn't poll).
    setHistoryKey((k) => k + 1);
    return ok;
  }, [refresh, taskId]);

  // Push streams `--progress` into `pushProgress` so a big upload shows a
  // determinate bar instead of freezing (the command is async, off the main
  // thread). Cleared when the push settles, success or failure.
  const push = useCallback(async () => {
    setPushProgress({ phase: "Starting push…", percent: null, detail: "" });
    const ok = await act(() => gitPush(taskId, setPushProgress), "Pushed");
    setPushProgress(null);
    return ok;
  }, [act, taskId, setPushProgress]);

  // Sync = pull (fast-forward) + push, like VS Code's "Sync Changes". A diverged
  // branch can't fast-forward, so the backend reports it and we ask the user
  // whether to rebase rather than merging silently or dying on a raw error.
  const sync = useCallback(async () => {
    setPushProgress({ phase: "Syncing…", percent: null, detail: "" });
    let diverged = false;
    // No label: the toast depends on the outcome, raised below.
    const ok = await act(async () => {
      diverged = (await gitSync(taskId, setPushProgress)) === "diverged";
    });
    setPushProgress(null);
    if (!ok) return;
    if (diverged) setDivergedPrompt(true);
    else toastSuccess("Synced");
  }, [act, taskId, setPushProgress]);

  // Chosen from the diverged prompt: replay local commits onto upstream, then push.
  const rebaseAndSync = useCallback(async () => {
    setDivergedPrompt(false);
    setPushProgress({ phase: "Rebasing…", percent: null, detail: "" });
    await act(async () => {
      await gitPullRebase(taskId);
      await gitPush(taskId, setPushProgress);
    }, "Synced (rebased)");
    setPushProgress(null);
  }, [act, taskId, setPushProgress]);

  const undoCommit = () => act(async () => {
    const message = await gitUndoLastCommit(taskId);
    // nonce: restoring the same message twice must still re-trigger the effect.
    setRestoreMessage((p) => ({ text: message, nonce: (p?.nonce ?? 0) + 1 }));
  }, "Last commit undone; changes kept staged");

  const onSelectFile = (path: string, group: "index" | "workingTree" | "merge" | "untracked") =>
    onSelect({ kind: "file", path, group });

  const diffMode = (group: string): "working-unstaged" | "working-staged" =>
    group === "index" ? "working-staged" : "working-unstaged";

  const branchBar = (
    <BranchBar taskId={taskId} info={branch} busy={busy} onAct={act}
      onPush={push} onRefresh={refresh} onUndoCommit={undoCommit} />
  );
  // While a push streams progress, show a determinate bar with phase/percent
  // (like the clone dialog). For any other op, fall back to the VS Code-style
  // thin indeterminate bar; an invisible placeholder otherwise so nothing jumps.
  const progress = pushProgress ? (
    <div className="git-push-progress" role="status" aria-live="polite">
      <div className="git-push-progress-head">
        <span className="git-push-progress-phase">{pushProgress.phase}</span>
        {pushProgress.percent != null && (
          <span className="git-push-progress-pct">{pushProgress.percent}%</span>
        )}
      </div>
      <div className="clone-progress-track">
        <div
          className={`clone-progress-bar${pushProgress.percent == null ? " indeterminate" : ""}`}
          style={pushProgress.percent != null ? { width: `${pushProgress.percent}%` } : undefined}
        />
      </div>
      {pushProgress.detail && <div className="clone-progress-detail">{pushProgress.detail}</div>}
    </div>
  ) : (
    <div className={`git-progress ${busy ? "on" : ""}`}>{busy && <div className="git-progress-bar" />}</div>
  );
  // Keep the inline banner to a single summary line; a multi-line/long error
  // (a rejected push, etc.) is one click away in a scrollable output view so it
  // can't overflow and break the panel layout. Prefer the line that actually
  // names the failure over git's generic "failed to push" wrapper.
  const fullError = actionError || error;
  const errorLines = fullError.split("\n").map((l) => l.trim()).filter(Boolean);
  const errorSummary =
    errorLines.find((l) => /rejected|error:|fatal:|denied|permission|forbidden|403|could not|remote:/i.test(l)) ??
    errorLines.find((l) => !/failed:?$/i.test(l)) ??
    errorLines[0] ?? fullError;
  const errorHasDetail = fullError.trim() !== errorSummary;
  const errorBanner = fullError && (
    <div className="git-error" role="alert">
      <span className="git-error-glyph">!</span>
      <span className="git-error-text">{errorSummary}</span>
      {errorHasDetail && (
        <button className="git-iconbtn git-error-more" title="View full output"
          onClick={() => setOutputText(fullError)}>Output</button>
      )}
      <button className="git-iconbtn" title="Dismiss" aria-label="Dismiss error"
        onClick={() => { setGitOp(taskId, { error: "" }); setError(""); }}>✕</button>
    </div>
  );
  const outputModal = outputText != null && (
    <GitOutputModal title="Git output" text={outputText} onClose={() => setOutputText(null)} />
  );

  const changesPanel = (
    <ChangesPanel taskId={taskId} changes={changes} branch={branch} stashes={stashes}
      restoreMessage={restoreMessage} onAct={act} onSync={sync} busy={busy}
      selectedPath={selection?.kind === "file" ? selection.path : null} onSelectFile={onSelectFile} />
  );
  const divergedDialog = divergedPrompt && (
    <ConfirmDialog
      title="Branch has diverged"
      body={`Your branch and ${branch?.upstream ?? "its upstream"} each have commits the other doesn't, so Sync can't fast-forward. Rebase your local commits on top of the remote and push?`}
      confirmLabel="Rebase & Sync"
      onConfirm={rebaseAndSync}
      onCancel={() => setDivergedPrompt(false)}
    />
  );
  const historyPanel = (
    <HistoryPanel taskId={taskId} base={branch?.base ?? null} reloadKey={historyKey} onAct={act}
      selectedHash={selection?.kind === "commit" ? selection.item.hash : null}
      onSelectCommit={(item) => onSelect({ kind: "commit", item })} />
  );
  const sections = <GitSections changesPanel={changesPanel} historyPanel={historyPanel} />;

  if (layout === "compact") {
    return (
      <aside className="git-panel compact" style={width ? { width, minWidth: width } : undefined}>
        {branchBar}
        {progress}
        {errorBanner}
        {sections}
        {allowComments && <ReviewComments key={commentsKey} taskId={taskId} />}
        {outputModal}
        {divergedDialog}
      </aside>
    );
  }

  return (
    <div className="git-panel full">
      {/* Branch bar + progress + error live inside the Changes (left) pane so
          they span only that column, not the whole app — the diff pane keeps its
          full height and the "Pull Requests" sub-tab isn't shoved down by them. */}
      <div className="git-full-body">
        <div className="git-full-left" style={{ width: leftPane.width }}>
          {branchBar}
          {progress}
          {errorBanner}
          {sections}
          {allowComments && <ReviewComments key={commentsKey} taskId={taskId} />}
        </div>
        <Resizer size={leftPane.width} min={300} max={720} onChange={leftPane.setWidth} />
        <div className="git-full-right">
          {selection?.kind === "file" && <DiffViewer taskId={taskId} path={selection.path} mode={diffMode(selection.group)} onChanged={refresh} onCommentAdded={() => setCommentsKey((k) => k + 1)} allowComments={allowComments} />}
          {selection?.kind === "commit" && <CommitDetail taskId={taskId} item={selection.item} />}
          {!selection && <div className="diff-empty">Select a file or commit.</div>}
        </div>
      </div>
      {outputModal}
      {divergedDialog}
    </div>
  );
}
