import { Issue, IssueStatus, Project } from "../api";

// Workflow order; the board renders groups in this order.
export const ISSUE_STATUSES: IssueStatus[] = [
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "done",
  "cancelled",
];

export const STATUS_LABELS: Record<IssueStatus, string> = {
  backlog: "Backlog",
  todo: "Todo",
  in_progress: "In Progress",
  in_review: "In Review",
  done: "Done",
  cancelled: "Cancelled",
};

// Theme accent each status pill/dot uses (CSS var names, like project colors).
export const STATUS_COLORS: Record<IssueStatus, string> = {
  backlog: "o0",
  todo: "o1",
  in_progress: "blue",
  in_review: "mauve",
  done: "green",
  cancelled: "o0",
};

// Index = priority value (0 none · 1 low · 2 medium · 3 high · 4 urgent).
export const PRIORITY_LABELS = ["No priority", "Low", "Medium", "High", "Urgent"] as const;

// "AGE-14" — the project's stored key plus the per-project number.
export function issueLabel(project: Pick<Project, "issue_key"> | null, issue: Pick<Issue, "seq">): string {
  return `${project?.issue_key ?? "ISSUE"}-${issue.seq}`;
}

export function isClosed(status: IssueStatus): boolean {
  return status === "done" || status === "cancelled";
}

// Board order within a status group: manual rank first (ascending, ranked
// before unranked), then urgent first, then oldest first.
export function compareIssues(a: Issue, b: Issue): number {
  if (a.rank != null || b.rank != null) {
    if (a.rank == null) return 1;
    if (b.rank == null) return -1;
    if (a.rank !== b.rank) return a.rank - b.rank;
  }
  if (a.priority !== b.priority) return b.priority - a.priority;
  return a.createdAt - b.createdAt || a.seq - b.seq;
}

// ── search & filters ────────────────────────────────────────────────────────
// Shared by both boards (the per-project Issues tab and the cross-project home
// overview) so a query behaves the same wherever it's typed.

// "all" shows every status, "open" hides done/cancelled, anything else pins
// one status.
export type StatusFilter = "all" | "open" | IssueStatus;

// Sort within whatever grouping the board applies. "board" is the default:
// workflow order, then the manual rank / priority / age chain.
export type IssueSort = "board" | "due" | "updated";

export interface IssueFilters {
  // Search terms from `searchTerms()` — every one of them must match.
  terms: string[];
  status: StatusFilter;
  // -1 = any priority.
  priority: number;
}

export const NO_FILTERS: IssueFilters = { terms: [], status: "all", priority: -1 };

// Whitespace-separated terms, lowercased. Multiple terms read as AND, so
// "auth flake" finds the issue that mentions both in any order.
export function searchTerms(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

export function filtersActive(f: IssueFilters): boolean {
  return f.terms.length > 0 || f.status !== "all" || f.priority >= 0;
}

// `label` is the issue's display key ("AGE-14") so searching for it — or for
// the bare number — finds the issue.
export function matchesFilters(issue: Issue, label: string, f: IssueFilters): boolean {
  if (f.status === "open" ? isClosed(issue.status) : f.status !== "all" && issue.status !== f.status) return false;
  if (f.priority >= 0 && issue.priority !== f.priority) return false;
  if (f.terms.length === 0) return true;
  // The thread is searched with the issue: half of what a tracker knows about
  // a ticket ends up in its discussion, and "where did we talk about X" is the
  // same question as "which issue is about X".
  const comments = issue.comments.map((c) => `${c.author}\n${c.body}`).join("\n");
  const hay = `${label}\n${issue.title}\n${issue.body}\n${comments}`.toLowerCase();
  return f.terms.every((t) => hay.includes(t));
}

// Edit ▸ Find Next / Find Previous over the board. ⌘F there narrows the list
// instead of scanning text, so the honest reading of "next match" is the next
// row the filter left standing. Wraps, the way a find does, where the arrow
// keys clamp at the ends; a selection the filter has hidden (or none at all)
// starts from the top, or the bottom stepping back. Null on an empty board.
export function stepSelection(
  visible: Pick<Issue, "id">[],
  selectedId: string | null,
  back: boolean,
): string | null {
  if (visible.length === 0) return null;
  const at = visible.findIndex((i) => i.id === selectedId);
  if (at < 0) return (back ? visible[visible.length - 1] : visible[0]).id;
  return visible[(at + (back ? -1 : 1) + visible.length) % visible.length].id;
}

// Ranges of `text` covered by any search term, merged and left-to-right — the
// highlight behind a matching title. Case-insensitive; empty when nothing hits.
export function matchRanges(text: string, terms: string[]): [number, number][] {
  const hay = text.toLowerCase();
  const spans: [number, number][] = [];
  for (const term of terms) {
    if (!term) continue;
    for (let at = hay.indexOf(term); at >= 0; at = hay.indexOf(term, at + 1)) {
      spans.push([at, at + term.length]);
    }
  }
  spans.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const merged: [number, number][] = [];
  for (const [start, end] of spans) {
    const last = merged[merged.length - 1];
    if (last && start <= last[1]) last[1] = Math.max(last[1], end);
    else merged.push([start, end]);
  }
  return merged;
}

// The board's sort comparator. Status order leads so a flat cross-project list
// still reads in workflow order; inside a status group that term is a no-op.
export function issueSorter(sort: IssueSort): (a: Issue, b: Issue) => number {
  if (sort === "due") {
    return (a, b) => {
      const da = a.due ?? "9999";
      const db = b.due ?? "9999";
      if (da !== db) return da < db ? -1 : 1;
      return compareIssues(a, b);
    };
  }
  if (sort === "updated") return (a, b) => b.updatedAt - a.updatedAt;
  return (a, b) => ISSUE_STATUSES.indexOf(a.status) - ISSUE_STATUSES.indexOf(b.status) || compareIssues(a, b);
}

// ── dates ───────────────────────────────────────────────────────────────────
// Dates are "YYYY-MM-DD" strings throughout (the storage format); `today`
// comes from `dateStamp(new Date())` at the call site so all of these stay
// clock-free and testable.

export function isOverdue(issue: Issue, today: string): boolean {
  return issue.due != null && issue.due < today && !isClosed(issue.status);
}

// "Aug 6 at 03:43 PM" — an instant (epoch seconds), in the user's locale and
// zone. For the things that happen at a time rather than on a day: when an
// issue was created, when a comment was posted.
export function fmtStamp(secs: number): string {
  return new Date(secs * 1000).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}

// "Aug 1", with the year appended when it isn't `today`'s. Invalid input
// (hand-edited files can hold anything) falls back to the raw string.
export function fmtDate(date: string, today: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!m) return date;
  const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const label = `${MONTHS[Number(m[2]) - 1] ?? m[2]} ${Number(m[3])}`;
  return m[1] === today.slice(0, 4) ? label : `${label} ${m[1]}`;
}

// The "what should I do today" selection: open issues due or scheduled by
// today, plus everything already in progress. Overdue-first (earliest due),
// then the normal board compare.
export function todayIssues(issues: Issue[], today: string): Issue[] {
  return issues
    .filter(
      (i) =>
        !isClosed(i.status) &&
        ((i.due != null && i.due <= today) ||
          (i.scheduled != null && i.scheduled <= today) ||
          i.status === "in_progress"),
    )
    .sort((a, b) => {
      const da = a.due ?? "9999";
      const db = b.due ?? "9999";
      if (da !== db) return da < db ? -1 : 1;
      return compareIssues(a, b);
    });
}

// Handoff from the home overview to a project's Issues tab: the overview
// stashes the clicked issue id here and IssuesView selects it once its own
// list loads (project selection resets tab state, so props can't carry it).
export const PENDING_ISSUE_KEY = "issues:pending-select";

// Same handoff trick for the palette's "New Issue": the Issues tab focuses
// quick-add when this flag is waiting.
export const PENDING_QUICKADD_KEY = "issues:focus-quickadd";

// The board remembers per project whether the detail pane is expanded (the
// list compressed to a sidebar). Per-project because how much room the detail
// wants depends on how long that tracker's descriptions run. Stored beside the
// other pane-layout prefs (loadFold/saveFold prefix these with "pane:").
export function issuesExpandedKey(projectId: string): string {
  return `issues-expanded:${projectId}`;
}

// Every status group folds, not just the finished ones — a long backlog buries
// the work in flight just as thoroughly as a long Done list does. Only the
// finished ones start folded, which is where the board began.
export const DEFAULT_COLLAPSED: IssueStatus[] = ISSUE_STATUSES.filter(isClosed);

// Which groups are folded, remembered per project (same reasoning as the
// expanded layout: a tracker's shape is its own).
export function issuesCollapsedKey(projectId: string): string {
  return `issues-collapsed:${projectId}`;
}

// Stored as a comma-joined status list beside the other pane prefs. Nothing
// stored means a first visit, not "everything open"; unknown names are dropped
// so a renamed status can't fold a group nobody can reach.
export function loadCollapsed(storage: Pick<Storage, "getItem">, key: string): Set<IssueStatus> {
  const raw = storage.getItem("pane:" + key);
  if (raw === null) return new Set(DEFAULT_COLLAPSED);
  return new Set(ISSUE_STATUSES.filter((s) => raw.split(",").includes(s)));
}

export function saveCollapsed(
  storage: Pick<Storage, "setItem">,
  key: string,
  collapsed: Set<IssueStatus>,
): void {
  storage.setItem("pane:" + key, ISSUE_STATUSES.filter((s) => collapsed.has(s)).join(","));
}

// The issue whose detail pane was open, remembered per project: leaving the
// tab to look at an agent, a diff or a note and coming back should return to
// the ticket being read, not to a blank pane. Stored beside the layout prefs
// above, so a board comes back in one piece — this issue, in the layout it
// was being read in.
export function issuesSelectedKey(projectId: string): string {
  return `issues-selected:${projectId}`;
}

// A stored selection is only honored while its issue is still there: it can
// be deleted from another window, or its file removed by hand, between two
// visits to the board.
export function loadSelected(
  storage: Pick<Storage, "getItem">,
  key: string,
  issues: Pick<Issue, "id">[],
): string | null {
  const raw = storage.getItem("pane:" + key);
  return raw !== null && issues.some((i) => i.id === raw) ? raw : null;
}

// Nothing open clears the key rather than storing a blank, so a closed pane
// and a first visit read the same on the way back in.
export function saveSelected(
  storage: Pick<Storage, "setItem" | "removeItem">,
  key: string,
  id: string | null,
): void {
  if (id === null) storage.removeItem("pane:" + key);
  else storage.setItem("pane:" + key, id);
}
