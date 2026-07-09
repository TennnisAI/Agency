import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuEntry =
  | { kind?: "item"; label: string; glyph?: React.ReactNode; hint?: string; danger?: boolean; disabled?: boolean; onClick: () => void }
  | { kind: "separator" }
  | { kind: "header"; label: string };

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
  const [pos, setPos] = useState({ x, y });

  // Clamp into the viewport once rendered (menus opened near an edge flip inward).
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(x, window.innerWidth - r.width - 4)),
      y: Math.max(4, Math.min(y, window.innerHeight - r.height - 4)),
    });
  }, [x, y]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="menu-overlay" onMouseDown={onClose} onContextMenu={(e) => { e.preventDefault(); onClose(); }}>
      <div ref={ref} className="ctx-menu" style={{ left: pos.x, top: pos.y }}
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
