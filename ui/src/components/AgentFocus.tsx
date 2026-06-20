import { useEffect, useState } from "react";
import { useRuns } from "../store/runs";
import { stopRun, discardRun } from "../api";
import FocusTerminal from "./FocusTerminal";
import MergeModal from "./MergeModal";
import ConfirmDialog from "./ConfirmDialog";

function badgeClass(a: string) {
  return ["claude", "pi", "hermes"].includes(a) ? `badge ${a}` : "badge";
}

export default function AgentFocus() {
  const { runs, focusedRunId, setFocusedRun, refreshRuns } = useRuns();
  const [showMerge, setShowMerge] = useState(false);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  useEffect(() => {
    setShowMerge(false);
  }, [focusedRunId]);
  const [railOpen, setRailOpen] = useState(true);
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;

  return (
    <div className="focus">
      {railOpen ? (
        <div className="rail">
          <div className="rail-head">
            <span>Agents</span>
            <button className="icon-btn" onClick={() => setRailOpen(false)}>«</button>
          </div>
          {runs.map((r) => (
            <button key={r.id} className={`rail-row ${r.id === focusedRunId ? "on" : ""}`} onClick={() => setFocusedRun(r.id)}>
              <span className={`dot ${r.status.state === "running" ? "running" : "exited"}`} />
              <span className="rail-name">{r.agent}: {r.prompt || r.branch}</span>
            </button>
          ))}
        </div>
      ) : (
        <button className="rail-stub icon-btn" onClick={() => setRailOpen(true)}>»</button>
      )}

      <div className="focus-main">
        {focused ? (
          <>
            <div className="focus-head">
              <span className={badgeClass(focused.agent)}>{focused.agent}</span>
              <code>{focused.branch}</code>
              <span className="spacer" />
              <button className="tile-act" title="Stop agent" onClick={async () => { await stopRun(focused.id); await refreshRuns(); }}>■ Stop</button>
              <button className="tile-act danger" title="Discard agent" onClick={() => setConfirmDiscard(true)}>✕ Discard</button>
              <button onClick={() => setShowMerge(true)}>Approve →</button>
            </div>
            <FocusTerminal key={focused.id} runId={focused.id} />
            {showMerge && <MergeModal taskId={focused.id} onClose={() => setShowMerge(false)} />}
            {confirmDiscard && (
              <ConfirmDialog
                title="Discard agent?"
                body={`Stop "${focused.agent}", remove its worktree, and delete the run. This cannot be undone.`}
                confirmLabel="Discard"
                danger
                onConfirm={async () => {
                  const id = focused.id;
                  setConfirmDiscard(false);
                  await discardRun(id);
                  setFocusedRun(null);
                  await refreshRuns();
                }}
                onCancel={() => setConfirmDiscard(false)}
              />
            )}
          </>
        ) : (
          <div className="board empty">Select an agent from the rail.</div>
        )}
      </div>
    </div>
  );
}
