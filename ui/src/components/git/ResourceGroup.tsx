import { useState } from "react";
import { FileChange } from "../../api";
import FileRow from "./FileRow";
import type { GitGroup } from "./status";
import { useVirtualRows } from "../../hooks/useVirtualRows";

export default function ResourceGroup({
  id, label, changes, selectedPath, onSelectFile, onFilePrimary, onFileDiscard,
  onStageAll, onUnstageAll, onDiscardAll,
}: {
  id: GitGroup;
  label: string;
  changes: FileChange[];
  selectedPath: string | null;
  onSelectFile: (c: FileChange) => void;
  onFilePrimary: (c: FileChange) => void;
  onFileDiscard?: (c: FileChange) => void;
  onStageAll?: () => void;
  onUnstageAll?: () => void;
  onDiscardAll?: () => void;
}) {
  const [open, setOpen] = useState(true);
  // Window the rows: with tens of thousands of changes, mounting every FileRow
  // (each ~7 DOM nodes) blocks the main thread for many seconds and freezes the
  // whole app. Only the visible slice is rendered; spacers hold the scrollbar.
  const { regionRef, range, padTop, padBottom } = useVirtualRows(open ? changes.length : 0);
  if (changes.length === 0) return null;
  return (
    <div className="git-group">
      <div className="git-group-head">
        <button className="git-twisty" onClick={() => setOpen((o) => !o)}>{open ? "▾" : "▸"}</button>
        <span className="git-group-label">{label}</span>
        <span className="git-count">{changes.length}</span>
        <span className="git-group-actions">
          {onDiscardAll && <button className="git-iconbtn" title="Discard All" onClick={onDiscardAll}>↩</button>}
          {onUnstageAll && <button className="git-iconbtn" title="Unstage All" onClick={onUnstageAll}>−</button>}
          {onStageAll && <button className="git-iconbtn" title="Stage All" onClick={onStageAll}>+</button>}
        </span>
      </div>
      {open && (
        <div className="git-rows" ref={regionRef}>
          {padTop > 0 && <div style={{ height: padTop, flexShrink: 0 }} aria-hidden />}
          {changes.slice(range.start, range.end).map((c) => (
            <FileRow key={`${id}:${c.path}`} change={c} group={id}
              selected={selectedPath === c.path}
              onSelect={() => onSelectFile(c)}
              onPrimary={() => onFilePrimary(c)}
              onDiscard={onFileDiscard ? () => onFileDiscard(c) : undefined} />
          ))}
          {padBottom > 0 && <div style={{ height: padBottom, flexShrink: 0 }} aria-hidden />}
        </div>
      )}
    </div>
  );
}
