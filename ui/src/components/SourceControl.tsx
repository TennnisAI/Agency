import { useCallback, useEffect, useState } from "react";
import {
  CommitInfo,
  FileChange,
  Project,
  gitCommit,
  gitPush,
  gitStage,
  gitStatus,
  gitUnstage,
  listProjects,
  projectLog,
} from "../api";
import DiffView from "./DiffView";

function isStaged(c: FileChange): boolean {
  return c.index !== " " && c.index !== "?";
}

export default function SourceControl({ taskId }: { taskId: string }) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [selected, setSelected] = useState<{ path: string; staged: boolean } | null>(null);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [projects, setProjects] = useState<Project[]>([]);
  const [histProject, setHistProject] = useState<string | null>(null);
  const [commits, setCommits] = useState<CommitInfo[]>([]);

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
    listProjects().then(setProjects);
  }, [refresh]);

  useEffect(() => {
    if (histProject) projectLog(histProject, 50).then(setCommits).catch(() => setCommits([]));
  }, [histProject]);

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
    <div className="source-control">
      <div className="sc-left">
        <div className="sc-commit">
          <textarea
            placeholder="Commit message"
            value={message}
            onChange={(e) => setMessage(e.target.value)}
          />
          <div className="row-actions">
            <button onClick={() => act(async () => { await gitCommit(taskId, message); setMessage(""); })}>
              Commit
            </button>
            <button className="ghost" onClick={() => act(() => gitPush(taskId))}>Push</button>
          </div>
        </div>
        {error && <div className="git-error">{error}</div>}

        <h4>Staged</h4>
        {staged.length === 0 && <div className="git-empty">none</div>}
        {staged.map((c) => (
          <div key={c.path} className="sc-file">
            <span className="git-status">{c.index}</span>
            <span className="git-path" onClick={() => setSelected({ path: c.path, staged: true })}>{c.path}</span>
            <button onClick={() => act(() => gitUnstage(taskId, c.path))}>−</button>
          </div>
        ))}

        <h4>Changes</h4>
        {unstaged.length === 0 && <div className="git-empty">none</div>}
        {unstaged.map((c) => (
          <div key={c.path} className="sc-file">
            <span className="git-status">{c.index === "?" ? "?" : c.worktree}</span>
            <span className="git-path" onClick={() => setSelected({ path: c.path, staged: false })}>{c.path}</span>
            <button onClick={() => act(() => gitStage(taskId, c.path))}>+</button>
          </div>
        ))}

        <h4>History</h4>
        <div className="hist-pills">
          {projects.map((p) => (
            <button
              key={p.id}
              className={`pill ${histProject === p.id ? "on" : ""}`}
              title={p.name}
              onClick={() => setHistProject(p.id)}
            >
              {p.name.slice(0, 1).toUpperCase()}
            </button>
          ))}
        </div>
        {commits.map((c) => (
          <div key={c.hash} className="hist-row">
            <code>{c.hash.slice(0, 7)}</code> <span>{c.summary}</span>
          </div>
        ))}
      </div>

      <div className="sc-right">
        {selected ? (
          <DiffView taskId={taskId} path={selected.path} staged={selected.staged} onChanged={refresh} />
        ) : (
          <div className="diff-empty">Select a file to view its diff.</div>
        )}
      </div>
    </div>
  );
}
