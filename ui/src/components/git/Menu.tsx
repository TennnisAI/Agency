import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuEntry =
  | { kind?: "item"; label: string; glyph?: React.ReactNode; hint?: string; danger?: boolean; disabled?: boolean; checked?: boolean; onClick: () => void }
  | { kind: "submenu"; label: string; glyph?: React.ReactNode; items: MenuEntry[] }
  | { kind: "separator" }
  | { kind: "header"; label: string };

/** Breathing room kept between the menu and the window edge. */
const EDGE_GAP = 4;

/**
 * Popup menu at fixed viewport coordinates (context menus and button dropdowns
 * both use this — fixed positioning escapes every overflow/scroll container).
 * Closes on outside click, Escape, or after running an item.
 */
export default function Menu({ x, y, items, onClose }: {
  x: number;
  y: number;
  items: MenuEntry[];
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    // A resize would strand the menu at coordinates measured against the old
    // window; it's anchored to a row that has moved anyway, so just close it.
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  return (
    <div className="menu-overlay" onMouseDown={onClose} onContextMenu={(e) => { e.preventDefault(); onClose(); }}>
      <Panel x={x} y={y} items={items} onClose={onClose} />
    </div>
  );
}

/** Where an open submenu hangs: to the right of its row, or left if cramped. */
interface SubPos { index: number; x: number; y: number; flipX: number }

/** One panel of a menu. Submenu rows open further panels, recursively. */
function Panel({ x, y, items, onClose, flipX }: {
  x: number;
  y: number;
  items: MenuEntry[];
  onClose: () => void;
  /** Right edge to hang from instead, when the panel won't fit to the right. */
  flipX?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  // Null until measured — see below.
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  const [sub, setSub] = useState<SubPos | null>(null);

  // Clamp into the viewport so a menu opened near an edge shifts inward instead
  // of spilling past it. The first pass renders at the origin, hidden, because a
  // fixed element is shrink-to-fit against the room left to its right: measuring
  // it at a near-the-edge x reports a squeezed width and clamping to that width
  // would bake the squeeze in. Both passes run in a layout effect, before paint,
  // so the origin pass never reaches the screen.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    // A submenu that doesn't fit to the right flips to the other side of its
    // parent row rather than sliding inward on top of it.
    const wantX = flipX !== undefined && x + r.width > window.innerWidth - EDGE_GAP
      ? flipX - r.width
      : x;
    setPos({
      x: Math.max(EDGE_GAP, Math.min(wantX, window.innerWidth - r.width - EDGE_GAP)),
      y: Math.max(EDGE_GAP, Math.min(y, window.innerHeight - r.height - EDGE_GAP)),
    });
  }, [x, y, flipX]);

  const openSub = (index: number, row: HTMLElement) => {
    const r = row.getBoundingClientRect();
    // Overlap the parent's padding a little so the pointer can cross into the
    // submenu without passing over a sibling row (which would close it).
    setSub({ index, x: r.right - 3, y: r.top - 5, flipX: r.left + 3 });
  };

  return (
    <>
      <div ref={ref} className="ctx-menu"
        style={pos ? { left: pos.x, top: pos.y } : { left: 0, top: 0, visibility: "hidden" }}
        onMouseDown={(e) => e.stopPropagation()}>
        {items.map((it, i) => {
          if (it.kind === "separator") return <div key={i} className="ctx-sep" />;
          if (it.kind === "header") return <div key={i} className="ctx-header">{it.label}</div>;
          if (it.kind === "submenu") {
            return (
              <button key={i} className={`ctx-item${sub?.index === i ? " open" : ""}`}
                onMouseEnter={(e) => openSub(i, e.currentTarget)}
                onClick={(e) => openSub(i, e.currentTarget)}>
                <span className="ctx-glyph">{it.glyph}</span>
                <span className="ctx-label">{it.label}</span>
                <span className="ctx-more">›</span>
              </button>
            );
          }
          return (
            <button key={i} className={`ctx-item ${it.danger ? "danger" : ""}`} disabled={it.disabled}
              onMouseEnter={() => setSub(null)}
              onClick={() => { onClose(); it.onClick(); }}>
              <span className="ctx-glyph">{it.checked ? "✓" : it.glyph}</span>
              <span className="ctx-label">{it.label}</span>
              {it.hint && <span className="ctx-hint">{it.hint}</span>}
            </button>
          );
        })}
      </div>
      {sub && items[sub.index]?.kind === "submenu" && (
        // Keyed by row so switching submenus remounts: a fresh panel measures
        // itself at the origin (see the layout effect) instead of at the
        // previous submenu's position, where it could measure squeezed.
        <Panel key={sub.index} x={sub.x} y={sub.y} flipX={sub.flipX} onClose={onClose}
          items={(items[sub.index] as { items: MenuEntry[] }).items} />
      )}
    </>
  );
}

/** Anchor helper: viewport coordinates just under a clicked element. */
export function menuAt(e: React.MouseEvent): { x: number; y: number } {
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
  return { x: r.left, y: r.bottom + 4 };
}
