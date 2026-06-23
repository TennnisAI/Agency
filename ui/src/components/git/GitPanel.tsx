import { useCallback, useEffect, useState } from "react";
import { FileChange, BranchInfo, HistoryItem, gitStatus, gitBranchInfo, gitPush } from "../../api";
import ChangesPanel from "./ChangesPanel";
import HistoryPanel from "./HistoryPanel";
import CommitDetail from "./CommitDetail";
import DiffViewer from "./DiffViewer";
import ReviewComments from "./ReviewComments";
import BranchBar from "./BranchBar";
import GitSections from "./GitSections";

export type GitSelection =
  | { kind: "file"; path: string; group: "index" | "workingTree" | "merge" | "untracked" }
  | { kind: "commit"; item: HistoryItem }
  | null;

export default function GitPanel({
  taskId,
  layout,
  selection,
  onSelect,
}: {
  taskId: string;
  layout: "compact" | "full";
  selection: GitSelection;
  onSelect: (sel: GitSelection) => void;
}) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [branch, setBranch] = useState<BranchInfo | null>(null);
  const [error, setError] = useState("");
  const [commentsKey, setCommentsKey] = useState(0);

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
      try { await fn(); setError(""); } catch (e) { setError(String(e)); }
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
      <aside className="git-panel compact">
        {error && <div className="git-error">{error}</div>}
        {sections}
        <ReviewComments key={commentsKey} taskId={taskId} />
      </aside>
    );
  }

  return (
    <div className="git-panel full">
      <BranchBar info={branch} onSync={() => act(() => gitPush(taskId))} onRefresh={refresh} />
      {error && <div className="git-error">{error}</div>}
      <div className="git-full-body">
        <div className="git-full-left">
          {sections}
          <ReviewComments key={commentsKey} taskId={taskId} />
        </div>
        <div className="git-full-right">
          {selection?.kind === "file" && <DiffViewer taskId={taskId} path={selection.path} mode={diffMode(selection.group)} onChanged={refresh} onCommentAdded={() => setCommentsKey((k) => k + 1)} />}
          {selection?.kind === "commit" && <CommitDetail taskId={taskId} item={selection.item} />}
          {!selection && <div className="diff-empty">Select a file or commit.</div>}
        </div>
      </div>
    </div>
  );
}
