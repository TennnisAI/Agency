/**
 * Which surface ⌘F belongs to right now.
 *
 * The Edit menu owns the ⌘F accelerator, and a native menu click carries no
 * DOM target at all — so "find" can't be a key handler sitting on one
 * component. Instead every surface that can be searched registers itself here
 * while it's mounted, and Find / Find and Replace ask this module to pick one.
 *
 * The pick is deliberately boring: a target has to be on screen, focus inside
 * it wins over everything else, and `rank` breaks the tie between two visible
 * surfaces (in the Issues tab the list filter outranks the description editor,
 * because a board you're reading is the thing you meant to search — but click
 * into the description first and focus overrides that).
 */

import { FindMode } from "./find";

export interface FindTarget {
  /**
   * The element this target owns. Decides both "is it on screen" and "does it
   * hold focus", so it must contain the find bar as well as the content —
   * otherwise typing in the bar would hand ⌘F to a different surface.
   */
  host: () => HTMLElement | null;
  /** Open (or re-focus) this surface's find bar. */
  open: (mode: FindMode) => void;
  /** Step to the next/previous match with the bar closed (Edit ▸ Find Next). */
  step?: (back: boolean) => void;
  /** False for surfaces where replacing means nothing (list filters, terminals). */
  canReplace: boolean;
  /** Tie-break between two visible targets; higher wins. See [`FindRank`]. */
  rank: number;
  /**
   * Never claim ⌘F unless focus is inside. Terminals set this: several are on
   * screen at once in the agent grid, and "the one you clicked" is the only
   * honest answer to which should search.
   */
  requiresFocus?: boolean;
}

/** The standing order between surfaces that are visible at the same time. */
export const FindRank = {
  /** The issue board's filter, beside an open description. */
  list: 30,
  /** A text editor: notes, files, the issue description. */
  editor: 20,
  /** A terminal in a side panel, beside an editor. */
  terminal: 10,
} as const;

/** Focus beats rank outright — clicking into a surface is an explicit choice. */
const FOCUS_BONUS = 1000;

const targets = new Set<FindTarget>();

/** Register while mounted. Returns the unregister for the effect's cleanup. */
export function registerFindTarget(target: FindTarget): () => void {
  targets.add(target);
  return () => {
    targets.delete(target);
  };
}

function onScreen(el: HTMLElement): boolean {
  // Inactive editor tabs are `display: none`, which zeroes all three.
  return !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
}

function score(target: FindTarget): number {
  const el = target.host();
  if (!el || !onScreen(el)) return -1;
  const focused = el.contains(document.activeElement);
  if (target.requiresFocus && !focused) return -1;
  return target.rank + (focused ? FOCUS_BONUS : 0);
}

function best(pool: Iterable<FindTarget>): FindTarget | null {
  let winner: FindTarget | null = null;
  let top = -1;
  for (const target of pool) {
    const s = score(target);
    if (s > top) {
      top = s;
      winner = target;
    }
  }
  return winner;
}

/**
 * The surface ⌘F should act on, or null when nothing on screen can be searched.
 * Replace prefers a surface that can actually replace, but falls back rather
 * than doing nothing — Edit ▸ Find and Replace over an issue board still opens
 * the board's filter.
 */
export function resolveFindTarget(mode: FindMode): FindTarget | null {
  if (mode === "replace") {
    const replaceable = best([...targets].filter((t) => t.canReplace));
    if (replaceable) return replaceable;
  }
  return best(targets);
}

/** Open the right find bar. False when there's nothing to search. */
export function requestFind(mode: FindMode): boolean {
  const target = resolveFindTarget(mode);
  if (!target) return false;
  target.open(mode === "replace" && !target.canReplace ? "find" : mode);
  return true;
}

/** Edit ▸ Find Next / Find Previous. False when the surface can't step. */
export function requestFindStep(back: boolean): boolean {
  const target = resolveFindTarget("find");
  if (!target?.step) return false;
  target.step(back);
  return true;
}

/** Test seam: drop every registration. */
export function resetFindTargets(): void {
  targets.clear();
}
