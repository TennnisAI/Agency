import { useEffect, useState } from "react";
import { useRuns } from "../store/runs";
import { discardRun, archiveRun, setRunTitle } from "../api";
import { runName } from "../agents";
import FocusTerminal from "./FocusTerminal";
import RunPanel from "./RunPanel";
import MergeModal from "./MergeModal";
import ConfirmDialog from "./ConfirmDialog";
import Resizer from "./Resizer";
import ArchivedSection from "./ArchivedSection";
import { usePaneWidth } from "../hooks/usePaneWidth";
import AgentAddMenu from "./AgentAddMenu";

function badgeClass(a: string) {
  return ["claude", "pi", "hermes"].includes(a) ? `badge ${a}` : "badge";
}

export default function AgentFocus() {
  const { runs, focusedRunId, setFocusedRun, refreshRuns, createAgent, createTerminal } = useRuns();
  const [showMerge, setShowMerge] = useState(false);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [panel, setPanel] = useState<"agent" | "run">("agent");
  useEffect(() => {
    setShowMerge(false);
    setPanel("agent");
  }, [focusedRunId]);
  const [railOpen, setRailOpen] = useState(true);
  const rail = usePaneWidth("rail", 312, 220, 520);
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;

  return (
    <div className="focus">
      {railOpen ? (
        <>
          <div className="rail" style={{ width: rail.width, minWidth: rail.width }}>
            <div className="rail-head">
              <span>Agents</span>
              <AgentAddMenu variant="icon" onSpawn={createAgent} onTerminal={createTerminal} />
              <span className="spacer" />
              <button className="icon-btn" onClick={() => setRailOpen(false)}>«</button>
            </div>
            {runs.map((r) => (
              <button key={r.id} className={`rail-row ${r.id === focusedRunId ? "on" : ""}`} onClick={() => setFocusedRun(r.id)}>
                <span className={`dot ${r.status.state === "running" ? "running" : "exited"}`} />
                <span className="rail-name">
                  {r.kind === "terminal" ? `≳ ${r.title || "terminal"}` : `${r.agent}: ${runName(r)}`}
                </span>
              </button>
            ))}
            <ArchivedSection />
          </div>
          <Resizer size={rail.width} min={220} max={520} onChange={rail.setWidth} side="left" />
        </>
      ) : (
        <div className="rail-stub">
          <button className="icon-btn" onClick={() => setRailOpen(true)}>»</button>
          <span className="rail-spine">AGENTS · {runs.length}</span>
        </div>
      )}

      <div className="focus-main">
        {focused ? (
          focused.kind === "terminal" ? (
            <>
              <div className="focus-head">
                <span className="badge">terminal</span>
                <span className="spacer" />
                <button className="tile-act danger" title="Close terminal" onClick={() => setConfirmDiscard(true)}>✕ Close</button>
              </div>
              <FocusTerminal key={focused.id} runId={focused.id} />
              {confirmDiscard && (
                <ConfirmDialog
                  title="Close terminal?"
                  body="Stop the shell and remove this terminal session."
                  confirmLabel="Close"
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
            <>
              <div className="focus-head">
                <span className={badgeClass(focused.agent)}>{focused.agent}</span>
                <code>{focused.branch}</code>
                <div className="focus-tabs">
                  <button className={panel === "agent" ? "on" : ""} onClick={() => setPanel("agent")}>Agent</button>
                  <button className={panel === "run" ? "on" : ""} onClick={() => setPanel("run")}>Run</button>
                </div>
                <span className="spacer" />
                <button className="tile-act danger" title="Discard agent" onClick={() => setConfirmDiscard(true)}>✕ Discard</button>
                <button className="tile-act" title="Archive agent" onClick={async () => {
                  const id = focused.id;
                  await archiveRun(id);
                  setFocusedRun(null);
                  await refreshRuns();
                }}>⌂ Archive</button>
                <button onClick={() => setShowMerge(true)}>Approve →</button>
              </div>
              {panel === "agent"
                ? <FocusTerminal key={focused.id} runId={focused.id}
                    onFirstPrompt={focused.title ? undefined : (line) => { setRunTitle(focused.id, line).catch(() => {}); }} />
                : <RunPanel key={`run-${focused.id}`} run={focused} />}
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
          )
        ) : (
          <div className="board empty">Select an agent from the rail.</div>
        )}
      </div>
    </div>
  );
}
