import { useCallback, useEffect, useState } from "react";
import { FileChange, BranchInfo, HistoryItem, gitStatus, gitBranchInfo, gitPush } from "../../api";
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

  const act = useCallback((fn: () => Promise<unknown>) => {
    (async () => {
      setActionError("");
      try { await fn(); } catch (e) { setActionError(String(e)); }
      await refresh();
    })();
  }, [refresh]);

  const onSelectFile = (path: string, group: "index" | "workingTree" | "merge" | "untracked") =>
    onSelect({ kind: "file", path, group });

  const diffMode = (group: string): "working-unstaged" | "working-staged" =>
    group === "index" ? "working-staged" : "working-unstaged";

  const changesPanel = (
    <ChangesPanel taskId={taskId} changes={changes} branch={branch} onAct={act}
      selectedPath={selection?.kind === "file" ? selection.path : null} onSelectFile={onSelectFile} />
  );
  const historyPanel = (
    <HistoryPanel taskId={taskId} base={branch?.base ?? null}
      selectedHash={selection?.kind === "commit" ? selection.item.hash : null}
      onSelectCommit={(item) => onSelect({ kind: "commit", item })} />
  );
  const sections = <GitSections changesPanel={changesPanel} historyPanel={historyPanel} />;

  if (layout === "compact") {
    return (
      <aside className="git-panel compact" style={width ? { width, minWidth: width } : undefined}>
        <BranchBar info={branch} onSync={() => act(() => gitPush(taskId))} onRefresh={refresh} />
        {(actionError || error) && <div className="git-error">{actionError || error}</div>}
        {sections}
        {allowComments && <ReviewComments key={commentsKey} taskId={taskId} />}
      </aside>
    );
  }

  return (
    <div className="git-panel full">
      <BranchBar info={branch} onSync={() => act(() => gitPush(taskId))} onRefresh={refresh} />
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
