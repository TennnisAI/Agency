import { useCallback, useEffect, useState } from "react";
import { Checkpoint, CommitFile, checkpointFiles } from "../../api";
import ChangedFiles from "./ChangedFiles";
import DiffViewer from "./DiffViewer";
import CheckpointRestoreDialog from "./CheckpointRestoreDialog";
import { checkpointLabel } from "./checkpoints";

/**
 * One checkpoint: what changed since the one before it (for a "turn ended"
 * checkpoint, what the agent did that turn), and the way back to it.
 */
export default function CheckpointDetail({ runId, cp, prev, checkout, onAct, onRevealInFiles }: {
  runId: string;
  cp: Checkpoint;
  prev: Checkpoint | null;
  checkout: boolean;
  onAct: (fn: () => Promise<unknown>, label?: string) => Promise<boolean>;
  onRevealInFiles?: (path: string) => void;
}) {
  const [files, setFiles] = useState<CommitFile[] | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [restoring, setRestoring] = useState(false);

  const load = useCallback(async () => {
    if (!prev) { setFiles([]); setPath(null); return; }
    try {
      const f = await checkpointFiles(runId, prev.commit, cp.commit);
      setFiles(f); setPath(f[0]?.path ?? null); setError("");
    } catch (e) { setError(String(e)); }
  }, [runId, prev, cp.commit]);
  useEffect(() => { load(); }, [load]);

  const summary = !prev
    ? "The earliest checkpoint this run has, so there is nothing before it to compare with."
    : files == null
      ? ""
      : `${files.length === 1 ? "1 file" : `${files.length} files`} changed since checkpoint ${prev.seq} (${checkpointLabel(prev.kind).toLowerCase()}).`;

  return (
    <div className="git-commitdetail">
      <div className="git-commitdetail-head git-checkpoint-head">
        <div className="git-checkpoint-head-text">
          <div className="git-commitdetail-subject">
            {checkpointLabel(cp.kind)} <span className="git-checkpoint-seq">checkpoint {cp.seq}</span>
          </div>
          <div className="git-commit-meta">
            <span>{new Date(cp.at * 1000).toLocaleString()}</span>
            <span>{summary}</span>
          </div>
        </div>
        <button className="btn-secondary" onClick={() => setRestoring(true)}
          title="Put the workspace's files back the way they were at this checkpoint">
          Restore files…
        </button>
      </div>
      {error && <div className="git-error">{error}</div>}
      {prev && files && files.length > 0 && (
        <div className="git-commitdetail-body">
          <ChangedFiles files={files} selected={path} onSelect={setPath} />
          <div className="git-commitdetail-diff">
            {path ? (
              <DiffViewer key={path} taskId={runId} path={path} mode="checkpoint" from={prev.commit}
                hash={cp.commit} onChanged={() => {}} allowComments={false}
                onRevealInFiles={onRevealInFiles} />
            ) : <div className="diff-empty">Select a file.</div>}
          </div>
        </div>
      )}
      {prev && files && files.length === 0 && (
        <div className="diff-empty">No files changed between these two checkpoints.</div>
      )}
      {restoring && (
        <CheckpointRestoreDialog runId={runId} cp={cp} checkout={checkout} onAct={onAct}
          onClose={() => setRestoring(false)} />
      )}
    </div>
  );
}
