import { useCallback, useEffect, useState } from "react";
import {
  CommitInfo,
  FileChange,
  gitCommit,
  gitDiff,
  gitLog,
  gitPush,
  gitStage,
  gitStatus,
  gitUnstage,
} from "../api";
import MergeModal from "./MergeModal";

export default function GitPanel({ taskId }: { taskId: string }) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [commits, setCommits] = useState<CommitInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState("");
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [showMerge, setShowMerge] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setCommits(await gitLog(taskId));
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, [taskId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function showDiff(file: FileChange) {
    setSelected(file.path);
    try {
      const staged = file.index !== " " && file.index !== "?";
      setDiff(await gitDiff(taskId, file.path, staged));
    } catch (e) {
      setError(String(e));
      setDiff("");
    }
  }

  async function act(fn: () => Promise<unknown>) {
    try {
      await fn();
      setError("");
    } catch (e) {
      setError(String(e));
    }
    await refresh();
  }

  const staged = changes.filter((c) => c.index !== " " && c.index !== "?");
  const unstaged = changes.filter((c) => c.index === " " || c.index === "?");

  return (
    <div className="git-panel">
      <div className="git-header">
        <h3>Changes</h3>
        <button onClick={refresh}>Refresh</button>
      </div>
      {error && <div className="git-error">{error}</div>}

      <div className="git-section">
        <h4>Staged</h4>
        {staged.length === 0 && <div className="git-empty">none</div>}
        {staged.map((c) => (
          <div key={c.path} className="git-file">
            <span className="git-status">{c.index}</span>
            <span className="git-path" onClick={() => showDiff(c)}>
              {c.path}
            </span>
            <button onClick={() => act(() => gitUnstage(taskId, c.path))}>−</button>
          </div>
        ))}
      </div>

      <div className="git-section">
        <h4>Unstaged</h4>
        {unstaged.length === 0 && <div className="git-empty">none</div>}
        {unstaged.map((c) => (
          <div key={c.path} className="git-file">
            <span className="git-status">{c.index === "?" ? "?" : c.worktree}</span>
            <span className="git-path" onClick={() => showDiff(c)}>
              {c.path}
            </span>
            <button onClick={() => act(() => gitStage(taskId, c.path))}>+</button>
          </div>
        ))}
      </div>

      <div className="git-commit">
        <textarea
          placeholder="Commit message"
          value={message}
          onChange={(e) => setMessage(e.target.value)}
        />
        <div className="git-actions">
          <button
            onClick={() =>
              act(async () => {
                await gitCommit(taskId, message);
                setMessage("");
              })
            }
          >
            Commit
          </button>
          <button onClick={() => act(() => gitPush(taskId))}>Push</button>
          <button onClick={() => setShowMerge(true)}>Approve &amp; merge</button>
        </div>
      </div>

      {selected && (
        <div className="git-diff">
          <h4>{selected}</h4>
          <pre>{diff || "(no diff)"}</pre>
        </div>
      )}

      <div className="git-section">
        <h4>Recent commits</h4>
        {commits.map((c) => (
          <div key={c.hash} className="git-commit-row">
            <code>{c.hash.slice(0, 7)}</code> <span>{c.summary}</span>
          </div>
        ))}
      </div>

      {showMerge && <MergeModal taskId={taskId} onClose={() => setShowMerge(false)} />}
    </div>
  );
}
