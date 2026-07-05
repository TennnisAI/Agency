import { useEffect, useState } from "react";
import { useRuns } from "../store/runs";
import { discardRun, archiveRun, setRunTitle } from "../api";
import { toastError } from "../lib/toast";
import { runName } from "../agents";
import FocusTerminal, { shellStream } from "./FocusTerminal";
import RunPanel from "./RunPanel";
import MergeModal from "./MergeModal";
import ConfirmDialog from "./ConfirmDialog";
import Resizer from "./Resizer";
import ArchivedSection from "./ArchivedSection";
import { usePaneWidth, loadFold, saveFold } from "../hooks/usePaneWidth";
import AgentAddMenu from "./AgentAddMenu";

const SHELL_MIN = 120;
const SHELL_MAX = 640;
const SHELL_FOLD_KEY = "focus-shell-open";

function badgeClass(a: string) {
  return ["claude", "pi", "hermes"].includes(a) ? `badge ${a}` : "badge";
}

// `onSpawn` lets the host view wrap agent creation with its pre-flight checks
// (missing-CLI install offer, repo readiness); without it the rail's add menu
// falls back to the raw store spawn.
export default function AgentFocus({
  onSpawn,
}: {
  onSpawn?: (agentId: string, opts?: { base: string; mergeTarget: string }) => void;
}) {
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
  // Companion terminal (bottom panel) — shared open-state + height across runs.
  const shellPane = usePaneWidth("focus-shell-h", 240, SHELL_MIN, SHELL_MAX);
  const [shellOpen, setShellOpen] = useState<boolean>(() =>
    typeof localStorage === "undefined" ? false : loadFold(localStorage, SHELL_FOLD_KEY, false),
  );
  const toggleShell = () =>
    setShellOpen((o) => {
      const next = !o;
      try {
        if (typeof localStorage !== "undefined") saveFold(localStorage, SHELL_FOLD_KEY, next);
      } catch { /* ignore quota / security errors */ }
      return next;
    });
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;

  return (
    <div className="focus">
      {railOpen ? (
        <>
          <div className="rail" style={{ width: rail.width, minWidth: rail.width }}>
            <div className="rail-head">
              <AgentAddMenu variant="header" onSpawn={onSpawn ?? createAgent} onTerminal={createTerminal} />
              <span className="spacer" />
              <button className="icon-btn" onClick={() => setRailOpen(false)}>«</button>
            </div>
            {runs.map((r) => (
              <button key={r.id} className={`rail-row ${r.id === focusedRunId ? "on" : ""}`} onClick={() => setFocusedRun(r.id)}>
                <span className={`dot ${r.status.state === "running" ? "running" : "exited"}`} />
                <span className="rail-name">
                  {r.kind === "terminal"
                    ? `≳ ${r.title || "terminal"}`
                    : `${r.raceId ? "∥ " : ""}${r.agent}: ${runName(r)}`}
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
                <button className="tile-act danger icon-only" title="Close terminal" onClick={() => setConfirmDiscard(true)}>✕</button>
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
                {panel === "agent" && (
                  <button
                    className={`focus-shell-toggle ${shellOpen ? "on" : ""}`}
                    title="Toggle terminal in this worktree"
                    onClick={toggleShell}
                  >≳ Terminal</button>
                )}
                <span className="spacer" />
                <button className="tile-act danger" title="Discard agent" onClick={() => setConfirmDiscard(true)}>✕ Discard</button>
                <button className="tile-act" title="Archive agent — stops it and removes the worktree; uncommitted work is auto-committed to its branch" onClick={async () => {
                  const id = focused.id;
                  try {
                    await archiveRun(id);
                    setFocusedRun(null);
                    await refreshRuns();
                  } catch (e) {
                    toastError(e, "Archive failed");
                  }
                }}>⌂ Archive</button>
                <button onClick={() => setShowMerge(true)}>Approve →</button>
              </div>
              {panel === "agent" ? (
                <div className="focus-body">
                  <FocusTerminal key={focused.id} runId={focused.id}
                    onFirstPrompt={focused.title ? undefined : (line) => { setRunTitle(focused.id, line).catch(() => {}); }} />
                  {shellOpen && (
                    <>
                      <Resizer orientation="horizontal" side="right"
                        size={shellPane.width} min={SHELL_MIN} max={SHELL_MAX} onChange={shellPane.setWidth} />
                      <div className="focus-shell" style={{ height: shellPane.width, flexShrink: 0 }}>
                        <div className="focus-shell-head">
                          <span className="focus-shell-title">≳ terminal · <code>{focused.branch}</code></span>
                          <span className="spacer" />
                          <button className="icon-btn" title="Hide terminal" onClick={toggleShell}>✕</button>
                        </div>
                        <FocusTerminal key={`shell-${focused.id}`} runId={focused.id} stream={shellStream} />
                      </div>
                    </>
                  )}
                </div>
              ) : (
                <RunPanel key={`run-${focused.id}`} run={focused} />
              )}
              {showMerge && (
                <MergeModal
                  taskId={focused.id}
                  onClose={() => setShowMerge(false)}
                  onArchived={() => {
                    setFocusedRun(null);
                    refreshRuns();
                  }}
                />
              )}
              {confirmDiscard && (
                <ConfirmDialog
                  title="Discard agent?"
                  body={`Stop "${focused.agent}", remove its worktree, and delete the run. This cannot be undone.`}
                  confirmLabel="Discard"
                  danger
                  onConfirm={async () => {
                    const id = focused.id;
                    setConfirmDiscard(false);
                    try {
                      await discardRun(id);
                      setFocusedRun(null);
                      await refreshRuns();
                    } catch (e) {
                      toastError(e, "Discard failed");
                    }
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
