import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuEntry =
  | { kind?: "item"; label: string; glyph?: React.ReactNode; hint?: string; danger?: boolean; disabled?: boolean; onClick: () => void }
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
  const ref = useRef<HTMLDivElement>(null);
  // Null until measured — see below.
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);

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
    setPos({
      x: Math.max(EDGE_GAP, Math.min(x, window.innerWidth - r.width - EDGE_GAP)),
      y: Math.max(EDGE_GAP, Math.min(y, window.innerHeight - r.height - EDGE_GAP)),
    });
  }, [x, y]);

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
      <div ref={ref} className="ctx-menu"
        style={pos ? { left: pos.x, top: pos.y } : { left: 0, top: 0, visibility: "hidden" }}
        onMouseDown={(e) => e.stopPropagation()}>
        {items.map((it, i) => {
          if (it.kind === "separator") return <div key={i} className="ctx-sep" />;
          if (it.kind === "header") return <div key={i} className="ctx-header">{it.label}</div>;
          return (
            <button key={i} className={`ctx-item ${it.danger ? "danger" : ""}`} disabled={it.disabled}
              onClick={() => { onClose(); it.onClick(); }}>
              <span className="ctx-glyph">{it.glyph}</span>
              <span className="ctx-label">{it.label}</span>
              {it.hint && <span className="ctx-hint">{it.hint}</span>}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** Anchor helper: viewport coordinates just under a clicked element. */
export function menuAt(e: React.MouseEvent): { x: number; y: number } {
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
  return { x: r.left, y: r.bottom + 4 };
}
