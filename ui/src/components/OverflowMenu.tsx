import { Fragment, ReactNode, useRef, useState } from "react";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { MoreIcon } from "./icons";

export type OverflowItem = {
  label: string;
  onSelect: () => void;
  icon?: ReactNode;
  danger?: boolean;
  /** Rendered above this item. */
  separator?: boolean;
};

// The "…" button that holds a header's secondary actions. Toolbars stay down to
// one primary action plus this, instead of a row of competing chips that wrap
// the moment the pane narrows.
//
// The menu is anchored in viewport coordinates (like AgentAddMenu) so an
// ancestor's `overflow: hidden` can't clip it.
export default function OverflowMenu({
  items,
  title = "More actions",
  icon,
  buttonClass = "head-icon-btn",
}: {
  items: OverflowItem[];
  title?: string;
  /** Trigger glyph, when the host wants something other than the "…". */
  icon?: ReactNode;
  /** Trigger class. Gains " on" while the menu is open, either way. */
  buttonClass?: string;
}) {
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; right: number }>();
  const btnRef = useRef<HTMLButtonElement>(null);

  const toggle = () => {
    setOpen((o) => {
      const next = !o;
      if (next && btnRef.current) {
        const r = btnRef.current.getBoundingClientRect();
        setCoords({ top: r.bottom + 5, right: Math.max(8, window.innerWidth - r.right) });
      }
      return next;
    });
  };

  // The header this hangs off shifts under a resize; the menu's coords don't.
  useDismissOnResize(open, () => setOpen(false));

  if (items.length === 0) return null;
  return (
    <>
      <button
        ref={btnRef}
        className={`${buttonClass}${open ? " on" : ""}`}
        title={title}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={toggle}
      >{icon ?? <MoreIcon />}</button>
      {open && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setOpen(false)} />
          <div className="agent-menu overflow-menu" role="menu" style={{ position: "fixed", ...coords }}>
            {items.map((it, i) => (
              <Fragment key={it.label}>
                {it.separator && i > 0 && <div className="agent-menu-sep" />}
                <button
                  role="menuitem"
                  className={it.danger ? "danger" : undefined}
                  onClick={() => { setOpen(false); it.onSelect(); }}
                >
                  {it.icon && <span className="overflow-menu-ico">{it.icon}</span>}
                  <span>{it.label}</span>
                </button>
              </Fragment>
            ))}
          </div>
        </>
      )}
    </>
  );
}
