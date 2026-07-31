// Weekly review note (one-stop Phase 8): pure ISO-week math and the markdown
// assembly for journal/weekly/2026-W31.md. All data comes from local reads
// (git log, the issue index, archived runs); the callers gather, this renders.

export const WEEKLY_DIR = "journal/weekly";

/** ISO week (Monday-start; the week's Thursday decides the year). */
export function isoWeek(d: Date): { year: number; week: number } {
  const t = new Date(d.getFullYear(), d.getMonth(), d.getDate());
  const day = (t.getDay() + 6) % 7; // Mon=0 .. Sun=6
  t.setDate(t.getDate() - day + 3); // this week's Thursday
  const year = t.getFullYear();
  const jan4 = new Date(year, 0, 4);
  const week1Mon = new Date(year, 0, 4 - ((jan4.getDay() + 6) % 7));
  // Round absorbs DST hour offsets between the two local dates.
  const week = 1 + Math.round(((t.getTime() - week1Mon.getTime()) / 86400000 - 3) / 7);
  return { year, week };
}

/** "2026-W31" for the ISO week containing `d`. */
export function isoWeekStamp(d: Date): string {
  const { year, week } = isoWeek(d);
  return `${year}-W${String(week).padStart(2, "0")}`;
}

/** Monday 00:00 local of `d`'s ISO week — the report window's start. */
export function isoWeekStart(d: Date): Date {
  const day = (d.getDay() + 6) % 7;
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() - day);
}

export function weeklyNotePath(d: Date): string {
  return `${WEEKLY_DIR}/${isoWeekStamp(d)}.md`;
}

export interface WeeklyData {
  /** Per-project merge subjects, in the order projects should appear. */
  merges: { project: string; subjects: string[] }[];
  /** Issues that reached done/cancelled this week; label is "AGE-14". */
  issuesClosed: { label: string; title: string }[];
  /** Agent runs archived this week. */
  runsArchived: { id: string; title: string; project: string }[];
}

/**
 * Render the weekly note. Issue and run lines use Phase 7 wikilink syntax
 * ([[AGE-14]], [[run:id|title]]) so every entry resolves and backlinks.
 * Empty sections are omitted; an empty week still gets the header and a
 * Notes section for narration.
 */
export function buildWeeklyNote(stamp: string, data: WeeklyData): string {
  const out: string[] = [`# Week ${stamp}`, ""];
  const merged = data.merges.filter((m) => m.subjects.length > 0);
  if (merged.length > 0) {
    out.push("## Merged", "");
    for (const m of merged) {
      out.push(`### ${m.project}`, "");
      for (const s of m.subjects) out.push(`- ${s}`);
      out.push("");
    }
  }
  if (data.issuesClosed.length > 0) {
    out.push("## Issues closed", "");
    for (const i of data.issuesClosed) out.push(`- [[${i.label}]] ${i.title}`);
    out.push("");
  }
  if (data.runsArchived.length > 0) {
    out.push("## Agent runs archived", "");
    for (const r of data.runsArchived) out.push(`- [[run:${r.id}|${r.title}]] (${r.project})`);
    out.push("");
  }
  out.push("## Notes", "");
  return out.join("\n");
}
