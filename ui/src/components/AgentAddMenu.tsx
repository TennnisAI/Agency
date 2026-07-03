import { useEffect, useRef, useState } from "react";
import { AgentProfile, listProfiles, listProjectBranches } from "../api";
import { agentLabel } from "../agents";
import { effectiveMergeTarget } from "../lib/branchTargets";
import RaceDialog from "./RaceDialog";
import GhImportDialog from "./GhImportDialog";

// The built-in shell profile backs the hardcoded "New terminal" entry, so it is
// never listed as a spawnable agent.
const TERMINAL_PROFILE = "shell";

export default function AgentAddMenu({
  onSpawn,
  onTerminal,
  variant = "button",
  projectId,
}: {
  onSpawn: (agentId: string, opts?: { base: string; mergeTarget: string }) => void;
  // Required (not optional) so every call site exposes the same options — the two
  // add-menus (top-right "+ Agent" and the rail "Agents +" header) cannot drift
  // out of sync.
  onTerminal: () => void;
  variant?: "button" | "icon" | "header";
  projectId?: string;
}) {
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; left?: number; right?: number }>();
  const [agents, setAgents] = useState<AgentProfile[]>([]);
  const [raceOpen, setRaceOpen] = useState(false);
  const [importMode, setImportMode] = useState<"issue" | "pr" | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);

  // Agent options are derived from the defined profiles (not hardcoded) so newly
  // added definitions show up. Both add-menus read the same source, so they stay
  // in sync. Refresh on open so a profile added moments ago appears immediately.
  const loadAgents = () =>
    listProfiles()
      .then((ps) => setAgents(ps.filter((p) => p.name !== TERMINAL_PROFILE)))
      .catch(() => {});
  useEffect(() => { loadAgents(); }, []);

  // Branch-picker state (only used when projectId is supplied).
  const [branches, setBranches] = useState<string[]>([]);
  const [base, setBase] = useState<string>("");
  const [targetOverride, setTargetOverride] = useState<string | null>(null);

  const showPicker = !!projectId;
  const mergeTarget = effectiveMergeTarget(base, targetOverride);
  const diverged = targetOverride !== null && targetOverride !== base;

  // Load branches when the menu opens (cheap; reflects any branch the user just made).
  useEffect(() => {
    if (!open || !projectId) return;
    let live = true;
    listProjectBranches(projectId)
      .then((pb) => {
        if (!live) return;
        setBranches(pb.branches);
        setBase((b) => (b && pb.branches.includes(b) ? b : pb.current));
      })
      .catch(() => { /* leave selects empty; spawn falls back to HEAD */ });
    return () => { live = false; };
  }, [open, projectId]);

  // Anchor the menu in viewport coordinates so an ancestor's `overflow: hidden`
  // (e.g. the focus rail) can't clip it. The icon/header buttons sit at the left
  // of their pane and open rightward; the primary button sits at the top-right
  // and opens leftward.
  const toggle = () => {
    setOpen((o) => {
      const next = !o;
      if (next) loadAgents();
      if (next && btnRef.current) {
        const r = btnRef.current.getBoundingClientRect();
        setCoords(
          variant === "button"
            ? { top: r.bottom + 4, right: window.innerWidth - r.right }
            : { top: r.bottom + 4, left: r.left },
        );
      }
      return next;
    });
  };

  const choose = (id: string) => {
    setOpen(false);
    if (showPicker && base) onSpawn(id, { base, mergeTarget });
    else onSpawn(id);
  };
  const chooseTerminal = () => { setOpen(false); onTerminal(); };

  return (
    <div className="agent-add">
      {variant === "header" ? (
        <button ref={btnRef} className="rail-head-add" title="Add agent" onClick={toggle}>Agents <span className="rail-head-plus">+</span></button>
      ) : variant === "icon" ? (
        <button ref={btnRef} className="icon-btn" title="Add agent" onClick={toggle}>+</button>
      ) : (
        <button ref={btnRef} className="btn-primary" onClick={toggle}>+ Agent ▾</button>
      )}
      {open && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setOpen(false)} />
          <div className="agent-menu" style={{ position: "fixed", ...coords }}>
            {agents.map((a) => (
              <button key={a.name} onClick={() => choose(a.name)}>{agentLabel(a.name)}</button>
            ))}
            <div className="agent-menu-sep" />
            <button onClick={() => { setOpen(false); setRaceOpen(true); }}>∥ Race agents…</button>
            <button onClick={() => { setOpen(false); setImportMode("issue"); }}>◈ GitHub issue…</button>
            <button onClick={() => { setOpen(false); setImportMode("pr"); }}>⇋ GitHub PR…</button>
            <div className="agent-menu-sep" />
            <button onClick={chooseTerminal}>≳ New terminal</button>
            {showPicker && branches.length > 0 && (
              <>
                <div className="agent-menu-sep" />
                <div className="branch-picker">
                  <label className="branch-row">
                    <span>from</span>
                    <select
                      value={base}
                      onChange={(e) => setBase(e.target.value)}
                      onClick={(e) => e.stopPropagation()}
                    >
                      {branches.map((b) => <option key={b} value={b}>{b}</option>)}
                    </select>
                  </label>
                  <label className="branch-row">
                    <span>into</span>
                    <select
                      value={mergeTarget}
                      onChange={(e) => setTargetOverride(e.target.value === base ? null : e.target.value)}
                      onClick={(e) => e.stopPropagation()}
                    >
                      {branches.map((b) => <option key={b} value={b}>{b}</option>)}
                    </select>
                    {diverged && (
                      <button
                        type="button"
                        className="branch-reset"
                        title="Sync merge target to base"
                        onClick={(e) => { e.stopPropagation(); setTargetOverride(null); }}
                      >↺</button>
                    )}
                  </label>
                </div>
              </>
            )}
          </div>
        </>
      )}
      {raceOpen && <RaceDialog onClose={() => setRaceOpen(false)} />}
      {importMode && <GhImportDialog mode={importMode} onClose={() => setImportMode(null)} />}
    </div>
  );
}
