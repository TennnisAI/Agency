export interface PaletteEntry {
  kind: "project" | "run" | "action";
  id: string;
  projectId: string;
  label: string;
  sublabel: string;
}

export function filterEntries(query: string, entries: PaletteEntry[]): PaletteEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return entries;
  const scored: { e: PaletteEntry; score: number }[] = [];
  for (const e of entries) {
    const label = e.label.toLowerCase();
    const sub = e.sublabel.toLowerCase();
    let score = -1;
    if (label.startsWith(q)) score = 0;
    else if (label.includes(q)) score = 1;
    else if (sub.includes(q)) score = 2;
    if (score >= 0) scored.push({ e, score });
  }
  // Stable sort by score (lower = better); preserve input order within a score.
  return scored
    .map((s, i) => ({ ...s, i }))
    .sort((a, b) => (a.score - b.score) || (a.i - b.i))
    .map((s) => s.e);
}
