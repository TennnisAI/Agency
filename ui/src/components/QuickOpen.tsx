import { useEffect, useMemo, useState } from "react";
import { FileRoot, listFiles } from "../api";
import { fuzzyFilter } from "../lib/fuzzy";
import { useModalKeys } from "../hooks/useModalKeys";
import { baseName, parentPath } from "../lib/filePath";
import { fileIcon } from "../lib/fileIcon";
import { FileIcon } from "./fileIcons";

/**
 * ⌘P file-name quick-open over the current root (focused agent's worktree or
 * the project checkout — the same root the Files tab shows). One listing per
 * open; the fuzzy ranking is client-side.
 */
export default function QuickOpen({
  root, onOpen, onClose,
}: {
  root: FileRoot;
  onOpen: (path: string) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [hi, setHi] = useState(0);
  const [files, setFiles] = useState<string[] | null>(null);
  const [failed, setFailed] = useState(false);
  useModalKeys(onClose);

  useEffect(() => {
    let cancelled = false;
    listFiles(root, "", 20000)
      .then((f) => { if (!cancelled) setFiles(f); })
      .catch(() => {
        if (!cancelled) {
          setFiles([]);
          setFailed(true);
        }
      });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [root.kind, root.id]);

  const rows = useMemo(() => fuzzyFilter(query, files ?? [], (p) => p, 50), [files, query]);

  const activate = (path: string) => {
    onOpen(path);
    onClose();
  };

  function onKey(ev: React.KeyboardEvent) {
    if (ev.key === "ArrowDown") {
      ev.preventDefault();
      setHi((h) => (rows.length === 0 ? 0 : Math.min(h + 1, rows.length - 1)));
    } else if (ev.key === "ArrowUp") {
      ev.preventDefault();
      setHi((h) => Math.max(h - 1, 0));
    } else if (ev.key === "Enter") {
      ev.preventDefault();
      if (rows[hi]) activate(rows[hi]);
    }
  }

  return (
    <div className="palette-overlay" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <input
          className="palette-input"
          autoFocus
          placeholder="Go to file…"
          value={query}
          onChange={(e) => { setQuery(e.target.value); setHi(0); }}
          onKeyDown={onKey}
        />
        <ul className="palette-list">
          {files === null && <li className="palette-empty">loading…</li>}
          {failed && <li className="palette-empty">Couldn't list files</li>}
          {files !== null && !failed && rows.length === 0 && <li className="palette-empty">No matching files</li>}
          {rows.map((p, i) => {
            const name = baseName(p);
            const { kind, color } = fileIcon(name);
            return (
              <li
                key={p}
                className={`palette-row ${i === hi ? "on" : ""}`}
                onMouseEnter={() => setHi(i)}
                onClick={() => activate(p)}
              >
                <span className="palette-kind file-icon" style={{ color }}>
                  <FileIcon kind={kind} />
                </span>
                <span className="palette-label">{name}</span>
                <span className="palette-sub">{parentPath(p)}</span>
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}
