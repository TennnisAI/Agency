import { useEffect, useRef, useState } from "react";

// Branch picker for the agent menu. A native <select> can't be trusted here:
// WebKit sizes it from its widest <option>, so one long branch name pushes the
// control straight out of the menu. This is a plain button whose label is
// clipped to whatever width the row gives it, with the full name on hover, and
// an in-app popup instead of the OS-layer select popup.
export default function BranchSelect({
  value,
  options,
  onChange,
  label,
}: {
  value: string;
  options: string[];
  onChange: (v: string) => void;
  // Spoken form of the field ("base branch"), used for the trigger's tooltip so
  // the hover tells you which end of the merge the name belongs to.
  label: string;
}) {
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{
    top?: number; bottom?: number; left?: number; right?: number; minWidth: number;
  }>({ minWidth: 0 });
  const btnRef = useRef<HTMLButtonElement>(null);

  const toggle = () => {
    setOpen((o) => {
      const next = !o;
      const r = btnRef.current?.getBoundingClientRect();
      if (next && r) {
        // Same fixed-coords + backdrop pattern as the menu around it, so an
        // ancestor's overflow can't clip the list. Flip left/up when the popup
        // wouldn't fit, since this menu often opens near an edge.
        const MENU_W = Math.max(r.width, 220);
        const height = Math.min(260, options.length * 30 + 10);
        const fitsRight = r.left + MENU_W <= window.innerWidth - 8;
        const fitsBelow = r.bottom + 4 + height <= window.innerHeight - 8;
        setCoords({
          ...(fitsBelow ? { top: r.bottom + 4 } : { bottom: window.innerHeight - r.top + 4 }),
          ...(fitsRight ? { left: r.left } : { right: window.innerWidth - r.right }),
          minWidth: r.width,
        });
      }
      return next;
    });
  };

  // Escape closes the branch list only, leaving the agent menu it opened from
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

  const { minWidth, ...pos } = coords;

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        className="branch-select"
        title={`${value} (${label})`}
        onClick={(e) => { e.stopPropagation(); toggle(); }}
      >
        <span className="branch-select-name">{value}</span>
        <span className="branch-select-caret">▾</span>
      </button>
      {open && (
        <>
          <div className="branch-menu-backdrop" onClick={(e) => { e.stopPropagation(); setOpen(false); }} />
          <div className="agent-menu branch-menu" style={{ position: "fixed", minWidth, ...pos }}>
            {options.map((b) => (
              <button
                key={b}
                type="button"
                title={b}
                className={b === value ? "on" : ""}
                // Keep the selected row in view when the list is long enough to
                // scroll, so opening the popup shows where you already are.
                ref={b === value ? (el) => el?.scrollIntoView({ block: "nearest" }) : undefined}
                onClick={(e) => {
                  e.stopPropagation();
                  setOpen(false);
                  if (b !== value) onChange(b);
                }}
              >
                <span className="branch-select-name">{b}</span>
                {b === value && <span className="branch-menu-tick">✓</span>}
              </button>
            ))}
          </div>
        </>
      )}
    </>
  );
}
