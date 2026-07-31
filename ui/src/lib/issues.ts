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

// Groups are collapsed by default once an issue is finished.
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

// ── dates ───────────────────────────────────────────────────────────────────
// Dates are "YYYY-MM-DD" strings throughout (the storage format); `today`
// comes from `dateStamp(new Date())` at the call site so all of these stay
// clock-free and testable.

export function isOverdue(issue: Issue, today: string): boolean {
  return issue.due != null && issue.due < today && !isClosed(issue.status);
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
