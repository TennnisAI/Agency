import { useCallback, useEffect, useState } from "react";
import { FileDiff, gitParseDiff, gitStageHunk, gitUnstageHunk } from "../api";

function lineClass(line: string): string {
  if (line.startsWith("+")) return "diff-add";
  if (line.startsWith("-")) return "diff-del";
  return "diff-ctx";
}

export default function DiffView({
  taskId,
  path,
  staged,
  onChanged,
}: {
  taskId: string;
  path: string;
  staged: boolean;
  onChanged: () => void;
}) {
  const [fd, setFd] = useState<FileDiff | null>(null);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try {
      setFd(await gitParseDiff(taskId, path, staged));
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, [taskId, path, staged]);

  useEffect(() => {
    load();
  }, [load]);

  async function act(i: number) {
    try {
      if (staged) await gitUnstageHunk(taskId, path, i);
      else await gitStageHunk(taskId, path, i);
      await load();
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  }

  if (error) return <div className="git-error">{error}</div>;
  if (!fd) return <div className="diff-empty">loading…</div>;
  if (fd.hunks.length === 0) return <div className="diff-empty">no textual changes</div>;

  return (
    <div className="diffview">
      {fd.hunks.map((h, i) => (
        <div className="hunk" key={i}>
          <div className="hunk-head">
            <code className="diff-hunk">{h.header}</code>
            <button onClick={() => act(i)}>{staged ? "Unstage hunk" : "Stage hunk"}</button>
          </div>
          <pre className="hunk-body">
            {h.lines.map((l, j) => (
              <div className={lineClass(l)} key={j}>
                {l || " "}
              </div>
            ))}
          </pre>
        </div>
      ))}
    </div>
  );
}
