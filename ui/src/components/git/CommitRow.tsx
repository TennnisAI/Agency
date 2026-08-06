import { HistoryItem } from "../../api";
import type { GraphRow } from "./graph";
import Graph from "./Graph.tsx";
import { BranchIcon, CloudIcon, TagIcon } from "./gitIcons";

function rel(ts: number): string {
  const s = Math.floor(Date.now() / 1000 - ts);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

type RefKind = "head" | "local" | "remote" | "tag";

/** Classify a `%D` ref ("HEAD -> main", "origin/main", "tag: v1", "feature/x"). */
export function refInfo(r: string): { name: string; kind: RefKind } {
  if (r === "HEAD") return { name: "HEAD", kind: "head" };
  if (r.startsWith("HEAD -> ")) return { name: r.slice(8), kind: "head" };
  if (r.startsWith("tag: ")) return { name: r.slice(5), kind: "tag" };
  // The app only ever configures an `origin` remote.
  if (r.startsWith("origin/")) return { name: r, kind: "remote" };
  return { name: r, kind: "local" };
}

function RefPill({ r }: { r: string }) {
  const { name, kind } = refInfo(r);
  const icon = kind === "tag" ? <TagIcon /> : kind === "remote" ? <CloudIcon /> : <BranchIcon />;
  return (
    <span className={`git-ref git-ref-${kind}`} title={r}>
      {icon}
      <span className="git-ref-name">{name}</span>
    </span>
  );
}

export default function CommitRow({ item, graphRow, isHead, aheadOfBase, selected, onSelect, onContextMenu }: {
  item: HistoryItem;
  graphRow: GraphRow;
  isHead: boolean;
  aheadOfBase: boolean;
  selected: boolean;
  onSelect: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
}) {
  return (
    <div className={`git-commitrow ${selected ? "sel" : ""} ${aheadOfBase ? "ahead" : "base"}`}
      onClick={onSelect} onContextMenu={onContextMenu}>
      <Graph row={graphRow} head={isHead} />
      <div className="git-commit-main">
        <div className="git-commit-line">
          <span className="git-commit-subject" title={item.subject}>{item.subject}</span>
          {item.refs.map((r) => <RefPill key={r} r={r} />)}
        </div>
        <div className="git-commit-meta">
          <span className="git-commit-author">{item.author}</span>
          <span className="git-commit-hash">{item.hash.slice(0, 7)}</span>
          <span className="git-commit-date" title={new Date(item.date * 1000).toLocaleString()}>{rel(item.date)}</span>
        </div>
      </div>
    </div>
  );
}
