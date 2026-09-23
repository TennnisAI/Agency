// The in-page drag every tree and board shares: mousedown, a 5px threshold,
// window listeners to track and commit on mouseup, Escape to cancel. Pointer
// events, NOT HTML5 drag-and-drop: wry's drag-drop layer intercepts drops at
// the NSView level on macOS, so an in-page HTML5 drag lifts but its drop never
// fires.
//
// One copy, because four had drifted: two ways of suppressing the click, and
// the same Escape leak in all of them. Their keydown listener was registered
// in the capture phase "so an Escape mid-drag can't reach anything else", but
// never stopped the event, so the Escape that cancelled a drag also reached
// the focused terminal and interrupted the agent mid-turn.

export interface PointerDragHandlers {
  /** The press crossed the threshold and is a drag now. */
  onStart?: (ev: MouseEvent) => void;
  /** Every move once the drag has started, the first one included. */
  onMove: (ev: MouseEvent) => void;
  /** Escape: clear whatever the drag shows. The release that follows is a no-op. */
  onCancel: () => void;
  /** Released after a drag that was not cancelled. */
  onDrop: (ev: MouseEvent) => void;
}

/** Below this many pixels (Manhattan) from the press, it is still a click. */
const THRESHOLD = 5;

/**
 * Track the press `e` as a potential drag. The caller has already decided the
 * press is one (left button, not on a control) and done its own mousedown
 * work; this calls `preventDefault` on it, which keeps WebKit from starting a
 * text selection at mousedown, long before the threshold.
 *
 * A drag that started swallows the click its release would otherwise fire,
 * cancelled or not: it must not open or toggle the row it ends over.
 */
export function startPointerDrag(e: React.MouseEvent, handlers: PointerDragHandlers) {
  e.preventDefault();
  const start = { x: e.clientX, y: e.clientY };
  let started = false;
  // Escape sets this so the drag can't restart on the next mousemove; the
  // still-held button then releases as a no-op.
  let cancelled = false;

  const onMove = (ev: MouseEvent) => {
    if (cancelled) return;
    if (!started) {
      if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < THRESHOLD) return;
      started = true;
      window.getSelection()?.removeAllRanges();
      handlers.onStart?.(ev);
    }
    ev.preventDefault();
    handlers.onMove(ev);
  };
  const onKey = (ev: KeyboardEvent) => {
    if (ev.key !== "Escape") return;
    // Stopped, not just observed: the focused terminal is still listening.
    ev.preventDefault();
    ev.stopPropagation();
    if (cancelled) return;
    cancelled = true;
    handlers.onCancel();
  };
  const onUp = (ev: MouseEvent) => {
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    window.removeEventListener("keydown", onKey, true);
    if (!started) return;
    const swallow = (click: MouseEvent) => {
      click.stopPropagation();
      click.preventDefault();
    };
    window.addEventListener("click", swallow, true);
    window.setTimeout(() => window.removeEventListener("click", swallow, true), 0);
    if (!cancelled) handlers.onDrop(ev);
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
  // Capture phase, so it runs before the terminal's own key handling.
  window.addEventListener("keydown", onKey, true);
}
