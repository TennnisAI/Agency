import { useCallback, useEffect, useState } from "react";
import { FileChange, gitCommit, gitPush, gitStage, gitStatus, gitUnstage } from "../api";

function isStaged(c: FileChange): boolean {
  return c.index !== " " && c.index !== "?";
}

export default function GitReviewPanel({
  taskId,
  onOpenSource,
}: {
  taskId: string;
  onOpenSource: () => void;
}) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, [taskId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function act(fn: () => Promise<unknown>) {
    try {
      await fn();
      setError("");
    } catch (e) {
      setError(String(e));
    }
    await refresh();
  }

  const staged = changes.filter(isStaged);
  const unstaged = changes.filter((c) => !isStaged(c));

  return (
    <aside className="review-panel">
      <div className="review-head">
        <h3>Review</h3>
        <button className="ghost" onClick={onOpenSource}>Open in Source Control →</button>
      </div>
      {error && <div className="git-error">{error}</div>}
      <div className="sc-commit">
        <textarea placeholder="Commit message" value={message} onChange={(e) => setMessage(e.target.value)} />
        <div className="row-actions">
          <button onClick={() => act(async () => { await gitCommit(taskId, message); setMessage(""); })}>Commit</button>
          <button className="ghost" onClick={() => act(() => gitPush(taskId))}>Push</button>
        </div>
      </div>
      <h4>Staged</h4>
      {staged.length === 0 && <div className="git-empty">none</div>}
      {staged.map((c) => (
        <div key={c.path} className="sc-file">
          <span className="git-status">{c.index}</span>
          <span className="git-path">{c.path}</span>
          <button onClick={() => act(() => gitUnstage(taskId, c.path))}>−</button>
        </div>
      ))}
      <h4>Changes</h4>
      {unstaged.length === 0 && <div className="git-empty">none</div>}
      {unstaged.map((c) => (
        <div key={c.path} className="sc-file">
          <span className="git-status">{c.index === "?" ? "?" : c.worktree}</span>
          <span className="git-path">{c.path}</span>
          <button onClick={() => act(() => gitStage(taskId, c.path))}>+</button>
        </div>
      ))}
    </aside>
  );
}
