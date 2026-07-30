import { useEffect, useMemo, useState } from "react";
import { FileRoot, listFiles } from "../api";
import { fuzzyFilter } from "../lib/fuzzy";
import { useListNav } from "../hooks/useListNav";
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
  const { hi, setHi, onKey } = useListNav(rows.length, (i) => activate(rows[i]), query);

  return (
    <div className="palette-overlay" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <input
          className="palette-input"
          autoFocus
          placeholder="Go to file…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
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
