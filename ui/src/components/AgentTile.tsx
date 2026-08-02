import { useEffect, useState } from "react";
import { RunInfo, runPreview, discardRun } from "../api";
import { useRuns } from "../store/runs";
import { runName } from "../agents";
import { toastError } from "../lib/toast";
import { runStatus } from "../lib/runstate";
import ConfirmDialog from "./ConfirmDialog";
import { TrashIcon } from "./icons";

function badgeClass(agent: string): string {
  if (agent === "claude") return "badge claude";
  if (agent === "pi") return "badge pi";
  if (agent === "hermes") return "badge hermes";
  return "badge";
}

export default function AgentTile({ run }: { run: RunInfo }) {
  const { setFocusedRun, setView, refreshRuns, focusedRunId } = useRuns();
  const [preview, setPreview] = useState("");
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [discarding, setDiscarding] = useState(false);

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
        <ConfirmDialog
          title={isTerminal ? "Close terminal?" : "Discard agent?"}
          body={isTerminal
            ? "Stop the shell and remove this terminal session."
            : run.worktree
              ? `Stop "${run.agent}", remove its worktree, and delete the run. This cannot be undone.`
              : `Stop "${run.agent}" and delete the run. Your checkout and its changes are left exactly as they are.`}
          confirmLabel={isTerminal ? "Close" : "Discard"}
          danger
          busy={discarding}
          onConfirm={async () => {
            setDiscarding(true);
            try {
              await discardRun(run.id);
              if (focusedRunId === run.id) setFocusedRun(null);
              await refreshRuns();
            } catch (e) {
              toastError(e, isTerminal ? "Close failed" : "Discard failed");
            } finally {
              setDiscarding(false);
              setConfirmDiscard(false);
            }
          }}
          onCancel={() => setConfirmDiscard(false)}
        />
      )}
    </div>
  );
}
