import { HistoryItem } from "../../api";
import type { GraphRow } from "./graph";
import Graph from "./Graph.tsx";

function rel(ts: number): string {
  const s = Math.floor(Date.now() / 1000 - ts);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

export default function CommitRow({ item, graphRow, aheadOfBase, selected, onSelect }: {
  item: HistoryItem;
  graphRow: GraphRow;
  aheadOfBase: boolean;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <div className={`git-commitrow ${selected ? "sel" : ""} ${aheadOfBase ? "ahead" : "base"}`} onClick={onSelect}>
      <Graph row={graphRow} />
      <div className="git-commit-main">
        <div className="git-commit-subject">{item.subject}</div>
        <div className="git-commit-meta">
          {item.refs.map((r) => <span key={r} className="git-ref">{r}</span>)}
          <span className="git-commit-author">{item.author}</span>
          <span className="git-commit-hash">{item.hash.slice(0, 7)}</span>
          <span className="git-commit-date">{rel(item.date)}</span>
        </div>
      </div>
    </div>
  );
}
