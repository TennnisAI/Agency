import { useEffect, useState } from "react";
import { RunInfo, runPreview } from "../api";
import { useRuns } from "../store/runs";
import { runName } from "../agents";
import { runStatus } from "../lib/runstate";
import RunRemoveDialog from "./RunRemoveDialog";
import { TrashIcon } from "./icons";

function badgeClass(agent: string): string {
  if (agent === "claude") return "badge claude";
  if (agent === "pi") return "badge pi";
  if (agent === "hermes") return "badge hermes";
  return "badge";
}

export default function AgentTile({ run }: { run: RunInfo }) {
  const { setFocusedRun, setView } = useRuns();
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

  const st = runStatus(run);
  const isTerminal = run.kind === "terminal";
  return (
    <div className="tile" onClick={() => { setFocusedRun(run.id); setView("focus"); }}>
      <div className="tile-head">
        <span className={`dot ${st.cls}`} />
        <span className="tile-title">{runName(run)}</span>
        {/* A run script is live in this workspace — a dev server, a build. The
            Run tab says which; here it is only "something is running". */}
        {run.runScriptsLive && (
          <span className="run-dot" title="A run script is running in this workspace" />
        )}
        {run.raceId && <span className="badge race" title="Racing: same prompt, parallel attempts">∥</span>}
        <span className={isTerminal ? "badge" : badgeClass(run.agent)}>{isTerminal ? "terminal" : run.agent}</span>
      </div>
      {!isTerminal && (
        <div className="tile-meta">
          <code>{run.branch}</code>
          {/* Without a worktree the stat is uncommitted work in the checkout,
              not a branch's diff against its base. */}
          <span className="diffstat" title={run.worktree ? "Changes on this branch" : "Uncommitted changes in the checkout"}>
            <span className="add">+{run.added}</span> <span className="del">−{run.deleted}</span> · {run.files}f
          </span>
          {!run.worktree && <span className="badge" title="Works in the project checkout, not an isolated worktree">in checkout</span>}
        </div>
      )}
      <pre className="tile-preview">{preview}</pre>
      <div className="tile-foot">
        <span title={st.title}>{st.text}</span>
        <button className="tile-act danger" title={isTerminal ? "Close terminal" : "Discard agent"} onClick={(e) => { e.stopPropagation(); setConfirmDiscard(true); }}><TrashIcon /></button>
      </div>
      {confirmDiscard && (
        <RunRemoveDialog run={run} action="discard" onClose={() => setConfirmDiscard(false)} />
      )}
    </div>
  );
}
