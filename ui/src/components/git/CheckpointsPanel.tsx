import { useCallback, useEffect, useState } from "react";
import { Checkpoint, checkpointList } from "../../api";
import Menu, { MenuEntry } from "./Menu";
import CheckpointRestoreDialog from "./CheckpointRestoreDialog";
import { checkpointLabel, newestFirst } from "./checkpoints";

/** Time of day for today's checkpoints, date and time for older ones. */
export function checkpointTime(at: number): string {
  const d = new Date(at * 1000);
  return d.toDateString() === new Date().toDateString()
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

/**
 * A run's workspace checkpoints (AGE-140), newest first. Picking one shows what
 * changed in that turn; its menu restores the files to it.
 */
export default function CheckpointsPanel({
  runId, checkout, reloadKey, selectedSeq, onSelect, onAct,
}: {
  runId: string;
  checkout: boolean;
  /** Bumped by the parent after a git action, restores included. */
  reloadKey: number;
  selectedSeq: number | null;
  onSelect: (cp: Checkpoint, prev: Checkpoint | null) => void;
  onAct: (fn: () => Promise<unknown>, label?: string) => Promise<boolean>;
}) {
  const [list, setList] = useState<Checkpoint[] | null>(null);
  const [error, setError] = useState("");
  const [menu, setMenu] = useState<{ x: number; y: number; cp: Checkpoint } | null>(null);
  const [restoring, setRestoring] = useState<Checkpoint | null>(null);

  const load = useCallback(async () => {
    try { setList(await checkpointList(runId)); setError(""); }
    catch (e) { setError(String(e)); }
  }, [runId]);
  // Checkpoints land from a background thread at turn boundaries, so nothing
  // here is told when one appears: a slow poll, like the history graph's.
  useEffect(() => { load(); }, [load, reloadKey]);
  useEffect(() => {
    const id = setInterval(() => load(), 5000);
    return () => clearInterval(id);
  }, [load]);

  if (error) return <div className="git-error">{error}</div>;
  if (!list) return null;
  const rows = newestFirst(list);
  if (rows.length === 0) {
    return (
      <div className="git-checkpoints-empty">
        No checkpoints yet. Agency saves one when you send a prompt and when the agent finishes a turn.
      </div>
    );
  }

  const menuItems = (cp: Checkpoint): MenuEntry[] => [
    { label: "Restore files to here…", danger: true, onClick: () => setRestoring(cp) },
  ];

  return (
    <div className="git-checkpoints">
      {rows.map(({ cp, prev }) => (
        <div
          key={cp.seq}
          className={`git-checkpoint-row ${selectedSeq === cp.seq ? "sel" : ""}`}
          onClick={() => onSelect(cp, prev)}
          onContextMenu={(e) => { e.preventDefault(); setMenu({ x: e.clientX, y: e.clientY, cp }); }}
        >
          <span className="git-checkpoint-label">{checkpointLabel(cp.kind)}</span>
          <span className="git-checkpoint-seq">{cp.seq}</span>
          <span className="git-checkpoint-time" title={new Date(cp.at * 1000).toLocaleString()}>
            {checkpointTime(cp.at)}
          </span>
        </div>
      ))}
      {menu && <Menu x={menu.x} y={menu.y} items={menuItems(menu.cp)} onClose={() => setMenu(null)} />}
      {restoring && (
        <CheckpointRestoreDialog runId={runId} cp={restoring} checkout={checkout} onAct={onAct}
          onClose={() => setRestoring(null)} />
      )}
    </div>
  );
}
