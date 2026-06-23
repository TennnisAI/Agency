import { useState } from "react";
import { AGENT_TYPES } from "../agents";

export default function AgentAddMenu({
  onSpawn,
  variant = "button",
}: {
  onSpawn: (agentId: string) => void;
  variant?: "button" | "icon";
}) {
  const [open, setOpen] = useState(false);
  const choose = (id: string) => { setOpen(false); onSpawn(id); };

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
        </div>
      )}
    </div>
  );
}
