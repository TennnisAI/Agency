// Manual ordering within a status group (one-stop Phase 6): ranks are
// fractional floats, ascending. Dragging assigns the moved issue the midpoint
// of its new neighbors; the first drag in a partially-ranked group
// materializes ranks for everyone (in the current visual order) so a single
// midpoint can't leapfrog unranked rows; when midpoints collapse, the whole
// group renormalizes back to whole numbers.

export interface Ranked {
  id: string;
  rank: number | null;
}

/** Ranks closer than this are treated as collapsed → renormalize. */
const MIN_GAP = 1e-6;

/**
 * Plan the updates for dragging `group[from]` to visual position `to` within
 * its status group. `group` is in current visual order. Returns the minimal
 * set of `{id, rank}` updates to persist (empty for a no-op drop).
 */
export function planReorder(group: Ranked[], from: number, to: number): Ranked[] {
  if (from === to || from < 0 || to < 0 || from >= group.length || to >= group.length) return [];
  const next = [...group];
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved);

  // Any unranked member means midpoints can't order the group reliably:
  // materialize the whole new order as 1..n.
  if (group.some((g) => g.rank == null)) {
    return next.map((g, i) => ({ id: g.id, rank: i + 1 }));
  }

  const before = to > 0 ? next[to - 1].rank! : null;
  const after = to < next.length - 1 ? next[to + 1].rank! : null;
  const mid =
    before == null ? after! - 1 :
    after == null ? before + 1 :
    (before + after) / 2;
  // Collapsed gap (or a float that stopped resolving): renormalize everyone.
  if ((before != null && mid - before < MIN_GAP) || (after != null && after - mid < MIN_GAP)) {
    return next.map((g, i) => ({ id: g.id, rank: i + 1 }));
  }
  return [{ id: moved.id, rank: mid }];
}
