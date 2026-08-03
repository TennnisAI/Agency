import { useEffect, useRef } from "react";

// Close a popover when the window resizes. Menus, pickers and pills are placed
// once, at fixed viewport coordinates measured from their trigger; a resize
// moves the trigger without moving them, so they end up stranded mid-window
// (and can be pushed off the edge). Reflowing them mid-drag would be busier
// than useful — dismissing is what the user expects from a menu whose anchor
// walked away.
//
// Pass `open: false` while the popover is closed so nothing is listening.
export function useDismissOnResize(open: boolean, onDismiss: () => void): void {
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;

  useEffect(() => {
    if (!open) return;
    const dismiss = () => dismissRef.current();
    window.addEventListener("resize", dismiss);
    return () => window.removeEventListener("resize", dismiss);
  }, [open]);
}
