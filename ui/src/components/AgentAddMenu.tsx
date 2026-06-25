import { useEffect, useRef, useState } from "react";
import { AGENT_TYPES } from "../agents";
import { listProjectBranches } from "../api";
import { effectiveMergeTarget } from "../lib/branchTargets";

export default function AgentAddMenu({
  onSpawn,
  onTerminal,
  variant = "button",
  projectId,
}: {
  onSpawn: (agentId: string, opts?: { base: string; mergeTarget: string }) => void;
  onTerminal?: () => void;
  variant?: "button" | "icon";
  projectId?: string;
}) {
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; left?: number; right?: number }>();
  const btnRef = useRef<HTMLButtonElement>(null);

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

  const toggle = () => {
    setOpen((o) => {
      const next = !o;
      if (next && btnRef.current) {
        const r = btnRef.current.getBoundingClientRect();
        setCoords(
          variant === "icon"
            ? { top: r.bottom + 4, left: r.left }
            : { top: r.bottom + 4, right: window.innerWidth - r.right },
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
  const chooseTerminal = () => { setOpen(false); onTerminal?.(); };

  return (
    <div className="agent-add">
      {variant === "icon" ? (
        <button ref={btnRef} className="icon-btn" title="Add agent" onClick={toggle}>+</button>
      ) : (
        <button ref={btnRef} className="btn-primary" onClick={toggle}>+ Agent ▾</button>
      )}
      {open && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setOpen(false)} />
          <div className="agent-menu" style={{ position: "fixed", ...coords }}>
            {AGENT_TYPES.map((a) => (
              <button key={a.id} onClick={() => choose(a.id)}>{a.label}</button>
            ))}
            {onTerminal && (
              <>
                <div className="agent-menu-sep" />
                <button onClick={chooseTerminal}>≳ New terminal</button>
              </>
            )}
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
    </div>
  );
}
