import { useEffect, useRef, useState } from "react";
import { useRuns } from "../store/runs";
import {
  discardRun, archiveRun, setRunTitle, renameRun,
  listProfiles, AgentProfile,
  listRunSessions, startRunSession, closeRunSession, RunSessionInfo,
  RunInfo, stopLoop,
} from "../api";
import { toastError } from "../lib/toast";
import { runStatus } from "../lib/runstate";
import { runName, agentLabel } from "../agents";
import FocusTerminal, { shellStream } from "./FocusTerminal";
import RunPanel from "./RunPanel";
import MergeModal from "./MergeModal";
import ConfirmDialog from "./ConfirmDialog";
import PromptDialog from "./PromptDialog";
import Resizer from "./Resizer";
import ArchivedSection from "./ArchivedSection";
import { usePaneWidth, loadFold, saveFold } from "../hooks/usePaneWidth";
import AgentAddMenu from "./AgentAddMenu";
import { TrashIcon, InboxIcon, TerminalIcon, PencilIcon } from "./icons";

const SHELL_MIN = 120;
const SHELL_MAX = 640;
const SHELL_FOLD_KEY = "focus-shell-open";

function badgeClass(a: string) {
  return ["claude", "pi", "hermes"].includes(a) ? `badge ${a}` : "badge";
}

// Live progress of a looping run: attempt counter, phase, and a stop control
// while active; the outcome once terminal. State arrives with the run via the
// store's poll, so this renders fresh data without its own polling.
function LoopStrip({ run, onChanged }: { run: RunInfo; onChanged: () => void }) {
  const cfg = run.loopConfig;
  const st = run.loopState;
  if (!cfg || !st) return null;
  const active = st.status === "awaitingAgent" || st.status === "checking";
  const text =
    st.status === "awaitingAgent" ? `attempt ${st.attempt}/${cfg.maxAttempts} · running`
    : st.status === "checking" ? `attempt ${st.attempt}/${cfg.maxAttempts} · checking`
    : st.status === "complete" ? (cfg.checkCommand
        ? `complete · checks passed on attempt ${st.attempt}`
        : `complete · ${st.attempt} attempts`)
    : st.status === "stalled" ? `stalled · after attempt ${st.attempt}`
    : "stopped";
  return (
    <div className={`loop-strip ${active ? "active" : st.status}`}>
      <span className="loop-glyph">⟳</span>
      <span className="loop-text">{text}</span>
      {cfg.checkCommand && <code className="loop-check-cmd">{cfg.checkCommand}</code>}
      <span className="spacer" />
      {active && (
        <button
          className="tile-act"
          title="Stop the loop — the worktree and its commits stay"
          onClick={async () => {
            try {
              await stopLoop(run.id);
            } catch (e) {
              toastError(e, "Couldn't stop loop");
            }
            onChanged();
          }}
        >■ Stop loop</button>
      )}
    </div>
  );
}

// `onSpawn` lets the host view wrap agent creation with its pre-flight checks
// (missing-CLI install offer, repo readiness); without it the rail's add menu
// falls back to the raw store spawn.
export default function AgentFocus({
  onSpawn,
}: {
  onSpawn?: (agentId: string, opts?: { base: string; mergeTarget: string }) => void;
}) {
  const { runs, focusedRunId, setFocusedRun, refreshRuns, createAgent, createTerminal, selectedProjectId } = useRuns();
  const [showMerge, setShowMerge] = useState(false);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [confirmArchive, setConfirmArchive] = useState(false);
  // Run being renamed (its display title). Any run — agent or terminal.
  const [renaming, setRenaming] = useState<RunInfo | null>(null);
  // "agent" (primary terminal), "run" (RunPanel), or an extra-session id —
  // extra agent tabs sharing this run's worktree.
  const [panel, setPanel] = useState<string>("agent");
  const [sessions, setSessions] = useState<RunSessionInfo[]>([]);
  const [confirmCloseTab, setConfirmCloseTab] = useState<string | null>(null);
  // "+" tab menu: agent profiles to open as an extra tab. Anchored in viewport
  // coordinates like AgentAddMenu so ancestor overflow can't clip it.
  const [addOpen, setAddOpen] = useState(false);
  const [addCoords, setAddCoords] = useState<{ top: number; left: number }>();
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const addBtnRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    setShowMerge(false);
    setPanel("agent");
    setAddOpen(false);
    setSessions([]);
    if (!focusedRunId) return;
    // `live` drops a response that lands after the run changed (a slow fetch for
    // the previous run must not repopulate this one's tab strip). The interval
    // re-polls so a tab whose session died — agent exited, or the app was quit
    // and reopened — updates its status and drops out of the strip on its own,
    // instead of lingering as a dead tab until the next focus change.
    let live = true;
    const load = () =>
      listRunSessions(focusedRunId)
        .then((s) => { if (live) setSessions(s); })
        .catch(() => {});
    load();
    const iv = setInterval(load, 4000);
    return () => { live = false; clearInterval(iv); };
  }, [focusedRunId]);

  const toggleAddMenu = () => {
    setAddOpen((o) => {
      const next = !o;
      if (next) {
        listProfiles()
          .then((ps) => setProfiles(ps.filter((p) => p.name !== "shell")))
          .catch(() => {});
        if (addBtnRef.current) {
          const r = addBtnRef.current.getBoundingClientRect();
          setAddCoords({ top: r.bottom + 4, left: r.left });
        }
      }
      return next;
    });
  };

  const spawnTab = async (agent: string) => {
    setAddOpen(false);
    if (!focusedRunId) return;
    try {
      const s = await startRunSession(focusedRunId, agent);
      setSessions((prev) => [...prev, s]);
      setPanel(s.id);
    } catch (e) {
      toastError(e, "Couldn't open agent tab");
    }
  };

  // The tab strip scrolls horizontally (VS Code-style): vertical wheel input
  // pans it, and the active tab is kept in view when tabs change.
  const tabsScrollRef = useRef<HTMLDivElement>(null);
  const onTabsWheel = (e: React.WheelEvent<HTMLDivElement>) => {
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) e.currentTarget.scrollLeft += e.deltaY;
  };
  useEffect(() => {
    tabsScrollRef.current
      ?.querySelector(".session-tab.on")
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [panel, sessions.length]);
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
              <AgentAddMenu variant="header" projectId={selectedProjectId ?? undefined} onSpawn={onSpawn ?? createAgent} onTerminal={createTerminal} />
              <span className="spacer" />
              <button className="icon-btn" onClick={() => setRailOpen(false)}>«</button>
            </div>
            {runs.map((r) => (
              <button key={r.id} className={`rail-row ${r.id === focusedRunId ? "on" : ""}`}
                onClick={() => setFocusedRun(r.id)}
                onDoubleClick={() => setRenaming(r)}
                title="Double-click to rename">
                <span className={`dot ${runStatus(r).cls}`} />
                <span className="rail-name">
                  {r.kind === "terminal"
                    ? `≳ ${r.title || "terminal"}`
                    : `${r.loopConfig ? "⟳ " : r.raceId ? "∥ " : ""}${r.agent}: ${runName(r)}`}
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
                <span className="focus-name">{focused.title || "terminal"}</span>
                <button className="tile-act icon-only" title="Rename" onClick={() => setRenaming(focused)}><PencilIcon /></button>
                <span className="spacer" />
                <button className="tile-act danger icon-only" title="Close terminal" onClick={() => setConfirmDiscard(true)}><TrashIcon /></button>
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
                    try {
                      await discardRun(id);
                      setFocusedRun(null);
                      await refreshRuns();
                    } catch (e) {
                      toastError(e, "Close failed");
                    }
                  }}
                  onCancel={() => setConfirmDiscard(false)}
                />
              )}
            </>
          ) : (
            <>
              <div className="focus-head">
                <span className={badgeClass(focused.agent)}>{focused.agent}</span>
                {focused.title && <span className="focus-name">{focused.title}</span>}
                <button className="tile-act icon-only" title="Rename agent" onClick={() => setRenaming(focused)}><PencilIcon /></button>
                <code>{focused.branch}</code>
                {panel !== "run" && (
                  <button
                    className={`focus-shell-toggle icon-only ${shellOpen ? "on" : ""}`}
                    title="Toggle terminal in this worktree"
                    onClick={toggleShell}
                  ><TerminalIcon /></button>
                )}
                <span className="spacer" />
                <button className="tile-act danger icon-only" title="Discard agent" onClick={() => setConfirmDiscard(true)}><TrashIcon /></button>
                <button className="tile-act icon-only" title="Archive agent" onClick={() => setConfirmArchive(true)}><InboxIcon /></button>
                <button onClick={() => setShowMerge(true)}>Approve →</button>
              </div>
              <LoopStrip run={focused} onChanged={refreshRuns} />
              <div className="session-tabs">
                <div className="session-tabs-scroll" ref={tabsScrollRef} onWheel={onTabsWheel}>
                  <button
                    className={`session-tab ${panel === "agent" ? "on" : ""}`}
                    onClick={() => setPanel("agent")}
                  >{agentLabel(focused.agent)}</button>
                  {sessions
                    .filter((s) => s.status.state !== "gone" || s.id === panel)
                    .map((s) => (
                    <button
                      key={s.id}
                      className={`session-tab ${panel === s.id ? "on" : ""}`}
                      title={s.agent === "shell"
                        ? "Terminal — extra shell in this worktree"
                        : `${agentLabel(s.agent)} — extra agent in this worktree`}
                      onClick={() => setPanel(s.id)}
                    >
                      {s.agent === "shell" ? "≳ terminal" : agentLabel(s.agent)} · {s.id.split("--").pop()}
                      <span
                        className="tab-close"
                        title="Close this agent tab"
                        onClick={(e) => { e.stopPropagation(); setConfirmCloseTab(s.id); }}
                      >✕</span>
                    </button>
                  ))}
                </div>
                <button
                  ref={addBtnRef}
                  className="session-tab-add"
                  title="New agent tab in this worktree"
                  onClick={toggleAddMenu}
                >+</button>
                <span className="spacer" />
                <button
                  className={`session-tab run-tab ${panel === "run" ? "on" : ""}`}
                  onClick={() => setPanel("run")}
                >Run</button>
              </div>
              {addOpen && (
                <>
                  <div className="agent-menu-backdrop" onClick={() => setAddOpen(false)} />
                  <div className="agent-menu" style={{ position: "fixed", ...addCoords }}>
                    {profiles.map((p) => (
                      <button key={p.name} onClick={() => spawnTab(p.name)}>{agentLabel(p.name)}</button>
                    ))}
                    <div className="agent-menu-sep" />
                    <button onClick={() => spawnTab("shell")}>≳ New terminal</button>
                  </div>
                </>
              )}
              {panel !== "run" ? (
                <div className="focus-body">
                  <FocusTerminal key={panel === "agent" ? focused.id : panel}
                    runId={panel === "agent" ? focused.id : panel}
                    onFirstPrompt={panel === "agent" && !focused.title ? (line) => { setRunTitle(focused.id, line).catch(() => {}); } : undefined} />
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
              {confirmCloseTab && (
                <ConfirmDialog
                  title="Close agent tab?"
                  body="Stop this extra agent session. The worktree, branch and other tabs are untouched."
                  confirmLabel="Close"
                  danger
                  onConfirm={async () => {
                    const sid = confirmCloseTab;
                    setConfirmCloseTab(null);
                    try {
                      await closeRunSession(sid);
                      setSessions((prev) => prev.filter((s) => s.id !== sid));
                      setPanel((p) => (p === sid ? "agent" : p));
                    } catch (e) {
                      toastError(e, "Close failed");
                    }
                  }}
                  onCancel={() => setConfirmCloseTab(null)}
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
              {confirmArchive && (
                <ConfirmDialog
                  title="Archive agent?"
                  body={`Stop "${focused.agent}" and remove its worktree. Any uncommitted work is auto-committed to its "${focused.branch}" branch first.`}
                  confirmLabel="Archive"
                  onConfirm={async () => {
                    const id = focused.id;
                    setConfirmArchive(false);
                    try {
                      await archiveRun(id);
                      setFocusedRun(null);
                      await refreshRuns();
                    } catch (e) {
                      toastError(e, "Archive failed");
                    }
                  }}
                  onCancel={() => setConfirmArchive(false)}
                />
              )}
            </>
          )
        ) : (
          <div className="board empty">Select an agent from the rail.</div>
        )}
      </div>

      {renaming && (
        <PromptDialog
          title={renaming.kind === "terminal" ? "Rename terminal" : "Rename agent"}
          placeholder="New name"
          initial={renaming.title ?? ""}
          confirmLabel="Rename"
          onConfirm={(v) => {
            const id = renaming.id;
            setRenaming(null);
            renameRun(id, v).then(refreshRuns).catch((e) => toastError(e, "Rename failed"));
          }}
          onCancel={() => setRenaming(null)}
        />
      )}
    </div>
  );
}
