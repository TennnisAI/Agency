import { useState } from "react";
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
  const choose = (id: string) => { setOpen(false); onSpawn(id); };
  const chooseTerminal = () => { setOpen(false); onTerminal?.(); };

  return (
    <div className="agent-add">
      {variant === "icon" ? (
        <button className="icon-btn" title="Add agent" onClick={() => setOpen((o) => !o)}>+</button>
      ) : (
        <button className="btn-primary" onClick={() => setOpen((o) => !o)}>+ Agent ▾</button>
      )}
      {open && (
        <div className="agent-menu" onMouseLeave={() => setOpen(false)}>
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
      )}
    </div>
  );
}
