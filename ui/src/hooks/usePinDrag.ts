import { useMemo, useRef, useState } from "react";
import { RunInfo, movePinnedRun } from "../api";
import { pinDropSide, pinIndex } from "../lib/runstate";
import { startPointerDrag } from "../lib/pointerDrag";
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

type Target = { id: string; overId: string };

/**
 * Drag-to-reorder for pinned runs (AGE-161), shared by the agent grid and the
 * focus rail, on the shared pointer drag (`lib/pointerDrag`).
 *
 * Only pinned runs join in, and only once there are two of them to order. The
 * host marks each one's element with `data-pin-run` so the pointer can find
 * what it is over, and renders `PinGrip` with `onGrab` as the handle. The drop
 * sends the pair to the backend, which plans the ranks against every pin
 * (archived ones too) and writes them in one transaction.
 */
export function usePinDrag() {
  const { runs, refreshRuns, applyPinRanks } = useRuns();
  // Mouse events outrun renders, so the live target is a ref, and `drag`
  // only mirrors it when the target changes. Set on every mousemove, it
  // re-rendered the whole host (the focus view's terminals included) dozens
  // of times a second while the pointer sat on one tile.
  const live = useRef<Target | null>(null);
  const [drag, setDrag] = useState<Target | null>(null);
  const show = (next: Target | null) => {
    live.current = next;
    setDrag(next);
  };

  async function drop(target: Target) {
    try {
      // The indicator stays up until the new ranks are in the store. Cleared
      // at mouseup, the tile snapped back to its old place for the round trip
      // and read as a drop that had not taken.
      applyPinRanks(await movePinnedRun(target.id, target.overId));
    } catch (e) {
      toastError(e, "Couldn't reorder pins");
    } finally {
      show(null);
    }
    refreshRuns();
  }

  function grab(id: string, e: React.MouseEvent) {
    if (e.button !== 0) return;
    // The grip sits on a card or a row that opens the run on click; the press
    // is the drag's, not theirs.
    e.stopPropagation();
    startPointerDrag(e, {
      onStart: () => show({ id, overId: id }),
      onMove: (ev) => {
        const over = (document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)
          ?.closest<HTMLElement>("[data-pin-run]")?.dataset.pinRun;
        // Off the pins (an unpinned run, a gap in the grid) keeps the last
        // target, so the drop lands where the indicator last showed it would.
        if (over && over !== live.current?.overId) show({ id, overId: over });
      },
      onCancel: () => show(null),
      onDrop: () => {
        const target = live.current;
        if (!target || target.overId === target.id) return show(null);
        void drop(target);
      },
    });
  }

  const index = useMemo(() => pinIndex(runs), [runs]);
  const reorderable = index.size >= 2;
  return {
    /** A drag is under way: the host shows the grabbing cursor throughout. */
    active: drag != null,
    /** The run's part in a drag, or undefined for a run that takes none. */
    pinDrag(run: RunInfo): PinDrag | undefined {
      if (!reorderable || !index.has(run.id)) return undefined;
      return {
        dragging: drag?.id === run.id,
        over: drag ? pinDropSide(index, drag.id, run.id) : null,
        onGrab: (e) => grab(run.id, e),
      };
    },
  };
}
