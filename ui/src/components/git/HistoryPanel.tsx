import { useCallback, useEffect, useState } from "react";
import { HistoryItem, gitLogGraph } from "../../api";
import { computeGraph } from "./graph";
import CommitRow from "./CommitRow";

export default function HistoryPanel({ taskId, base, onSelectCommit, selectedHash, reloadKey = 0 }: {
  taskId: string;
  base: string | null;
  onSelectCommit: (item: HistoryItem) => void;
  selectedHash: string | null;
  // Bumped by the parent after a git action to force an immediate reload.
  reloadKey?: number;
}) {
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try { setItems(await gitLogGraph(taskId, 80)); setError(""); }
    catch (e) { setError(String(e)); }
  }, [taskId]);
  // Reload on mount, whenever the parent signals an action (reloadKey), and on a
  // slow poll so commits made outside the app (by the agent/terminal) show up.
  useEffect(() => { load(); }, [load, reloadKey]);
  useEffect(() => {
    const id = setInterval(() => load(), 5000);
    return () => clearInterval(id);
  }, [load]);

  const graph = computeGraph(items.map((i) => ({ hash: i.hash, parents: i.parents })));
  // commits before the base hash (newest-first) are "ahead of base"
  const baseIdx = base ? items.findIndex((i) => i.hash.startsWith(base) || base.startsWith(i.hash)) : -1;

  if (error) return <div className="git-error">{error}</div>;
  return (
    <div className="git-history">
      {items.map((item, i) => (
        <CommitRow key={item.hash} item={item} graphRow={graph[i]}
          aheadOfBase={baseIdx < 0 ? false : i < baseIdx}
          selected={selectedHash === item.hash}
          onSelect={() => onSelectCommit(item)} />
      ))}
    </div>
  );
}
