import { IssuePatch, IssueStatus } from "../api";
import { ISSUE_STATUSES, PRIORITY_LABELS } from "./issues";

// A new issue being written in the composer (AGE-255). The board used to
// capture issues from a one-line input that only became an issue on Enter:
// the rest of the ticket could only be filled in after that, and a half-typed
// title was dropped the moment the user left the tab to check on an agent,
// because the board unmounts on every tab switch. The draft is therefore
// stored, per project, on every keystroke, and outlives the board, the tab and
// a restart.
export interface IssueDraft {
  title: string;
  body: string;
  status: IssueStatus;
  // 0 none · 1 low · 2 medium · 3 high · 4 urgent, as on `Issue`.
  priority: number;
  due: string | null;
  scheduled: string | null;
  // Whether the composer is up. A draft closed without a title is kept but put
  // away, and the board should come back to it closed, not sprung open.
  open: boolean;
}

export const NEW_DRAFT: IssueDraft = {
  title: "",
  body: "",
  status: "todo",
  priority: 0,
  due: null,
  scheduled: null,
  open: true,
};

export function issuesDraftKey(projectId: string): string {
  return `issues-draft:${projectId}`;
}

const DATE = /^\d{4}-\d{2}-\d{2}$/;

// Storage is the user's to edit and another build's to have written, so
// every field is checked against the exact values the tracker accepts, and a
// field that fails falls back to the new-draft default rather than reaching
// `createIssue` to be rejected there with the draft already cleared.
export function parseDraft(raw: string | null): IssueDraft | null {
  if (raw === null) return null;
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof v !== "object" || v === null || Array.isArray(v)) return null;
  const o = v as Record<string, unknown>;
  const str = (x: unknown) => (typeof x === "string" ? x : "");
  const date = (x: unknown) => (typeof x === "string" && DATE.test(x) ? x : null);
  return {
    title: str(o.title),
    body: str(o.body),
    status: ISSUE_STATUSES.includes(o.status as IssueStatus) ? (o.status as IssueStatus) : NEW_DRAFT.status,
    priority:
      Number.isInteger(o.priority) && (o.priority as number) >= 0 && (o.priority as number) < PRIORITY_LABELS.length
        ? (o.priority as number)
        : NEW_DRAFT.priority,
    due: date(o.due),
    scheduled: date(o.scheduled),
    open: o.open === true,
  };
}

export function loadDraft(storage: Pick<Storage, "getItem">, key: string): IssueDraft | null {
  return parseDraft(storage.getItem("pane:" + key));
}

// Null clears the key: a draft that was saved or discarded is gone, not blank.
export function saveDraft(
  storage: Pick<Storage, "setItem" | "removeItem">,
  key: string,
  draft: IssueDraft | null,
): void {
  if (draft === null) storage.removeItem("pane:" + key);
  else storage.setItem("pane:" + key, JSON.stringify(draft));
}

// What closing the composer does with what is in it. Closing saves, as the
// detail pane's edits do, but only a draft with a title can become an issue:
// one with only a description is kept for later rather than thrown away, and
// one with nothing typed in it is simply gone. Properties alone are not worth
// keeping; they take a click each to set again.
export type CloseOutcome = "create" | "keep" | "discard";

export function closeOutcome(draft: IssueDraft): CloseOutcome {
  if (draft.title.trim()) return "create";
  if (draft.body.trim()) return "keep";
  return "discard";
}

// Whether there is anything to come back to: what the + button marks.
export function draftWaiting(draft: IssueDraft | null): boolean {
  return draft !== null && closeOutcome(draft) !== "discard";
}

// The properties `createIssue` cannot carry, as the patch that follows it.
// Null when every one is still at its default, so a plain issue is one write.
export function draftExtras(draft: IssueDraft): IssuePatch | null {
  const p: IssuePatch = {};
  if (draft.priority !== 0) p.priority = draft.priority;
  if (draft.due) p.due = draft.due;
  if (draft.scheduled) p.scheduled = draft.scheduled;
  return Object.keys(p).length > 0 ? p : null;
}
