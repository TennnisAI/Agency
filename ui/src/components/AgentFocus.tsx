import { useEffect, useState } from "react";
import { useRuns } from "../store/runs";
import FocusTerminal from "./FocusTerminal";
import MergeModal from "./MergeModal";

function badgeClass(a: string) {
  return ["claude", "pi", "hermes"].includes(a) ? `badge ${a}` : "badge";
}

export default function AgentFocus() {
  const { runs, focusedRunId, setFocusedRun } = useRuns();
  const [showMerge, setShowMerge] = useState(false);
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
              <button onClick={() => setShowMerge(true)}>Approve →</button>
            </div>
            <FocusTerminal key={focused.id} runId={focused.id} />
            {showMerge && <MergeModal taskId={focused.id} onClose={() => setShowMerge(false)} />}
          </>
        ) : (
          <div className="board empty">Select an agent from the rail.</div>
        )}
      </div>
    </div>
  );
}
