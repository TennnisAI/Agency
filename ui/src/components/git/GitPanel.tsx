import { useCallback, useEffect, useRef, useState } from "react";
import { FileChange, BranchInfo, HistoryItem, gitStatus, gitBranchInfo, gitPush } from "../../api";
import ChangesPanel from "./ChangesPanel";
import HistoryPanel from "./HistoryPanel";
import CommitDetail from "./CommitDetail";
import DiffViewer from "./DiffViewer";
import BranchBar from "./BranchBar";

type Selection =
  | { kind: "file"; path: string; group: "index" | "workingTree" | "merge" | "untracked" }
  | { kind: "commit"; item: HistoryItem }
  | null;

export default function GitPanel({ taskId, layout }: { taskId: string; layout: "compact" | "full" }) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [branch, setBranch] = useState<BranchInfo | null>(null);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<"changes" | "history">("changes");
  const [sel, setSel] = useState<Selection>(null);
  const visible = useRef(true);

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setBranch(await gitBranchInfo(taskId));
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId]);

  useEffect(() => { refresh(); }, [refresh]);
  useEffect(() => {
    const id = setInterval(() => { if (visible.current) refresh(); }, 2000);
    return () => clearInterval(id);
  }, [refresh]);

  const act = useCallback((fn: () => Promise<unknown>) => {
    (async () => {
      try { await fn(); setError(""); } catch (e) { setError(String(e)); }
      await refresh();
    })();
  }, [refresh]);

  const onSelectFile = (path: string, group: "index" | "workingTree" | "merge" | "untracked") =>
    setSel({ kind: "file", path, group });

  const diffMode = (group: string): "working-unstaged" | "working-staged" =>
    group === "index" ? "working-staged" : "working-unstaged";

  const changesPanel = (
    <ChangesPanel taskId={taskId} changes={changes} branch={branch} onAct={act}
      selectedPath={sel?.kind === "file" ? sel.path : null} onSelectFile={onSelectFile} />
  );
  const historyPanel = (
    <HistoryPanel taskId={taskId} base={branch?.base ?? null}
      selectedHash={sel?.kind === "commit" ? sel.item.hash : null}
      onSelectCommit={(item) => setSel({ kind: "commit", item })} />
  );

  if (layout === "compact") {
    return (
      <aside className="git-panel compact">
        {error && <div className="git-error">{error}</div>}
        <div className="git-tabs">
          <button className={tab === "changes" ? "on" : ""} onClick={() => setTab("changes")}>Changes</button>
          <button className={tab === "history" ? "on" : ""} onClick={() => setTab("history")}>History</button>
        </div>
        {tab === "changes" ? changesPanel : historyPanel}
        {sel?.kind === "file" && (
          <div className="git-compact-diff">
            <DiffViewer taskId={taskId} path={sel.path} mode={diffMode(sel.group)} onChanged={refresh} />
          </div>
        )}
        {sel?.kind === "commit" && <div className="git-compact-diff"><CommitDetail taskId={taskId} item={sel.item} /></div>}
      </aside>
    );
  }

  return (
    <div className="git-panel full">
      <BranchBar info={branch} onSync={() => act(() => gitPush(taskId))} onRefresh={refresh} />
      {error && <div className="git-error">{error}</div>}
      <div className="git-full-body">
        <div className="git-full-left">
          <div className="git-tabs">
            <button className={tab === "changes" ? "on" : ""} onClick={() => setTab("changes")}>Changes</button>
            <button className={tab === "history" ? "on" : ""} onClick={() => setTab("history")}>History</button>
          </div>
          {tab === "changes" ? changesPanel : historyPanel}
        </div>
        <div className="git-full-right">
          {sel?.kind === "file" && <DiffViewer taskId={taskId} path={sel.path} mode={diffMode(sel.group)} onChanged={refresh} />}
          {sel?.kind === "commit" && <CommitDetail taskId={taskId} item={sel.item} />}
          {!sel && <div className="diff-empty">Select a file or commit.</div>}
        </div>
      </div>
    </div>
  );
}
