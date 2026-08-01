// Shared fuzzy matcher for path-shaped targets (quick-open, palette). Scores
// are ordinal only — lower is better; the absolute value carries no meaning.

const TIER = 1_000_000;
const POS = 1_000;

/**
 * Score `target` against `query`, or null when the query's characters don't
 * even appear in order. Tiers (best first): basename prefix, basename
 * substring, path substring, in-order character subsequence. Within a tier,
 * earlier matches rank first, then shorter targets.
 */
export function fuzzyScore(query: string, target: string): number | null {
  const q = query.toLowerCase();
  const t = target.toLowerCase();
  if (!q) return 0;
  const lenPart = Math.min(target.length, POS - 1);
  const name = t.slice(t.lastIndexOf("/") + 1);
  if (name.startsWith(q)) return lenPart;
  const ni = name.indexOf(q);
  if (ni >= 0) return TIER + Math.min(ni, POS - 1) * POS + lenPart;
  const pi = t.indexOf(q);
  if (pi >= 0) return 2 * TIER + Math.min(pi, POS - 1) * POS + lenPart;
  let qi = 0;
  let first = -1;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      if (qi === 0) first = ti;
      qi++;
    }
  }
  if (qi < q.length) return null;
  return 3 * TIER + Math.min(first, POS - 1) * POS + lenPart;
}

/** Rank-sorted matches, capped at `limit`. Empty query = head of the list. */
export function fuzzyFilter<T>(query: string, items: T[], key: (item: T) => string, limit = 50): T[] {
  const q = query.trim();
  if (!q) return items.slice(0, limit);
  const scored: { item: T; score: number; i: number }[] = [];
  for (let i = 0; i < items.length; i++) {
    const score = fuzzyScore(q, key(items[i]));
    if (score !== null) scored.push({ item: items[i], score, i });
  }
  scored.sort((a, b) => a.score - b.score || a.i - b.i);
  return scored.slice(0, limit).map((s) => s.item);
}
