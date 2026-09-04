import { RunInfo, RunStanding, pinRun, setRunStanding } from "../api";
import { SNOOZE_PRESETS, fmtWake, isPinned, standingTitle } from "../lib/runstate";
import { toastError } from "../lib/toast";
import OverflowMenu, { OverflowItem } from "./OverflowMenu";
import { MenuEntry } from "./git/Menu";
import { PinIcon, SettleIcon, SnoozeIcon } from "./icons";

// Saying what you want done with a run, instead of leaving it to be guessed
// (AGE-141). The board answers "which of these needs me", and the honest
// answer to that is the one the person gave: settle a run you have dealt with,
// snooze one you will get to later, pin one that should stay where you put it.
// Only where nobody has said anything does the clock still decide.
//
// Every action is one write and then a refresh: the standing is derived
// backend-side from the record plus the pane, so there is no local state to
// keep in step with it.

/**
 * The writes behind the standing entries, plus the labels that depend on what
 * the run's standing is now. Shared so the overflow list and the right-click
 * menu offer the same choices under the same names wherever a run is listed.
 */
function attentionActions(run: RunInfo, onChanged: () => void) {
  const standing = run.attention.standing;
  const pinned = isPinned(run);
  const set = async (next: RunStanding | null) => {
    try {
      await setRunStanding(run.id, next);
      onChanged();
    } catch (e) {
      toastError(e, "Couldn't record that");
    }
  };
  // "Unsettle" and "Wake now" are the same instruction — put this back in
  // front of me — and both land on `active` rather than clearing the record,
  // so the run keeps its place instead of dropping back to the clock.
  const suppressed = standing?.kind === "settled" || standing?.kind === "snoozed";
  return {
    settleLabel: suppressed ? (standing?.kind === "settled" ? "Unsettle" : "Wake now") : "Settle",
    settle: () => set(suppressed ? { kind: "active" } : { kind: "settled" }),
    snooze: (ms: number) => set({ kind: "snoozed", untilMs: Date.now() + ms }),
    pinLabel: pinned ? "Unpin" : "Pin",
    pin: async () => {
      try {
        await pinRun(run.id, !pinned);
        onChanged();
      } catch (e) {
        toastError(e, pinned ? "Couldn't unpin this run" : "Couldn't pin this run");
      }
    },
  };
}

/**
 * The menu entries for one run, for hosts that already have an overflow menu
 * (the focus header). `onChanged` refreshes whatever list the host is showing.
 */
export function attentionItems(run: RunInfo, onChanged: () => void): OverflowItem[] {
  const a = attentionActions(run, onChanged);
  const items: OverflowItem[] = [];
  // A terminal is never waiting on you, so settling and snoozing have nothing
  // to act on. It can still be pinned: that is about where it sits.
  if (run.kind === "agent") {
    items.push({ label: a.settleLabel, icon: <SettleIcon />, onSelect: a.settle });
    SNOOZE_PRESETS.forEach((preset, i) => {
      items.push({
        label: `Snooze ${preset.label}`,
        icon: <SnoozeIcon />,
        separator: i === 0,
        onSelect: () => a.snooze(preset.ms),
      });
    });
  }
  items.push({ label: a.pinLabel, icon: <PinIcon />, separator: items.length > 0, onSelect: a.pin });
  return items;
}

/**
 * The same choices as a run's right-click menu sees them. The presets go in a
 * submenu rather than three more rows: that menu already carries the run's whole
 * vocabulary, and the snooze durations are the part nobody scans for.
 */
export function attentionEntries(run: RunInfo, onChanged: () => void): MenuEntry[] {
  const a = attentionActions(run, onChanged);
  const entries: MenuEntry[] = [];
  if (run.kind === "agent") {
    entries.push(
      { label: a.settleLabel, onClick: a.settle },
      {
        kind: "submenu",
        label: "Snooze",
        items: SNOOZE_PRESETS.map((preset) => ({
          label: preset.label,
          onClick: () => a.snooze(preset.ms),
        })),
      },
    );
  }
  entries.push({ label: a.pinLabel, onClick: a.pin });
  return entries;
}

/**
 * The control on a run tile. It carries only what the status line beside it
 * cannot: a pin, which has no other home on the card, and a snooze's wake time
 * in clock terms (the line counts down instead). Settled and active both read
 * in that line already, so the control stays a quiet affordance for them
 * rather than saying it twice with an icon that could only mean one of the
 * two. It is revealed by hovering the tile, like the close button.
 */
export default function AttentionMarker({
  run,
  onChanged,
}: {
  run: RunInfo;
  onChanged: () => void;
}) {
  const standing = run.attention.standing;
  const pinned = isPinned(run);
  const marked = pinned || standing?.kind === "snoozed";
  const title = pinned
    ? `Pinned, so it keeps its place here. ${standing ? standingTitle(standing) : ""}`.trim()
    : standing
      ? standingTitle(standing)
      : run.kind === "agent"
        ? "Settle, snooze or pin this agent"
        : "Pin this terminal";
  // A pin outranks the standing on the chip: it is the one of the two that is
  // otherwise invisible.
  const face = pinned ? (
    <>
      <PinIcon size={12} />
      <span className="attn-label">pinned</span>
    </>
  ) : standing?.kind === "snoozed" ? (
    <>
      <SnoozeIcon size={12} />
      <span className="attn-label">{fmtWake(standing.untilMs)}</span>
    </>
  ) : (
    <SettleIcon size={12} />
  );
  return (
    // Swallows clicks: the card behind this opens the run.
    <span className="attn-wrap" onClick={(e) => e.stopPropagation()}>
      <OverflowMenu
        buttonClass={`attn-chip${marked ? " marked" : " quiet"}`}
        icon={face}
        title={title}
        items={attentionItems(run, onChanged)}
      />
    </span>
  );
}
