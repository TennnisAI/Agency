import { useEffect, useMemo, useState } from "react";
import { FileRoot, createFile, createDir, renamePath, trashPath, writeFile } from "../api";
import { DocsIndex, SearchHit, searchDocs, stripExt } from "../lib/docsIndex";
import { joinPath, parentPath, baseName } from "../lib/filePath";
import { FileIcon } from "./fileIcons";
import Menu, { MenuEntry } from "./git/Menu";
import PromptDialog from "./PromptDialog";
import ConfirmDialog from "./ConfirmDialog";
import { toastError } from "../lib/toast";
import { revealLabel, reveal, copyAbsPath } from "../lib/fileActions";

// A folder level derived from the index's note paths.
interface TreeDir {
  path: string; // rel to docs dir
  dirs: Map<string, TreeDir>;
  notes: { path: string; title: string }[];
}

type Dialog =
  | { kind: "newNote"; dir: string }
  | { kind: "newFolder"; dir: string }
  | { kind: "rename"; orig: string };

function Twistie({ open }: { open: boolean }) {
  return (
    <svg className={`tree-twistie ${open ? "open" : ""}`} width="10" height="10" viewBox="0 0 24 24"
      fill="currentColor" aria-hidden>
      <path d="M8 5l8 7-8 7z" />
    </svg>
  );
}

/**
 * The Docs tab's note tree + search pane. Renders directly from the index (no
 * extra IPC; the corpus poll keeps it fresh). Markdown-only by design — other
 * files stay reachable via the Files tab. `extraDirs` keeps freshly created
 * empty folders visible until a note lands in them (the corpus can't see
 * folders without markdown).
 */
export default function DocsTree({
  root, docsDir, rootLabel, index, selected, query, onQuery, onSelect, onOpenHit, onRenamed, onDeleted, refresh,
}: {
  root: FileRoot;
  docsDir: string;
  /** Label for the tree root when docsDir is "" (the workspace vault). */
  rootLabel?: string;
  index: DocsIndex | null;
  selected: string | null;
  query: string;
  onQuery: (q: string) => void;
  onSelect: (path: string | null) => void;
  onOpenHit: (hit: SearchHit) => void;
  onRenamed: (from: string, to: string) => void;
  onDeleted: (path: string) => void;
  refresh: () => Promise<void>;
}) {
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [extraDirs, setExtraDirs] = useState<string[]>([]);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [confirmDel, setConfirmDel] = useState<{ path: string; isDir: boolean } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);

  const rootKey = `${root.kind}:${root.id}:${docsDir}`;
  useEffect(() => {
    setOpen(new Set());
    setExtraDirs([]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rootKey]);

  const tree = useMemo(() => {
    const rootDir: TreeDir = { path: "", dirs: new Map(), notes: [] };
    const dirAt = (dir: string): TreeDir => {
      if (dir === "") return rootDir;
      let cur = rootDir;
      let acc = "";
      for (const part of dir.split("/")) {
        acc = joinPath(acc, part);
        let next = cur.dirs.get(part);
        if (!next) {
          next = { path: acc, dirs: new Map(), notes: [] };
          cur.dirs.set(part, next);
        }
        cur = next;
      }
      return cur;
    };
    if (index) {
      for (const d of index.docs.values()) {
        dirAt(parentPath(d.path)).notes.push({ path: d.path, title: d.title });
      }
    }
    for (const dir of extraDirs) dirAt(dir);
    const sortDir = (d: TreeDir) => {
      // The journal reads newest-first (date-stamped names, so name order is
      // date order); everything else alphabetical.
      if (d.path === "journal") {
        d.notes.sort((a, b) => b.path.localeCompare(a.path));
      } else {
        d.notes.sort((a, b) => a.title.localeCompare(b.title));
      }
      for (const sub of d.dirs.values()) sortDir(sub);
    };
    sortDir(rootDir);
    return rootDir;
  }, [index, extraDirs]);

  // Keep the selected note's ancestors expanded (e.g. after wikilink navigation).
  useEffect(() => {
    if (!selected) return;
    setOpen((s) => {
      const n = new Set(s);
      let dir = parentPath(selected);
      while (dir !== "") {
        n.add(dir);
        dir = parentPath(dir);
      }
      return n;
    });
  }, [selected]);

  const toRepo = (path: string) => joinPath(docsDir, path);

  const toggle = (path: string) =>
    setOpen((s) => {
      const n = new Set(s);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });

  const doCreate = async (kind: "newNote" | "newFolder", dir: string, name: string) => {
    try {
      if (kind === "newNote") {
        const file = /\.(md|markdown)$/i.test(name) ? name : `${name}.md`;
        const path = joinPath(dir, file);
        await createFile(root, toRepo(path));
        await writeFile(root, toRepo(path), `# ${stripExt(file)}\n\n`);
        await refresh();
        onSelect(path);
      } else {
        const path = joinPath(dir, name);
        await createDir(root, toRepo(path));
        setExtraDirs((d) => [...d, path]);
        setOpen((s) => new Set(s).add(path));
      }
    } catch (e) {
      toastError(e, "Couldn't create");
    }
  };

  const doRename = async (orig: string, name: string) => {
    const dest = joinPath(parentPath(orig), name);
    if (dest === orig) return;
    try {
      await renamePath(root, toRepo(orig), toRepo(dest));
      await refresh();
      onRenamed(orig, dest);
    } catch (e) {
      toastError(e, "Rename failed");
    }
  };

  const doDelete = async (path: string) => {
    try {
      await trashPath(root, toRepo(path));
      setExtraDirs((d) => d.filter((x) => x !== path && !x.startsWith(path + "/")));
      await refresh();
      onDeleted(path);
    } catch (e) {
      toastError(e, "Delete failed");
    }
  };

  const openMenu = (e: React.MouseEvent, entry: { path: string; isDir: boolean } | null) => {
    e.preventDefault();
    e.stopPropagation();
    const dir = entry ? (entry.isDir ? entry.path : parentPath(entry.path)) : "";
    const items: MenuEntry[] = [
      { label: "New Note…", onClick: () => setDialog({ kind: "newNote", dir }) },
      { label: "New Folder…", onClick: () => setDialog({ kind: "newFolder", dir }) },
    ];
    if (entry) {
      items.push(
        { kind: "separator" },
        { label: "Rename…", onClick: () => setDialog({ kind: "rename", orig: entry.path }) },
        { label: "Delete", danger: true, onClick: () => setConfirmDel({ path: entry.path, isDir: entry.isDir }) },
        { kind: "separator" },
        { label: revealLabel, onClick: () => reveal(root, toRepo(entry.path)) },
        { label: "Copy Path", onClick: () => copyAbsPath(root, toRepo(entry.path)) },
      );
    } else {
      items.push(
        { kind: "separator" },
        { label: revealLabel, onClick: () => reveal(root, docsDir) },
      );
    }
    setMenu({ x: e.clientX, y: e.clientY, items });
  };

  const renderDir = (dir: TreeDir, depth: number): React.ReactNode[] => {
    const rows: React.ReactNode[] = [];
    const pad = { paddingLeft: 8 + depth * 12 };
    for (const [name, sub] of [...dir.dirs.entries()].sort(([a], [b]) => a.localeCompare(b))) {
      const isOpen = open.has(sub.path);
      rows.push(
        <div key={sub.path} className="tree-row dir" style={pad}
          onClick={() => toggle(sub.path)}
          onContextMenu={(e) => openMenu(e, { path: sub.path, isDir: true })}>
          <span className="tree-twistie-slot">
            <Twistie open={isOpen} />
          </span>
          <span className="tree-icon file-icon" style={{ color: "var(--blue)" }}>
            <FileIcon kind="folder" open={isOpen} />
          </span>
          <span className="tree-name">{name}</span>
        </div>,
      );
      if (isOpen) rows.push(...renderDir(sub, depth + 1));
    }
    for (const note of dir.notes) {
      rows.push(
        <div key={note.path}
          className={`tree-row file ${selected === note.path ? "on" : ""}`}
          style={pad}
          onClick={() => onSelect(note.path)}
          onContextMenu={(e) => openMenu(e, { path: note.path, isDir: false })}>
          <span className="tree-twistie-slot" />
          <span className="tree-icon file-icon" style={{ color: "var(--sub0)" }}>
            <NoteGlyph />
          </span>
          <span className="tree-name" title={note.path}>{note.title}</span>
        </div>,
      );
    }
    return rows;
  };

  const hits = useMemo(
    () => (index && query.trim() ? searchDocs(index, query) : []),
    [index, query],
  );

  const searching = query.trim() !== "";

  return (
    <>
      <div className="docs-search">
        <input
          className="docs-search-input"
          placeholder="Search notes…  (#tag)"
          value={query}
          onChange={(e) => onQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Escape") onQuery(""); }}
        />
      </div>
      <div className="files-root-label">
        <span className="files-root-name" title={docsDir || rootLabel}>{docsDir ? `${docsDir}/` : rootLabel ?? "/"}</span>
        <span className="spacer" style={{ flex: 1 }} />
        <button className="files-tool-btn" title="New Note" onClick={() => setDialog({ kind: "newNote", dir: "" })}>
          <NewNoteGlyph />
        </button>
        <button className="files-tool-btn" title="New Folder" onClick={() => setDialog({ kind: "newFolder", dir: "" })}>
          <NewFolderGlyph />
        </button>
        <button className="files-tool-btn" title="Collapse folders" onClick={() => setOpen(new Set())}>
          <CollapseGlyph />
        </button>
      </div>
      <div className="files-tree-body" onContextMenu={(e) => { if (e.target === e.currentTarget) openMenu(e, null); }}>
        {searching ? (
          <div className="docs-search-results">
            {hits.length === 0 && <div className="docs-search-none">No matches</div>}
            {hits.map((h, i) => (
              <div key={`${h.path}:${h.line}:${i}`} className="docs-search-hit" onClick={() => onOpenHit(h)}>
                <span className="docs-search-hit-title">{index?.docs.get(h.path)?.title ?? h.path}</span>
                <span className="docs-search-hit-snippet">{h.snippet}</span>
              </div>
            ))}
          </div>
        ) : index && index.docs.size === 0 && extraDirs.length === 0 ? (
          <div className="docs-search-none">No notes yet — create one.</div>
        ) : (
          renderDir(tree, 0)
        )}
      </div>

      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}

      {dialog && dialog.kind !== "rename" && (
        <PromptDialog
          title={dialog.kind === "newNote" ? "New Note" : "New Folder"}
          body={dialog.dir ? `in ${dialog.dir}/` : undefined}
          placeholder={dialog.kind === "newNote" ? "note name" : "folder name"}
          confirmLabel="Create"
          onConfirm={(v) => { setDialog(null); void doCreate(dialog.kind, dialog.dir, v); }}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog && dialog.kind === "rename" && (
        <PromptDialog
          title="Rename"
          initial={baseName(dialog.orig)}
          confirmLabel="Rename"
          onConfirm={(v) => { setDialog(null); void doRename(dialog.orig, v); }}
          onCancel={() => setDialog(null)}
        />
      )}
      {confirmDel && (
        <ConfirmDialog
          title={confirmDel.isDir ? "Delete folder?" : "Delete note?"}
          body={`Move "${baseName(confirmDel.path)}" to the trash.`}
          confirmLabel="Delete"
          danger
          onConfirm={() => { const p = confirmDel.path; setConfirmDel(null); void doDelete(p); }}
          onCancel={() => setConfirmDel(null)}
        />
      )}
    </>
  );
}

// ── Toolbar glyphs (stroked SVG, matching FileTree's convention) ──────
const gp = {
  width: 15, height: 15, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor",
  strokeWidth: 1.9, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, "aria-hidden": true,
};
function NewNoteGlyph() {
  return (
    <svg {...gp}>
      <path d="M13 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h6" />
      <polyline points="13 3 13 8 18 8" />
      <line x1="12" y1="19" x2="18" y2="19" />
      <line x1="15" y1="16" x2="15" y2="22" />
    </svg>
  );
}
function NewFolderGlyph() {
  return (
    <svg {...gp}>
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v3" />
      <path d="M3 7v10a2 2 0 0 0 2 2h6" />
      <line x1="16" y1="16" x2="22" y2="16" />
      <line x1="19" y1="13" x2="19" y2="19" />
    </svg>
  );
}
function CollapseGlyph() {
  return (
    <svg {...gp}>
      <polyline points="8 4 12 8 16 4" />
      <polyline points="8 20 12 16 16 20" />
    </svg>
  );
}
// Document with text lines — the note icon.
function NoteGlyph() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth={1.9} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
      <polyline points="14 3 14 8 19 8" />
      <line x1="9" y1="13" x2="15" y2="13" />
      <line x1="9" y1="17" x2="13" y2="17" />
    </svg>
  );
}
