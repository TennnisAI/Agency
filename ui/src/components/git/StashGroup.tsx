import { useState } from "react";
import { StashEntry } from "../../api";
import { StashIcon } from "./gitIcons";

/** Collapsible "Stashes" list mirroring the changes ResourceGroups. */
export default function StashGroup({ stashes, onApply, onPop, onDrop }: {
  stashes: StashEntry[];
  onApply: (s: StashEntry) => void;
  onPop: (s: StashEntry) => void;
  onDrop: (s: StashEntry) => void;
}) {
  const [open, setOpen] = useState(true);
  if (stashes.length === 0) return null;
  return (
    <div className="git-group">
      <div className="git-group-head">
        <button className="git-twisty" onClick={() => setOpen((o) => !o)}>{open ? "▾" : "▸"}</button>
        <span className="git-group-label">Stashes</span>
        <span className="git-count">{stashes.length}</span>
      </div>
      {open && stashes.map((s) => (
        <div key={s.index} className="git-row" title={s.message}>
          <span className="git-fileicon"><StashIcon /></span>
          <span className="git-name">
            <span className="git-basename">{s.message}</span>
            <span className="git-dir">{`stash@{${s.index}}`}</span>
          </span>
          <span className="git-row-actions">
            <button className="git-iconbtn" title="Drop Stash"
              onClick={(e) => { e.stopPropagation(); onDrop(s); }}>✕</button>
            <button className="git-iconbtn" title="Apply Stash (keep in list)"
              onClick={(e) => { e.stopPropagation(); onApply(s); }}>⇡</button>
            <button className="git-iconbtn" title="Pop Stash (apply and remove)"
              onClick={(e) => { e.stopPropagation(); onPop(s); }}>↥</button>
          </span>
        </div>
      ))}
    </div>
  );
}
