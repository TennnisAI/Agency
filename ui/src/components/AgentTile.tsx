import { useEffect, useState } from "react";
import { RunInfo, runPreview, discardRun } from "../api";
import { useRuns } from "../store/runs";
import { runName } from "../agents";
import ConfirmDialog from "./ConfirmDialog";

function badgeClass(agent: string): string {
  if (agent === "claude") return "badge claude";
  if (agent === "pi") return "badge pi";
  if (agent === "hermes") return "badge hermes";
  return "badge";
}
function statusLabel(s: RunInfo["status"]): { cls: string; text: string } {
  if (s.state === "running") return { cls: "running", text: "running" };
  if (s.state === "exited") return { cls: "exited", text: `exited (${s.code})` };
  return { cls: "exited", text: "gone" };
}

export default function AgentTile({ run }: { run: RunInfo }) {
  const { setFocusedRun, setView, refreshRuns, focusedRunId } = useRuns();
  const [preview, setPreview] = useState("");
  const [confirmDiscard, setConfirmDiscard] = useState(false);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const p = await runPreview(run.id, 12);
        if (alive) setPreview(p);
      } catch {
        /* ignore */
      }
    };
    tick();
    const t = window.setInterval(tick, 1500);
    return () => { alive = false; window.clearInterval(t); };
  }, [run.id]);

  const st = statusLabel(run.status);
  const isTerminal = run.kind === "terminal";
  return (
    <div className="tile" onClick={() => { setFocusedRun(run.id); setView("focus"); }}>
      <div className="tile-head">
        <span className={`dot ${st.cls}`} />
        <span className="tile-title">{runName(run)}</span>
        {run.raceId && <span className="badge race" title="Racing: same prompt, parallel attempts">⚡</span>}
        <span className={isTerminal ? "badge" : badgeClass(run.agent)}>{isTerminal ? "terminal" : run.agent}</span>
      </div>
      {!isTerminal && (
        <div className="tile-meta">
          <code>{run.branch}</code>
          <span className="diffstat"><span className="add">+{run.added}</span> <span className="del">−{run.deleted}</span> · {run.files}f</span>
        </div>
      )}
      <pre className="tile-preview">{preview}</pre>
      <div className="tile-foot">
        {st.text}
        <button className="tile-act danger" title={isTerminal ? "Close terminal" : "Discard agent"} onClick={(e) => { e.stopPropagation(); setConfirmDiscard(true); }}>✕</button>
      </div>
      {confirmDiscard && (
        <ConfirmDialog
          title={isTerminal ? "Close terminal?" : "Discard agent?"}
          body={isTerminal ? "Stop the shell and remove this terminal session." : `Stop "${run.agent}", remove its worktree, and delete the run. This cannot be undone.`}
          confirmLabel={isTerminal ? "Close" : "Discard"}
          danger
          onConfirm={async () => {
            await discardRun(run.id);
            if (focusedRunId === run.id) setFocusedRun(null);
            setConfirmDiscard(false);
            await refreshRuns();
          }}
          onCancel={() => setConfirmDiscard(false)}
        />
      )}
    </div>
  );
}
