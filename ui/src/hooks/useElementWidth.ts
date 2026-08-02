import { useEffect, useRef, useState } from "react";

// Measured inline size of an element, for toolbars that shed labels instead of
// wrapping. CSS container queries would be the natural fit, but `container-type`
// implies `contain: layout`, which makes the element a containing block for
// `position: fixed` descendants — and every dropdown in these headers is
// viewport-anchored. A ResizeObserver keeps them working.
//
// Starts at 0 ("not measured yet"); callers should treat 0 as "assume roomy" so
// the first paint isn't a flash of collapsed labels.
export function useElementWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width ?? 0;
      // Round: sub-pixel jitter from the resizer drag would otherwise re-render
      // the whole header on every mouse move.
      setWidth((prev) => (Math.abs(prev - w) < 1 ? prev : Math.round(w)));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, width] as const;
}
