import { useRef, useState } from "react";
import { RunInfo, rankPinnedRun } from "../api";
import { isPinned, pinDropSide, planPinDrop } from "../lib/runstate";
import { toastError } from "../lib/toast";
import { useRuns } from "../store/runs";

/** What one pinned run's tile or rail row needs to take part in a drag. */
export interface PinDrag {
  /** This run is the one being dragged. */
  dragging: boolean;
  /** The dragged run would land on this side of this one. */
  over: "before" | "after" | null;
  /** Mousedown on the grab handle. */
  onGrab: (e: React.MouseEvent) => void;
}

/**
 * Drag-to-reorder for pinned runs (AGE-161), shared by the agent grid and the
 * focus rail. Pointer-based (mousedown on the grip, 5px threshold, track, commit
 * on mouseup), NOT HTML5 drag-and-drop: wry's drag-drop layer intercepts drops
 * at the NSView level on macOS, so an in-page HTML5 drag lifts but its drop
 * never fires. The pattern is IssuesView's.
 *
 * Only pinned runs join in, and only once there are two of them to order. The
 * host marks each one's element with `data-pin-run` so the pointer can find
 * what it is over, and renders `PinGrip` with `onGrab` as the handle.
 */
export function usePinDrag() {
  const { runs, refreshRuns } = useRuns();
  // Mouse handlers plan against the freshest board, not their closure's: the
  // runs refresh on a timer while the pointer is down.
  const runsRef = useRef(runs);
  runsRef.current = runs;
  const [drag, setDrag] = useState<{ id: string; overId: string } | null>(null);

  async function drop(id: string, overId: string) {
    const plan = planPinDrop(runsRef.current, id, overId);
    if (plan.length === 0) return;
    try {
      await Promise.all(plan.map((u) => rankPinnedRun(u.id, u.rank!)));
    } catch (e) {
      toastError(e, "Couldn't reorder pins");
    }
    // After a failure too: a renumbering can land partway.
    refreshRuns();
  }

  function grab(id: string, e: React.MouseEvent) {
    if (e.button !== 0) return;
    // The grip sits on a card or a row that opens the run on click; the press
    // is the drag's, not theirs. preventDefault also keeps WebKit from starting
    // a text selection, which begins at mousedown, long before the threshold.
    e.preventDefault();
    e.stopPropagation();
    const start = { x: e.clientX, y: e.clientY };
    let live: { id: string; overId: string } | null = null;
    // Escape sets this so the drag can't restart on the next mousemove; the
    // still-held button then releases as a no-op.
    let cancelled = false;

    const onMove = (ev: MouseEvent) => {
      if (cancelled) return;
      if (!live) {
        if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < 5) return;
        live = { id, overId: id };
      }
      ev.preventDefault();
      const over = (document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)
        ?.closest<HTMLElement>("[data-pin-run]");
      // Off the pins (an unpinned run, a gap in the grid) keeps the last
      // target, so the drop lands where the indicator last showed it would.
      if (over?.dataset.pinRun) live = { id, overId: over.dataset.pinRun };
      setDrag(live);
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      cancelled = true;
      setDrag(null);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("keydown", onKey, true);
      setDrag(null);
      if (!live) return;
      // A drag that ends over its own card would otherwise click it and open
      // the run. Swallowed for this one click only, cancelled or not.
      const swallow = (ev: MouseEvent) => {
        ev.stopPropagation();
        ev.preventDefault();
      };
      window.addEventListener("click", swallow, true);
      window.setTimeout(() => window.removeEventListener("click", swallow, true), 0);
      if (!cancelled) drop(live.id, live.overId);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    // Capture phase so an Escape mid-drag can't reach anything else.
    window.addEventListener("keydown", onKey, true);
  }

  const reorderable = runs.filter(isPinned).length >= 2;
  return {
    /** A drag is under way: the host shows the grabbing cursor throughout. */
    active: drag != null,
    /** The run's part in a drag, or undefined for a run that takes none. */
    pinDrag(run: RunInfo): PinDrag | undefined {
      if (!reorderable || !isPinned(run)) return undefined;
      return {
        dragging: drag?.id === run.id,
        over: drag ? pinDropSide(runs, drag.id, run.id) : null,
        onGrab: (e) => grab(run.id, e),
      };
    },
  };
}
