import { useMemo, useState } from "react";
import { DocsIndex } from "../lib/docsIndex";
import { useModalKeys } from "../hooks/useModalKeys";

interface Row {
  kind: "note" | "create";
  path: string; // note path, or the name to create
  label: string;
  sublabel: string;
}

/**
 * Cmd+P fuzzy note switcher for the Docs tab. Enter opens the highlighted
 * note; when nothing matches (or on the trailing "create" row) the query
 * becomes a new note.
 */
export default function DocsQuickSwitcher({
  index, onOpen, onCreate, onClose,
}: {
  index: DocsIndex | null;
  onOpen: (path: string) => void;
  onCreate: (name: string) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [hi, setHi] = useState(0);
  useModalKeys(onClose);

  const rows = useMemo<Row[]>(() => {
    const q = query.trim().toLowerCase();
    const notes: Row[] = index
      ? [...index.docs.values()]
          .map((d) => ({ kind: "note" as const, path: d.path, label: d.title, sublabel: d.path }))
          .filter((r) => !q || r.label.toLowerCase().includes(q) || r.sublabel.toLowerCase().includes(q))
          .sort((a, b) => {
            if (q) {
              const as = a.label.toLowerCase().startsWith(q) ? 0 : 1;
              const bs = b.label.toLowerCase().startsWith(q) ? 0 : 1;
              if (as !== bs) return as - bs;
            }
            return a.label.localeCompare(b.label);
          })
      : [];
    const name = query.trim();
    const exists = name && notes.some((r) => r.label.toLowerCase() === name.toLowerCase());
    if (name && !exists) {
      notes.push({ kind: "create", path: name, label: `Create "${name}"`, sublabel: `${name}.md` });
    }
    return notes;
  }, [index, query]);

  const activate = (row: Row) => {
    if (row.kind === "note") onOpen(row.path);
    else onCreate(row.path);
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
          placeholder="Open or create a note…"
          value={query}
          onChange={(e) => { setQuery(e.target.value); setHi(0); }}
          onKeyDown={onKey}
        />
        <ul className="palette-list">
          {rows.length === 0 && <li className="palette-empty">No notes</li>}
          {rows.map((r, i) => (
            <li
              key={`${r.kind}:${r.path}`}
              className={`palette-row ${i === hi ? "on" : ""}`}
              onMouseEnter={() => setHi(i)}
              onClick={() => activate(r)}
            >
              <span className="palette-kind">{r.kind === "create" ? "＋" : "▹"}</span>
              <span className="palette-label">{r.label}</span>
              <span className="palette-sub">{r.sublabel}</span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
