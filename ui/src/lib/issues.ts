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
export function issueLabel(project: Pick<Project, "issue_key"> | null, issue: Issue): string {
  return `${project?.issue_key ?? "ISSUE"}-${issue.seq}`;
}

// Groups are collapsed by default once an issue is finished.
export function isClosed(status: IssueStatus): boolean {
  return status === "done" || status === "cancelled";
}

// Board order within a group: urgent first, then oldest first.
export function compareIssues(a: Issue, b: Issue): number {
  if (a.priority !== b.priority) return b.priority - a.priority;
  return a.createdAt - b.createdAt || a.seq - b.seq;
}

// Handoff from the home overview to a project's Issues tab: the overview
// stashes the clicked issue id here and IssuesView selects it once its own
// list loads (project selection resets tab state, so props can't carry it).
export const PENDING_ISSUE_KEY = "issues:pending-select";

// Same handoff trick for the palette's "New Issue": the Issues tab focuses
// quick-add when this flag is waiting.
export const PENDING_QUICKADD_KEY = "issues:focus-quickadd";
