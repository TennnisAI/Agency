import { useCallback, useEffect, useState } from "react";
import { CommitFile, HistoryItem, gitCommitFiles } from "../../api";
import { decorate } from "./status";
import DiffViewer from "./DiffViewer";

export default function CommitDetail({ taskId, item }: { taskId: string; item: HistoryItem }) {
  const [files, setFiles] = useState<CommitFile[]>([]);
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try { const f = await gitCommitFiles(taskId, item.hash); setFiles(f); setPath(f[0]?.path ?? null); setError(""); }
    catch (e) { setError(String(e)); }
  }, [taskId, item.hash]);
  useEffect(() => { load(); }, [load]);

  return (
    <div className="git-commitdetail">
      <div className="git-commitdetail-head">
        <div className="git-commitdetail-subject">{item.subject}</div>
        <div className="git-commit-meta">
          <span>{item.author}</span><span className="git-commit-hash">{item.hash.slice(0, 9)}</span>
          <span>{new Date(item.date * 1000).toLocaleString()}</span>
        </div>
      </div>
      {error && <div className="git-error">{error}</div>}
      <div className="git-commitdetail-body">
        <div className="git-commitdetail-files">
          {files.map((f) => {
            const dec = decorate(f.status, " ");
            return (
              <div key={f.path} className={`git-row ${path === f.path ? "sel" : ""}`} onClick={() => setPath(f.path)} title={f.path}>
                <span className="git-letter" style={{ color: `var(${dec.varName})` }}>{dec.letter}</span>
                <span className="git-name">{f.path}</span>
              </div>
            );
          })}
        </div>
        <div className="git-commitdetail-diff">
          {path ? <DiffViewer taskId={taskId} path={path} mode="commit" hash={item.hash} onChanged={() => {}} />
                : <div className="diff-empty">Select a file.</div>}
        </div>
      </div>
    </div>
  );
}
