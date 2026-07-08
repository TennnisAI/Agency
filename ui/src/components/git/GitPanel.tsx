import { useCallback, useEffect, useState } from "react";
import { FileChange, BranchInfo, HistoryItem, gitStatus, gitBranchInfo, gitPush, gitFetch, gitPull } from "../../api";
import { toastSuccess } from "../../lib/toast";
import ChangesPanel from "./ChangesPanel";
import HistoryPanel from "./HistoryPanel";
import CommitDetail from "./CommitDetail";
import DiffViewer from "./DiffViewer";
import ReviewComments from "./ReviewComments";
import BranchBar from "./BranchBar";
import GitSections from "./GitSections";
import Resizer from "../Resizer";
import { usePaneWidth } from "../../hooks/usePaneWidth";

export type GitSelection =
  | { kind: "file"; path: string; group: "index" | "workingTree" | "merge" | "untracked" }
  | { kind: "commit"; item: HistoryItem }
  | null;

export default function GitPanel({
  taskId,
  layout,
  selection,
  onSelect,
  width,
  allowComments = true,
}: {
  taskId: string;
  layout: "compact" | "full";
  selection: GitSelection;
  onSelect: (sel: GitSelection) => void;
  width?: number;
  allowComments?: boolean;
}) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [branch, setBranch] = useState<BranchInfo | null>(null);
  // `error` is the transient status-refresh error (re-evaluated every poll).
  // `actionError` is sticky: it survives the follow-up refresh so a failed
  // commit/push/publish stays readable until the next action.
  const [error, setError] = useState("");
  const [actionError, setActionError] = useState("");
  const [commentsKey, setCommentsKey] = useState(0);
  // `busy` disables action buttons + shows spinners while a git op runs.
  // `historyKey` bumps after every action so the commit graph reloads.
  const [busy, setBusy] = useState(false);
  const [historyKey, setHistoryKey] = useState(0);
  const leftPane = usePaneWidth("git-full-left", 360, 300, 720);

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setBranch(await gitBranchInfo(taskId));
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId]);

  useEffect(() => { refresh(); }, [refresh]);
  useEffect(() => {
    const id = setInterval(() => refresh(), 2000);
    return () => clearInterval(id);
  }, [refresh]);

  // `label`, when given, raises a success toast once the op resolves.
  // Resolves true on success so callers can react (e.g. CommitBox only clears
  // the message once the commit actually landed); never rejects.
  const act = useCallback(async (fn: () => Promise<unknown>, label?: string): Promise<boolean> => {
    setActionError("");
    setBusy(true);
    let ok = false;
    try {
      await fn();
      ok = true;
      if (label) toastSuccess(label);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(false);
    }
    await refresh();
    // New/changed commits: reload the history graph (it doesn't poll).
    setHistoryKey((k) => k + 1);
    return ok;
  }, [refresh]);

  const onSelectFile = (path: string, group: "index" | "workingTree" | "merge" | "untracked") =>
    onSelect({ kind: "file", path, group });

  const diffMode = (group: string): "working-unstaged" | "working-staged" =>
    group === "index" ? "working-staged" : "working-unstaged";

  const changesPanel = (
    <ChangesPanel taskId={taskId} changes={changes} branch={branch} onAct={act} busy={busy}
      selectedPath={selection?.kind === "file" ? selection.path : null} onSelectFile={onSelectFile} />
  );
  const historyPanel = (
    <HistoryPanel taskId={taskId} base={branch?.base ?? null} reloadKey={historyKey}
      selectedHash={selection?.kind === "commit" ? selection.item.hash : null}
      onSelectCommit={(item) => onSelect({ kind: "commit", item })} />
  );
  const sections = <GitSections changesPanel={changesPanel} historyPanel={historyPanel} />;

  if (layout === "compact") {
    return (
      <aside className="git-panel compact" style={width ? { width, minWidth: width } : undefined}>
        <BranchBar info={branch} busy={busy}
          onSync={() => act(() => gitPush(taskId), "Pushed")}
          onFetch={() => act(() => gitFetch(taskId), "Fetched")}
          onPull={() => act(() => gitPull(taskId), "Pulled")}
          onRefresh={refresh} />
        {(actionError || error) && <div className="git-error">{actionError || error}</div>}
        {sections}
        {allowComments && <ReviewComments key={commentsKey} taskId={taskId} />}
      </aside>
    );
  }

  return (
    <div className="git-panel full">
      <BranchBar info={branch} busy={busy}
        onSync={() => act(() => gitPush(taskId), "Pushed")}
        onFetch={() => act(() => gitFetch(taskId), "Fetched")}
        onPull={() => act(() => gitPull(taskId), "Pulled")}
        onRefresh={refresh} />
      {(actionError || error) && <div className="git-error">{actionError || error}</div>}
      <div className="git-full-body">
        <div className="git-full-left" style={{ width: leftPane.width }}>
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
    </div>
  );
}
