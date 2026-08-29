// Fixed-position popovers (menus, pickers) are placed by measuring their
// trigger and handing the result to `style`, so an ancestor's overflow can't
// clip them. These are those coordinates: one horizontal and one vertical edge,
// whichever pair keeps the popover on screen.
export type MenuCoords = { top?: number; bottom?: number; left?: number; right?: number };

/** Which way a popover opens when there is room to go either way. */
export type MenuSide = "right" | "left";

/** Breathing room kept between a popover and the window edge. */
const EDGE_GAP = 8;
/** Distance from the trigger, matching every other menu in the app. */
const OFFSET = 4;

// Where a popover of the given size hangs off its trigger. It hangs off the
// trigger's bottom-left corner and opens down and to the right, but a trigger
// near an edge flips it: right-anchored (opening leftward) when the popover
// would run past the right edge, bottom-anchored (opening upward) when it
// wouldn't clear the bottom. `opens: "left"` reverses the horizontal
// preference, for a trigger that sits at the right of its own pane and should
// cover that pane rather than its neighbour; it still flips back if the popover
// would run past the left edge.
//
// The sizes are the popover's, and the caller is expected to have measured the
// rendered element (see useMenuAnchor) wherever the content decides the height.
export function anchorMenu(
  r: DOMRect,
  width: number,
  height: number,
  opens: MenuSide = "right",
): MenuCoords {
  const roomRight = r.left + width <= window.innerWidth - EDGE_GAP;
  const roomLeft = r.right - width >= EDGE_GAP;
  const openLeft = opens === "left" ? roomLeft : !roomRight;
  const fitsBelow = r.bottom + OFFSET + height <= window.innerHeight - EDGE_GAP;
  return {
    ...(fitsBelow ? { top: r.bottom + OFFSET } : { bottom: window.innerHeight - r.top + OFFSET }),
    ...(openLeft ? { right: window.innerWidth - r.right } : { left: r.left }),
  };
}
