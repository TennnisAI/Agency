import { RunInfo, pinRun } from "../api";
import { isPinned } from "../lib/runstate";
import { toastError } from "../lib/toast";
import { MenuEntry } from "./git/Menu";
import { PinIcon } from "./icons";
import { OverflowItem } from "./OverflowMenu";

// Pinning a run holds its place on the board (AGE-141, later trimmed to pin
// alone). One write and a refresh: the rank is owned backend-side.

async function togglePin(run: RunInfo, onChanged: () => void) {
  const pinned = isPinned(run);
  try {
    await pinRun(run.id, !pinned);
    onChanged();
  } catch (e) {
    toastError(e, pinned ? "Couldn't unpin this run" : "Couldn't pin this run");
  }
}

/** Menu entry for hosts that already have an overflow or right-click menu. */
export function pinItems(run: RunInfo, onChanged: () => void): OverflowItem[] {
  const pinned = isPinned(run);
  return [
    {
      label: pinned ? "Unpin" : "Pin",
      icon: <PinIcon filled={pinned} />,
      onSelect: () => togglePin(run, onChanged),
    },
  ];
}

/** The same choice as a run's right-click menu sees it. */
export function pinEntries(run: RunInfo, onChanged: () => void): MenuEntry[] {
  const pinned = isPinned(run);
  return [{ label: pinned ? "Unpin" : "Pin", onClick: () => togglePin(run, onChanged) }];
}

/**
 * The control on a run tile: a pin icon, filled when pinned, outline when not.
 * Quiet until the tile is hovered (like the close button); stays visible once
 * pinned so the board order is readable at a glance.
 */
export default function AttentionMarker({
  run,
  onChanged,
}: {
  run: RunInfo;
  onChanged: () => void;
}) {
  const pinned = isPinned(run);
  return (
    // Swallows clicks: the card behind this opens the run.
    <span className="attn-wrap" onClick={(e) => e.stopPropagation()}>
      <button
        type="button"
        className={`attn-chip${pinned ? " marked" : " quiet"}`}
        title={pinned ? "Unpin: stop holding this run at the top" : "Pin: keep this run at the top"}
        aria-pressed={pinned}
        onClick={() => togglePin(run, onChanged)}
      >
        <PinIcon size={12} filled={pinned} />
      </button>
    </span>
  );
}
