import { useCallback, useEffect, useState } from "react";
import {
  HistoryItem, gitLogGraph, gitCherryPick, gitRevertCommit, gitResetTo, gitCreateBranch,
} from "../../api";
import { toastSuccess } from "../../lib/toast";
import { computeGraph } from "./graph";
import CommitRow from "./CommitRow";
import Menu, { MenuEntry } from "./Menu";
import { BranchIcon } from "./gitIcons";
import ConfirmDialog from "../ConfirmDialog";
import PromptDialog from "../PromptDialog";

export default function HistoryPanel({ taskId, base, onSelectCommit, selectedHash, reloadKey = 0, onAct }: {
  taskId: string;
  base: string | null;
  onSelectCommit: (item: HistoryItem) => void;
  selectedHash: string | null;
  // Bumped by the parent after a git action to force an immediate reload.
  reloadKey?: number;
  onAct: (fn: () => Promise<unknown>, label?: string) => Promise<boolean>;
}) {
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [error, setError] = useState("");
  const [menu, setMenu] = useState<{ x: number; y: number; item: HistoryItem } | null>(null);
  const [confirm, setConfirm] = useState<{ title: string; body: string; label: string; run: () => void } | null>(null);
  const [branchFrom, setBranchFrom] = useState<HistoryItem | null>(null);

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
  // Every row shares the widest row's column count so the gutter (and thus
  // every rail) lines up vertically from row to row.
  const lanes = graph.reduce((m, r) => Math.max(m, r.lanes), 1);
  // commits before the base hash (newest-first) are "ahead of base"
  const baseIdx = base ? items.findIndex((i) => i.hash.startsWith(base) || base.startsWith(i.hash)) : -1;
  const headIdx = items.findIndex((i) => i.refs.some((r) => r === "HEAD" || r.startsWith("HEAD -> ")));

  const copy = async (text: string, what: string) => {
    await navigator.clipboard.writeText(text);
    toastSuccess(`${what} copied`);
  };

  const menuItems = (item: HistoryItem): MenuEntry[] => {
    const short = item.hash.slice(0, 7);
    return [
      { label: "Copy Commit Hash", hint: short, onClick: () => copy(item.hash, "Hash") },
      { label: "Copy Commit Message", onClick: () => copy(item.subject, "Message") },
      { kind: "separator" },
      { label: "Create Branch from Commit…", glyph: <BranchIcon />, onClick: () => setBranchFrom(item) },
      { label: "Cherry-Pick Commit", onClick: () => onAct(() => gitCherryPick(taskId, item.hash), `Cherry-picked ${short}`) },
      {
        label: "Revert Commit",
        onClick: () => onAct(() => gitRevertCommit(taskId, item.hash), `Reverted ${short}`),
      },
      { kind: "separator" },
      {
        label: "Reset Branch to Here (Soft)",
        onClick: () => onAct(() => gitResetTo(taskId, item.hash, "soft"), `Reset (soft) to ${short}`),
      },
      {
        label: "Reset Branch to Here (Hard)…",
        danger: true,
        onClick: () => setConfirm({
          title: "Hard reset",
          body: `Reset the current branch to ${short} and discard all changes after it, including uncommitted work? This cannot be undone.`,
          label: "Hard Reset",
          run: () => onAct(() => gitResetTo(taskId, item.hash, "hard"), `Reset (hard) to ${short}`),
        }),
      },
    ];
  };

  if (error) return <div className="git-error">{error}</div>;
  return (
    <div className="git-history">
      {items.map((item, i) => (
        <CommitRow key={item.hash} item={item} graphRow={graph[i]} lanes={lanes}
          isHead={i === (headIdx < 0 ? 0 : headIdx)}
          aheadOfBase={baseIdx < 0 ? false : i < baseIdx}
          selected={selectedHash === item.hash}
          onSelect={() => onSelectCommit(item)}
          onContextMenu={(e) => { e.preventDefault(); setMenu({ x: e.clientX, y: e.clientY, item }); }} />
      ))}
      {menu && <Menu x={menu.x} y={menu.y} items={menuItems(menu.item)} onClose={() => setMenu(null)} />}
      {confirm && (
        <ConfirmDialog title={confirm.title} body={confirm.body} confirmLabel={confirm.label} danger
          onConfirm={() => { confirm.run(); setConfirm(null); }}
          onCancel={() => setConfirm(null)} />
      )}
      {branchFrom && (
        <PromptDialog title="Create branch" body={`New branch at ${branchFrom.hash.slice(0, 7)} — ${branchFrom.subject}`}
          placeholder="branch name" confirmLabel="Create & Switch"
          onConfirm={(name) => {
            onAct(() => gitCreateBranch(taskId, name, branchFrom.hash, true), `Switched to ${name}`);
            setBranchFrom(null);
          }}
          onCancel={() => setBranchFrom(null)} />
      )}
    </div>
  );
}
