import type { GraphRow } from "./graph";

const W = 11, H = 22, R = 4;
const x = (i: number) => W * (i + 1);

export default function Graph({ row }: { row: GraphRow }) {
  const lanes = Math.max(row.input.length, row.output.length, 1);
  const width = W * (lanes + 1);
  const cx = x(row.circleIndex);
  return (
    <svg className="git-graph" width={width} height={H} viewBox={`0 0 ${width} ${H}`}>
      {row.output.map((lane, i) => (
        <line key={`o${i}`} x1={x(i)} y1={H / 2} x2={x(i)} y2={H}
          stroke={`var(${lane.color})`} strokeWidth={2} />
      ))}
      {row.input.map((lane, i) => (
        <line key={`i${i}`} x1={x(i)} y1={0} x2={x(i)} y2={H / 2}
          stroke={`var(${lane.color})`} strokeWidth={2} />
      ))}
      <circle cx={cx} cy={H / 2} r={R} fill={`var(${row.color})`} />
    </svg>
  );
}
