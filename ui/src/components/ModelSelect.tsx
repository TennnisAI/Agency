import { useEffect, useRef, useState } from "react";
import { AgentModelInfo } from "../api";
import { modelIdError, modelOptions } from "../agents";
import { useDismissOnResize } from "../hooks/useDismissOnResize";

// Model picker for a single agent: the vendor's stable aliases, whatever has
// been used with this agent before, and a field for anything else.
//
// The list is deliberately short. Every one of these CLIs can reach models
// Agency has never heard of, and a hardcoded catalogue of dated model names
// would be wrong within a release, so the offered ids are only the aliases the
// vendor keeps pointed at the current model. The typed field is the real
// answer for the rest, and using one once puts it in the list.
//
// Same trigger-plus-portal shape as BranchSelect, for the same reason: a native
// <select> is sized by its widest option and can't hold a text field anyway.
export default function ModelSelect({
  info,
  value,
  onChange,
  compact = false,
}: {
  // Undefined while the model list is still loading, and carrying
  // `supported: false` for an agent whose CLI takes no model flag. Either way
  // the control renders nothing (see below).
  info: AgentModelInfo | undefined;
  value: string | null;
  onChange: (model: string | null) => void;
  // Chip form, for a row that already names the agent.
  compact?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  const [error, setError] = useState("");
  const [coords, setCoords] = useState<{
    top?: number; bottom?: number; left?: number; right?: number; minWidth: number;
  }>({ minWidth: 0 });
  const btnRef = useRef<HTMLButtonElement>(null);

  // Escape closes this popup only, leaving the menu or dialog it opened from
  // standing — the same thing a click on the backdrop does.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(false);
      btnRef.current?.focus();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open]);

  // The popup is placed against the trigger's rect, measured once at open.
  useDismissOnResize(open, () => setOpen(false));

  // Nothing to pick until we know this agent can be told a model at all: an
  // agent whose CLI takes no model flag gets no control, and neither does one
  // whose entry hasn't loaded, so a failed load leaves runs on their defaults
  // rather than offering a choice that could not be applied.
  if (!info?.supported) return null;

  const options = modelOptions(info.suggested, info.recent);

  const toggle = () => {
    setOpen((o) => {
      const next = !o;
      const r = btnRef.current?.getBoundingClientRect();
      if (next) {
        setTyped("");
        setError("");
      }
      if (next && r) {
        const MENU_W = Math.max(r.width, 240);
        const height = Math.min(320, (options.length + 3) * 30 + 60);
        const fitsRight = r.left + MENU_W <= window.innerWidth - 8;
        const fitsBelow = r.bottom + 4 + height <= window.innerHeight - 8;
        setCoords({
          ...(fitsBelow ? { top: r.bottom + 4 } : { bottom: window.innerHeight - r.top + 4 }),
          ...(fitsRight ? { left: r.left } : { right: window.innerWidth - r.right }),
          minWidth: Math.max(r.width, 240),
        });
      }
      return next;
    });
  };

  const pick = (model: string | null) => {
    setOpen(false);
    onChange(model);
  };

  const applyTyped = () => {
    const problem = modelIdError(typed);
    if (problem) return setError(problem);
    pick(typed.trim());
  };

  const { minWidth, ...pos } = coords;
  const label = value ?? "default";

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        className={compact ? "model-chip" : "branch-select"}
        title={value ? `Model: ${value}` : "Model: the agent's own default"}
        onClick={(e) => { e.stopPropagation(); toggle(); }}
      >
        <span className="branch-select-name">{label}</span>
        <span className="branch-select-caret">▾</span>
      </button>
      {open && (
        <>
          <div className="branch-menu-backdrop" onClick={(e) => { e.stopPropagation(); setOpen(false); }} />
          <div className="agent-menu model-menu" style={{ position: "fixed", minWidth, ...pos }}>
            <button
              type="button"
              className={value === null ? "on" : ""}
              onClick={(e) => { e.stopPropagation(); pick(null); }}
            >
              <span className="branch-select-name">Agent's default</span>
              {value === null && <span className="branch-menu-tick">✓</span>}
            </button>
            {options.length > 0 && <div className="agent-menu-sep" />}
            {options.map((m) => (
              <button
                key={m}
                type="button"
                title={m}
                className={m === value ? "on" : ""}
                onClick={(e) => { e.stopPropagation(); pick(m); }}
              >
                <span className="branch-select-name">{m}</span>
                {m === value && <span className="branch-menu-tick">✓</span>}
              </button>
            ))}
            <div className="agent-menu-sep" />
            <div className="model-other" onClick={(e) => e.stopPropagation()}>
              <input
                className="settings-input"
                placeholder="Other model…"
                value={typed}
                spellCheck={false}
                autoComplete="off"
                onChange={(e) => { setTyped(e.target.value); setError(""); }}
                onKeyDown={(e) => {
                  if (e.key !== "Enter") return;
                  e.preventDefault();
                  applyTyped();
                }}
              />
              <button type="button" className="model-use" disabled={!typed.trim()} onClick={applyTyped}>
                Use
              </button>
            </div>
            {error && <div className="model-note model-error">{error}</div>}
            {!error && info.listCommand && (
              <div className="model-note">
                Run <code>{info.listCommand}</code> to see what this agent can use.
              </div>
            )}
          </div>
        </>
      )}
    </>
  );
}
