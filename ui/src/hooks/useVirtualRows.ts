import { useCallback, useLayoutEffect, useRef, useState } from "react";

export interface Range {
  start: number;
  end: number;
}

const DEFAULT_OVERSCAN = 8;
const DEFAULT_ROW_H = 22;

/**
 * Pure windowing math. `relTop` is the rows region's top edge measured from the
 * scroll viewport's top (0 = flush with the top, negative once the region has
 * scrolled above it). Returns the half-open `[start, end)` slice of rows that
 * fall within the viewport, padded by `overscan` on each side and clamped to
 * `[0, count]`. When the region sits entirely above or below the viewport the
 * slice collapses to empty.
 */
export function visibleRange(
  relTop: number,
  viewportH: number,
  rowH: number,
  count: number,
  overscan: number,
): Range {
  if (count === 0 || rowH <= 0) return { start: 0, end: 0 };
  const firstVis = Math.floor((0 - relTop) / rowH);
  const lastVis = Math.ceil((viewportH - relTop) / rowH);
  const clamp = (n: number) => Math.min(count, Math.max(0, n));
  return { start: clamp(firstVis - overscan), end: clamp(lastVis + overscan) };
}

/** Nearest ancestor that scrolls vertically, or null (window-scrolled). */
function findScrollParent(el: HTMLElement | null): HTMLElement | null {
  let node = el?.parentElement ?? null;
  while (node) {
    const oy = getComputedStyle(node).overflowY;
    if (oy === "auto" || oy === "scroll") return node;
    node = node.parentElement;
  }
  return null;
}

/**
 * Windows a uniform-height row list against its nearest scrolling ancestor, so
 * only the visible slice is mounted. Groups share one scroll container, so the
 * region's position is measured live (its offset shifts when sibling groups
 * above it grow/collapse) rather than tracked by index.
 *
 * Returns a ref for the rows wrapper plus the slice to render and the spacer
 * heights that preserve the full scroll extent above and below it.
 */
export function useVirtualRows(count: number, overscan = DEFAULT_OVERSCAN) {
  const regionRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLElement | null>(null);
  const rowHRef = useRef(DEFAULT_ROW_H);
  const [range, setRange] = useState<Range>(() => ({ start: 0, end: Math.min(count, overscan) }));

  const recompute = useCallback(() => {
    const region = regionRef.current;
    const scroller = scrollRef.current;
    if (!region) return;
    // Measure a real row once it exists; row height is uniform within a group.
    const row = region.querySelector<HTMLElement>(".git-row");
    if (row && row.offsetHeight > 0) rowHRef.current = row.offsetHeight;
    const rowH = rowHRef.current;
    const viewportH = scroller ? scroller.clientHeight : window.innerHeight;
    const scrollerTop = scroller ? scroller.getBoundingClientRect().top : 0;
    const relTop = region.getBoundingClientRect().top - scrollerTop;
    const next = visibleRange(relTop, viewportH, rowH, count, overscan);
    setRange((prev) => (prev.start === next.start && prev.end === next.end ? prev : next));
  }, [count, overscan]);

  // Recompute on every render (cheap: two getBoundingClientRect + one measure).
  // This also catches layout shifts from sibling groups, which fire no scroll
  // or resize event of their own.
  useLayoutEffect(recompute);

  useLayoutEffect(() => {
    const scroller = findScrollParent(regionRef.current);
    scrollRef.current = scroller;
    const target: HTMLElement | Window = scroller ?? window;

    let raf = 0;
    const onScroll = () => {
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        recompute();
      });
    };
    target.addEventListener("scroll", onScroll, { passive: true });

    let ro: ResizeObserver | undefined;
    if (scroller && typeof ResizeObserver !== "undefined") {
      ro = new ResizeObserver(onScroll);
      ro.observe(scroller);
    }
    recompute();
    return () => {
      target.removeEventListener("scroll", onScroll);
      if (raf) cancelAnimationFrame(raf);
      ro?.disconnect();
    };
  }, [recompute]);

  const rowH = rowHRef.current;
  return {
    regionRef,
    range,
    padTop: range.start * rowH,
    padBottom: Math.max(0, count - range.end) * rowH,
  };
}
