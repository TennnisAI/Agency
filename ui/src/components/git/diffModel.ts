import { diffWordsWithSpace } from "diff";
import type { FileDiff } from "../../api";

export type Span = { text: string; changed: boolean };
export type DiffRow = {
  kind: "ctx" | "add" | "del" | "spacer";
  hunkIndex: number;
  lineIndex: number; // index within hunk.lines (body), for staging selection
  oldNo: number | null;
  newNo: number | null;
  oldSpans: Span[] | null;
  newSpans: Span[] | null;
};

export function wordSpans(oldText: string, newText: string): { old: Span[]; new: Span[] } {
  const parts = diffWordsWithSpace(oldText, newText);
  const oldS: Span[] = [];
  const newS: Span[] = [];
  for (const p of parts) {
    if (p.added) newS.push({ text: p.value, changed: true });
    else if (p.removed) oldS.push({ text: p.value, changed: true });
    else { oldS.push({ text: p.value, changed: false }); newS.push({ text: p.value, changed: false }); }
  }
  return { old: oldS, new: newS };
}

const plain = (text: string): Span[] => [{ text, changed: false }];

function parseStarts(header: string): { oldStart: number; newStart: number } {
  const m = header.match(/@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
  return { oldStart: m ? +m[1] : 1, newStart: m ? +m[2] : 1 };
}

/** Build side-by-side aligned rows. Consecutive del/add runs are paired up so
 *  modified lines sit on one row (enabling intra-line word diff). */
export function buildRows(fd: FileDiff): DiffRow[] {
  const rows: DiffRow[] = [];
  fd.hunks.forEach((hunk, hunkIndex) => {
    let { oldStart: oldNo, newStart: newNo } = parseStarts(hunk.header);
    const lines = hunk.lines;
    let i = 0;
    while (i < lines.length) {
      const line = lines[i];
      const kind = line[0] ?? " ";
      if (kind === " ") {
        rows.push({ kind: "ctx", hunkIndex, lineIndex: i, oldNo, newNo, oldSpans: plain(line.slice(1)), newSpans: plain(line.slice(1)) });
        oldNo++; newNo++; i++;
        continue;
      }
      // collect a run of deletes then a run of adds
      const dels: number[] = [];
      const adds: number[] = [];
      while (i < lines.length && lines[i][0] === "-") { dels.push(i); i++; }
      while (i < lines.length && lines[i][0] === "+") { adds.push(i); i++; }
      const pairs = Math.max(dels.length, adds.length);
      for (let p = 0; p < pairs; p++) {
        const d = dels[p];
        const a = adds[p];
        if (d !== undefined && a !== undefined) {
          const { old, new: nw } = wordSpans(lines[d].slice(1), lines[a].slice(1));
          rows.push({ kind: "del", hunkIndex, lineIndex: d, oldNo, newNo: null, oldSpans: old, newSpans: null });
          rows[rows.length - 1].newNo = null;
          rows.push({ kind: "add", hunkIndex, lineIndex: a, oldNo: null, newNo, oldSpans: null, newSpans: nw });
          // pair them visually: represent as a single modify row pair (del row carries new side too)
          rows.splice(rows.length - 2, 2, {
            kind: "del", hunkIndex, lineIndex: d, oldNo, newNo,
            oldSpans: old, newSpans: nw,
          });
          oldNo++; newNo++;
        } else if (d !== undefined) {
          rows.push({ kind: "del", hunkIndex, lineIndex: d, oldNo, newNo: null, oldSpans: plain(lines[d].slice(1)), newSpans: null });
          oldNo++;
        } else if (a !== undefined) {
          rows.push({ kind: "add", hunkIndex, lineIndex: a, oldNo: null, newNo, oldSpans: null, newSpans: plain(lines[a].slice(1)) });
          newNo++;
        }
      }
    }
  });
  return rows;
}
