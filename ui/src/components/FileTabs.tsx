import { useEffect, useRef, useState } from "react";
import { baseName, parentPath } from "../lib/filePath";

/**
 * The editor tab strip. Duplicate basenames get their parent folder as a
 * dimmed suffix; dirty tabs show a dot; middle-click or the hover × closes.
 * When tabs overflow, arrow buttons appear at the ends (VS Code-style); the
 * strip also scrolls with the wheel/trackpad and keeps the active tab in view.
 */
export default function FileTabs({
  open, active, dirty, onActivate, onClose,
}: {
  open: string[];
  active: string | null;
  dirty: Set<string>;
  onActivate: (path: string) => void;
  onClose: (path: string) => void;
}) {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const [overflow, setOverflow] = useState(false);
  const [canLeft, setCanLeft] = useState(false);
  const [canRight, setCanRight] = useState(false);

  const measure = () => {
    const el = scrollerRef.current;
    if (!el) return;
    setOverflow(el.scrollWidth > el.clientWidth + 1);
    setCanLeft(el.scrollLeft > 0);
    setCanRight(el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
  };

  // Re-measure when the tab set changes and when the pane is resized.
  useEffect(() => {
    measure();
    const el = scrollerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Keep the active tab visible — a freshly opened tab lands off-screen right
  // otherwise.
  useEffect(() => {
    if (!active) return;
    scrollerRef.current
      ?.querySelector<HTMLElement>(`[data-path="${CSS.escape(active)}"]`)
      ?.scrollIntoView({ inline: "nearest", block: "nearest" });
    measure();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, open]);

  const scrollBy = (dir: -1 | 1) => {
    const el = scrollerRef.current;
    if (!el) return;
    el.scrollBy({ left: dir * Math.max(120, el.clientWidth * 0.6), behavior: "smooth" });
  };

  const counts = new Map<string, number>();
  for (const p of open) {
    const b = baseName(p);
    counts.set(b, (counts.get(b) ?? 0) + 1);
  }

  return (
    <div className="file-tabs-wrap">
      {overflow && (
        <button
          className="file-tabs-arrow"
          title="Scroll tabs left"
          aria-label="Scroll tabs left"
          disabled={!canLeft}
          onClick={() => scrollBy(-1)}
        >
          <Chevron dir="left" />
        </button>
      )}
      <div
        className="file-tabs"
        ref={scrollerRef}
        role="tablist"
        onScroll={measure}
        onWheel={(e) => {
          // The strip has no vertical axis; let a mouse wheel drive it sideways.
          if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
            scrollerRef.current?.scrollBy({ left: e.deltaY });
          }
        }}
      >
        {open.map((p) => {
          const name = baseName(p);
          return (
            <div
              key={p}
              data-path={p}
              role="tab"
              aria-selected={p === active}
              className={`file-tab ${p === active ? "on" : ""}`}
              title={p}
              onClick={() => onActivate(p)}
              onAuxClick={(e) => { if (e.button === 1) { e.preventDefault(); onClose(p); } }}
            >
              <span className="file-tab-name">{name}</span>
              {(counts.get(name) ?? 0) > 1 && (
                <span className="file-tab-dir">{parentPath(p) || "/"}</span>
              )}
              {dirty.has(p) && <span className="file-tab-dirty">●</span>}
              <button
                className="file-tab-close"
                title="Close (⌘W)"
                aria-label={`Close ${name}`}
                onClick={(e) => { e.stopPropagation(); onClose(p); }}
              >
                ×
              </button>
            </div>
          );
        })}
      </div>
      {overflow && (
        <button
          className="file-tabs-arrow"
          title="Scroll tabs right"
          aria-label="Scroll tabs right"
          disabled={!canRight}
          onClick={() => scrollBy(1)}
        >
          <Chevron dir="right" />
        </button>
      )}
    </div>
  );
}

// Stroked chevron matching the app's icon convention.
function Chevron({ dir }: { dir: "left" | "right" }) {
  return (
    <svg
      width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden
    >
      {dir === "left" ? <polyline points="15 18 9 12 15 6" /> : <polyline points="9 18 15 12 9 6" />}
    </svg>
  );
}
