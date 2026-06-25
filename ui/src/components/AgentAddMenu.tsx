import { useEffect, useRef, useState } from "react";
import { AgentProfile, listProfiles } from "../api";
import { agentLabel } from "../agents";

// The built-in shell profile backs the hardcoded "New terminal" entry, so it is
// never listed as a spawnable agent.
const TERMINAL_PROFILE = "shell";

export default function AgentAddMenu({
  onSpawn,
  onTerminal,
  variant = "button",
}: {
  onSpawn: (agentId: string) => void;
  // Required (not optional) so every call site exposes the same options — the two
  // add-menus (top-right "+ Agent" and the rail "Agents +" header) cannot drift
  // out of sync.
  onTerminal: () => void;
  variant?: "button" | "icon" | "header";
}) {
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; left?: number; right?: number }>();
  const [agents, setAgents] = useState<AgentProfile[]>([]);
  const btnRef = useRef<HTMLButtonElement>(null);

  // Agent options are derived from the defined profiles (not hardcoded) so newly
  // added definitions show up. Both add-menus read the same source, so they stay
  // in sync. Refresh on open so a profile added moments ago appears immediately.
  const loadAgents = () =>
    listProfiles()
      .then((ps) => setAgents(ps.filter((p) => p.name !== TERMINAL_PROFILE)))
      .catch(() => {});
  useEffect(() => { loadAgents(); }, []);

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

  const choose = (id: string) => { setOpen(false); onSpawn(id); };
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
            <button onClick={chooseTerminal}>≳ New terminal</button>
          </div>
        </>
      )}
    </div>
  );
}
