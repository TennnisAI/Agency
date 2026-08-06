import type { GraphRow, Segment } from "./graph";

// Column pitch, row height (must match .git-commitrow height), dot radius.
// The SVG spans the entire row so adjacent rows' rails touch with no gaps.
export const LANE_W = 11;
export const ROW_H = 42;
const R = 3.5;
const x = (i: number) => LANE_W * (i + 1);

/** A rail segment as a path: straight when the columns match, an S-curve otherwise. */
function seg(s: Segment, y1: number, y2: number): string {
  const x1 = x(s.from);
  const x2 = x(s.to);
  if (x1 === x2) return `M ${x1} ${y1} L ${x2} ${y2}`;
  const ym = (y1 + y2) / 2;
  return `M ${x1} ${y1} C ${x1} ${ym}, ${x2} ${ym}, ${x2} ${y2}`;
}

export default function Graph({ row, head = false }: {
  row: GraphRow;
  /** Draw a highlight ring (this commit is HEAD). */
  head?: boolean;
}) {
  // Only as wide as this row's own columns. Columns are absolute (x() maps a
  // column to the same pixel in every row), so rails still meet across rows —
  // but a tangled row deep in the history no longer indents every other row's
  // text. The commit text then sits just past the rails the row actually has.
  const width = LANE_W * (row.lanes + 1);
  const cx = x(row.circleIndex);
  const cy = ROW_H / 2;
  const paths = row.segments.map((s, i) => {
    const [y1, y2] = s.kind === "pass" ? [0, ROW_H] : s.kind === "enter" ? [0, cy] : [cy, ROW_H];
    return (
      <path key={i} d={seg(s, y1, y2)} fill="none"
        stroke={`var(${s.color})`} strokeWidth={1.6} strokeLinecap="round" />
    );
  });
  return (
    <svg className="git-graph" width={width} height={ROW_H} viewBox={`0 0 ${width} ${ROW_H}`}>
      {paths}
      {head && <circle cx={cx} cy={cy} r={R + 2.5} fill="none" stroke={`var(${row.color})`} strokeWidth={1.2} opacity={0.55} />}
      {row.isMerge
        ? <circle cx={cx} cy={cy} r={R - 0.5} fill="var(--mantle)" stroke={`var(${row.color})`} strokeWidth={1.8} />
        : <circle cx={cx} cy={cy} r={R} fill={`var(${row.color})`} />}
    </svg>
  );
}
