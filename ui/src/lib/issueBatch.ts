import { Issue, IssuePatch } from "../api";

// Batch editing on the issue board: which rows are checked, and what a change
// applied to all of them actually has to write. Pure, so the rules are tested
// here and IssuesView only wires them to clicks and calls.

// The fields a batch can set. Title, body and links are per-issue by nature,
// and rank is the drag's business.
export type BatchPatch = Pick<IssuePatch, "status" | "priority" | "due" | "scheduled">;

const BATCH_FIELDS = ["status", "priority", "due", "scheduled"] as const;

// ⌘-click: flip one row. The first ⌘-click with nothing checked yet takes the
// open issue along, the way a file browser's first ⌘-click adds to the item
// already selected rather than starting over from nothing.
export function toggleChecked(checked: Set<string>, id: string, open: string | null): Set<string> {
  const next = new Set(checked);
  if (next.size === 0 && open && open !== id) next.add(open);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

// ⇧-click: add every visible row between the anchor and this one, inclusive.
// An anchor the board no longer shows (or none at all) checks just this row.
export function rangeChecked(
  checked: Set<string>,
  visible: Pick<Issue, "id">[],
  anchor: string | null,
  id: string,
): Set<string> {
  const to = visible.findIndex((i) => i.id === id);
  if (to < 0) return checked;
  const from = anchor ? visible.findIndex((i) => i.id === anchor) : -1;
  const next = new Set(checked);
  const [lo, hi] = from < 0 ? [to, to] : [Math.min(from, to), Math.max(from, to)];
  for (let n = lo; n <= hi; n++) next.add(visible[n].id);
  return next;
}

// Only rows on screen stay checked. A filter that hides a checked issue, or a
// status change that drops it into a folded group, takes it out of the batch:
// an edit applied to a row nobody can see is an edit nobody meant. Returns the
// same set when nothing went, so it can run on every render's list.
export function pruneChecked(checked: Set<string>, visible: Pick<Issue, "id">[]): Set<string> {
  if (checked.size === 0) return checked;
  const live = new Set(visible.map((i) => i.id));
  for (const id of checked) {
    if (!live.has(id)) return new Set([...checked].filter((c) => live.has(c)));
  }
  return checked;
}

// The writes a batch change needs: per issue, only the fields that differ, and
// no entry at all for an issue already there. Every write stamps `updated`, and
// on a shared backlog a stamp with no change behind it is a field the next sync
// may "decide" against the other machine's real edit.
export function batchWrites(issues: Issue[], patch: BatchPatch): { issue: Issue; patch: BatchPatch }[] {
  const out: { issue: Issue; patch: BatchPatch }[] = [];
  for (const issue of issues) {
    const diff: BatchPatch = {};
    for (const f of BATCH_FIELDS) {
      const v = patch[f];
      if (v !== undefined && v !== issue[f]) Object.assign(diff, { [f]: v });
    }
    if (Object.keys(diff).length > 0) out.push({ issue, patch: diff });
  }
  return out;
}

// The value every issue in the batch shares, for the menu's ✓; undefined when
// they differ (or there are none), so a mixed batch marks nothing.
export function sharedValue<K extends keyof BatchPatch>(issues: Issue[], key: K): Issue[K] | undefined {
  if (issues.length === 0) return undefined;
  const first = issues[0][key];
  return issues.every((i) => i[key] === first) ? first : undefined;
}

// What a finished batch says. A partial failure names how many landed, since
// the rows that did change are already on the board and "couldn't update" alone
// reads as if none had.
export function batchSummary(verb: string, done: number, total: number): string {
  const noun = (n: number) => `${n} issue${n === 1 ? "" : "s"}`;
  return done === total ? `${verb} ${noun(total)}` : `${verb} ${done} of ${noun(total)}`;
}
