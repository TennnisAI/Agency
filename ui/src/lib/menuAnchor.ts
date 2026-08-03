// Fixed-position popovers (menus, pickers) are placed by measuring their
// trigger and handing the result to `style`, so an ancestor's overflow can't
// clip them. These are those coordinates: one horizontal and one vertical edge,
// whichever pair keeps the popover on screen.
export type MenuCoords = { top?: number; bottom?: number; left?: number; right?: number };

/** Breathing room kept between a popover and the window edge. */
const EDGE_GAP = 8;
/** Distance from the trigger, matching every other menu in the app. */
const OFFSET = 4;

// Where a popover of the given size hangs off its trigger. Anchoring is by
// default top-left of the trigger's bottom-left corner, but a trigger near an
// edge flips it: right-anchored (opening leftward) when the popover would run
// past the right edge, bottom-anchored (opening upward) when it wouldn't clear
// the bottom. Sizes are the popover's CSS width/height — measuring the real
// element first would need a hidden pass, and these are fixed by stylesheet.
export function anchorMenu(r: DOMRect, width: number, height: number): MenuCoords {
  const fitsRight = r.left + width <= window.innerWidth - EDGE_GAP;
  const fitsBelow = r.bottom + OFFSET + height <= window.innerHeight - EDGE_GAP;
  return {
    ...(fitsBelow ? { top: r.bottom + OFFSET } : { bottom: window.innerHeight - r.top + OFFSET }),
    ...(fitsRight ? { left: r.left } : { right: window.innerWidth - r.right }),
  };
}
