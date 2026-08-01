// Daily-note path/date logic for the workspace journal. Pure — all file IO
// stays with the callers (App's open-daily-note flow, DocsEditor navigation).

export const JOURNAL_DIR = "journal";
export const DAILY_TEMPLATE_PATH = "templates/daily.md";

const DAILY_RE = /^journal\/\d{4}-\d{2}-\d{2}\.md$/;

const pad = (n: number) => String(n).padStart(2, "0");

/** Local-date stamp, YYYY-MM-DD. */
export function dateStamp(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Workspace-relative path of the daily note for `d`: journal/YYYY-MM-DD.md */
export function dailyNotePath(d: Date): string {
  return `${JOURNAL_DIR}/${dateStamp(d)}.md`;
}

export function isDailyNotePath(path: string): boolean {
  return DAILY_RE.test(path);
}

/**
 * The nearest *existing* daily note before/after `current`, or null at the
 * ends. Navigation never creates files — only ⌘⇧D creates (today's note).
 * Date stamps sort lexically, so string order is date order.
 */
export function adjacentDailyPath(
  paths: Iterable<string>,
  current: string,
  dir: "prev" | "next",
): string | null {
  if (!isDailyNotePath(current)) return null;
  const daily = [...paths].filter((p) => DAILY_RE.test(p)).sort();
  if (dir === "prev") {
    const before = daily.filter((p) => p < current);
    return before.length ? before[before.length - 1] : null;
  }
  const after = daily.filter((p) => p > current);
  return after.length ? after[0] : null;
}

/** Fill a `templates/daily.md` template: `{{date}}` → YYYY-MM-DD. */
export function renderDailyTemplate(template: string, d: Date): string {
  return template.split("{{date}}").join(dateStamp(d));
}

/** Content for a fresh daily note when no template exists. */
export function defaultDailyContent(d: Date): string {
  return `# ${dateStamp(d)}\n\n`;
}
