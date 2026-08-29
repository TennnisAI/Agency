import { RefObject, useLayoutEffect, useState } from "react";
import { MenuCoords, MenuSide, anchorMenu } from "../lib/menuAnchor";

// Places an open popover against the rect its trigger had when it opened, from
// the popover's own measured size rather than a guess at it.
//
// The guess is what broke: the agent menu is as tall as the number of agent
// profiles plus a branch picker, so no constant describes it, and every caller
// simply hung it below the trigger. Start an agent from an issue row near the
// bottom of the list and the menu ran off the screen with the agents cut off
// (AGE-173).
//
// Measuring happens in a layout effect, so the corrected position is the first
// one painted, and again through a ResizeObserver, because the menu's content
// arrives after it opens: the branch list is fetched, and the agent list is
// re-read on every open. A menu that grows while sitting near the bottom edge
// flips up as it crosses it.
//
// Pass `rect: null` while the popover is closed. A resize is not handled here:
// the trigger moves out from under a popover placed this way, and the app
// dismisses it instead (see useDismissOnResize).
export function useMenuAnchor(
  rect: DOMRect | null,
  menu: RefObject<HTMLElement | null>,
  opens: MenuSide = "right",
): MenuCoords | undefined {
  const [coords, setCoords] = useState<MenuCoords>();

  useLayoutEffect(() => {
    if (!rect) {
      setCoords(undefined);
      return;
    }
    const el = menu.current;
    // No element yet (the first pass of a caller that renders the menu only
    // once it has coords) still gets a placement, from a zero size: below and
    // to the right of the trigger, which is where it would have gone anyway.
    const place = () => {
      const box = el?.getBoundingClientRect();
      setCoords(anchorMenu(rect, box?.width ?? 0, box?.height ?? 0, opens));
    };
    place();
    if (!el) return;
    const ro = new ResizeObserver(place);
    ro.observe(el);
    return () => ro.disconnect();
  }, [rect, opens]);

  return coords;
}
