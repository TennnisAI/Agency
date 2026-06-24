import { useRef, useState } from "react";
import { AGENT_TYPES } from "../agents";

export default function AgentAddMenu({
  onSpawn,
  onTerminal,
  variant = "button",
}: {
  onSpawn: (agentId: string) => void;
  onTerminal?: () => void;
  variant?: "button" | "icon";
}) {
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; left?: number; right?: number }>();
  const btnRef = useRef<HTMLButtonElement>(null);

  // Anchor the menu in viewport coordinates so an ancestor's `overflow: hidden`
  // (e.g. the focus rail) can't clip it. The icon button sits at the left of its
  // pane and opens rightward; the primary button sits at the top-right and opens
  // leftward.
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

  const choose = (id: string) => { setOpen(false); onSpawn(id); };
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
          </div>
        </>
      )}
    </div>
  );
}
