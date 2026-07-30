import { useRef, useState } from "react";

// A quiet pill that opens an in-app menu — replaces native <select>, whose
// popup renders at the OS layer and lands wherever it pleases in the webview.
// Same fixed-coords + backdrop pattern as the IssueRow menus, so ancestors'
// overflow can't clip it and it can't drift from its trigger.
export default function PillSelect<T extends string | number>({
  value,
  options,
  onChange,
  defaultValue,
  title,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  // When set, the pill only lights up while the value differs from this —
  // an untouched filter stays quiet.
  defaultValue?: T;
  title?: string;
}) {
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; left?: number; right?: number }>({ top: 0 });
  const btnRef = useRef<HTMLButtonElement>(null);
  const current = options.find((o) => o.value === value);

  const openMenu = () => {
    const r = btnRef.current?.getBoundingClientRect();
    if (r) {
      const MENU_W = 220; // .agent-menu width class
      const fitsRight = r.left + MENU_W <= window.innerWidth - 8;
      setCoords(
        fitsRight
          ? { top: r.bottom + 4, left: r.left }
          : { top: r.bottom + 4, right: window.innerWidth - r.right },
      );
    }
    setOpen(true);
  };

  return (
    <>
      <button
        ref={btnRef}
        className={`filter-pill${defaultValue !== undefined && value !== defaultValue ? " active" : ""}`}
        title={title}
        onClick={openMenu}
      >
        {/* Every label is rendered stacked in one grid cell (only the active
            one visible) so the pill is sized by its widest option and never
            resizes when the selection changes. */}
        <span className="filter-pill-labels">
          {options.map((o) => (
            <span key={String(o.value)} className={o.value === value ? "on" : ""}>
              {o.label}
            </span>
          ))}
          {current === undefined && <span className="on">{String(value)}</span>}
        </span>
        <span className="filter-caret">▾</span>
      </button>
      {open && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setOpen(false)} />
          <div className="agent-menu filter-menu" style={{ position: "fixed", ...coords }}>
            {options.map((o) => (
              <button
                key={String(o.value)}
                onClick={() => {
                  setOpen(false);
                  if (o.value !== value) onChange(o.value);
                }}
              >
                {o.label}
                {o.value === value ? " ✓" : ""}
              </button>
            ))}
          </div>
        </>
      )}
    </>
  );
}
