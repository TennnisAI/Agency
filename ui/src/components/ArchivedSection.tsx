import { useCallback, useEffect, useState } from "react";
import { RunInfo, listArchivedRuns, restoreRun, discardRun } from "../api";
import { useRuns } from "../store/runs";

export default function ArchivedSection() {
  const { selectedProjectId, refreshRuns, setFocusedRun } = useRuns();
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<RunInfo[]>([]);

  const load = useCallback(async () => {
    if (!selectedProjectId) return setItems([]);
    try {
      setItems(await listArchivedRuns(selectedProjectId));
    } catch {
      /* ignore */
    }
  }, [selectedProjectId]);

  useEffect(() => {
    if (open) load();
  }, [open, load]);

  if (!selectedProjectId) return null;

  return (
    <div className="archived">
      <button className="archived-head" onClick={() => setOpen((o) => !o)}>
        {open ? "▾" : "▸"} Archived{items.length ? ` (${items.length})` : ""}
      </button>
      {open &&
        items.map((r) => (
          <div key={r.id} className="archived-row">
            <span className="archived-name">{r.agent}: {r.prompt || r.branch}</span>
            <button
              className="icon-btn"
              title="Restore"
              onClick={async () => {
                const restored = await restoreRun(r.id);
                await refreshRuns();
                await load();
                setFocusedRun(restored.id);
              }}
            >↺</button>
            <button
              className="icon-btn danger"
              title="Discard permanently"
              onClick={async () => {
                await discardRun(r.id);
                await load();
              }}
            >✕</button>
          </div>
        ))}
      {open && items.length === 0 && <div className="archived-empty">Nothing archived.</div>}
    </div>
  );
}
