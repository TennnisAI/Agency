import type { DiffRow } from "../git/diffModel";
import type { DraftComment } from "../../api";

// A pending inline comment held locally until the review is submitted. `id` is a
// client-only handle for editing/removing before submit.
export type DraftEntry = DraftComment & { id: string };

// GitHub review-comment anchor: a line + side, with an optional start for a
// multi-line span. Computed purely from the selected diff rows so it can be
// unit-tested without a live PR.
export type Side = "RIGHT" | "LEFT";
export type Anchor = {
  line: number;
  side: Side;
  startLine?: number;
  startSide?: Side;
};

// A row's anchor is its new-file line when it has one (added, context, and the
// paired "modify" rows all carry newNo) — GitHub anchors on the new file by
// default — and its old-file line only for a pure deletion.
function rowAnchor(r: DiffRow): { n: number; side: Side } | null {
  if (r.newNo != null) return { n: r.newNo, side: "RIGHT" };
  if (r.oldNo != null) return { n: r.oldNo, side: "LEFT" };
  return null;
}

// Reduce a selection of diff rows to one GitHub anchor. A single row → a
// single-line comment; a same-side span → a start/end range; a mixed span
// (deletions + additions) clamps to the RIGHT (new) side, which GitHub requires
// since a range can't straddle sides.
export function computeAnchor(rows: DiffRow[]): Anchor | null {
  const anchors = rows.map(rowAnchor).filter((a): a is { n: number; side: Side } => a != null);
  if (anchors.length === 0) return null;

  const sides = new Set(anchors.map((a) => a.side));
  let side: Side;
  let pool: number[];
  if (sides.size === 1) {
    side = anchors[0].side;
    pool = anchors.map((a) => a.n);
  } else {
    side = "RIGHT";
    pool = anchors.filter((a) => a.side === "RIGHT").map((a) => a.n);
  }

  const start = Math.min(...pool);
  const end = Math.max(...pool);
  if (start === end) return { line: end, side };
  return { line: end, side, startLine: start, startSide: side };
}
