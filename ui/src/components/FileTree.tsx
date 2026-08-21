import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  BackendSearchHit, DirEntry, FileRoot, listDir, searchFiles,
  createFile, createDir, importFile, renamePath, trashPath,
} from "../api";
import { ancestorDirs, joinPath, parentPath, baseName } from "../lib/filePath";
import { fileIcon } from "../lib/fileIcon";
import { FileIcon } from "./fileIcons";
import Menu, { MenuEntry } from "./git/Menu";
import PromptDialog from "./PromptDialog";
import ConfirmDialog from "./ConfirmDialog";
import { dirAtPoint, useFileDrop } from "../hooks/useFileDrop";
import { dropName, nameList, uniqueName } from "../lib/fileDrop";
import { toastError, toastInfo } from "../lib/toast";
import { revealLabel, reveal, copyAbsPath, copyRelPath, ignorePath } from "../lib/fileActions";

// A pending create/rename dialog. `dir` is the container for a create; `orig`
// is the existing path for a rename.
type Dialog =
  | { kind: "newFile"; dir: string }
  | { kind: "newFolder"; dir: string }
  | { kind: "rename"; orig: string };

// Small disclosure triangle; rotates from ▸ (closed) to ▾ (open) via CSS.
function Twistie({ open }: { open: boolean }) {
  return (
    <svg className={`tree-twistie ${open ? "open" : ""}`} width="13" height="13" viewBox="0 0 24 24"
      fill="currentColor" aria-hidden>
      <path d="M7 4l10 8-10 8z" />
    </svg>
  );
}

export default function FileTree({
  root, selected, revealTarget, query, onQuery, onSelect, onOpenHit, onRenamed, onDeleted,
}: {
  root: FileRoot;
  selected: string | null;
  /**
   * "Reveal in Files": show this path in the tree. The nonce is what makes
   * revealing the same path twice a fresh request. (Not `reveal` — that name
   * belongs to the Finder action imported above.)
   */
  revealTarget?: { path: string; nonce: number } | null;
  /** Find-in-files query; non-empty replaces the tree with grouped hits. */
  query: string;
  onQuery: (q: string) => void;
  onSelect: (path: string | null) => void;
  /** A search hit was clicked: open `path` at the 1-based `line`. */
  onOpenHit: (path: string, line: number) => void;
  onRenamed: (from: string, to: string) => void;
  onDeleted: (path: string) => void;
}) {
  // Centralized tree state keyed by dir path ("" = root). Lifting it out of the
  // rows lets a mutation refresh exactly the affected directory.
  const [cache, setCache] = useState<Map<string, DirEntry[]>>(new Map());
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [errors, setErrors] = useState<Map<string, string>>(new Map());
  const [rootError, setRootError] = useState("");
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [confirmDel, setConfirmDel] = useState<{ path: string; isDir: boolean } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);

  const rootKey = `${root.kind}:${root.id}`;
  // The scrolling tree body: the drop target below, and what a reveal scrolls.
  const bodyRef = useRef<HTMLDivElement>(null);
  // The reveal walk below runs in a promise chain, where `cache` would be
  // whatever it was when the effect started.
  const cacheRef = useRef(cache);
  cacheRef.current = cache;

  const loadDir = useCallback((path: string) => {
    return listDir(root, path)
      .then((entries) => {
        setCache((m) => new Map(m).set(path, entries));
        setErrors((m) => { const n = new Map(m); n.delete(path); return n; });
      })
      .catch((e) => {
        if (path === "") setRootError(String(e));
        else setErrors((m) => new Map(m).set(path, String(e)));
      });
  }, [root]);

  // Re-root whenever the FileRoot changes (focused agent ↔ project main).
  useEffect(() => {
    setCache(new Map());
    setOpen(new Set());
    setErrors(new Map());
    setRootError("");
    loadDir("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rootKey]);

  const toggle = (path: string) => {
    setOpen((s) => {
      const n = new Set(s);
      if (n.has(path)) { n.delete(path); }
      else { n.add(path); if (!cache.has(path)) loadDir(path); }
      return n;
    });
  };

  const expand = (path: string) => {
    setOpen((s) => (s.has(path) ? s : new Set(s).add(path)));
    if (!cache.has(path)) loadDir(path);
  };

  // Reveal: open every folder on the way to the path, then scroll its row into
  // view. Nothing below the root is listed until it is opened, so the row does
  // not exist yet when the request lands — the listings are fetched first, and
  // the scroll still retries across a few frames because a row mounts one
  // render after the listing that holds it.
  useEffect(() => {
    if (!revealTarget) return;
    let cancelled = false;
    // An untracked folder arrives from source control with a trailing slash;
    // it is the thing to open, not just something on the way to it.
    const isDir = revealTarget.path.endsWith("/");
    const target = isDir ? revealTarget.path.slice(0, -1) : revealTarget.path;
    const dirs = ancestorDirs(target);
    if (isDir) dirs.push(target);
    setOpen((s) => new Set([...s, ...dirs]));
    const pending = dirs.filter((d) => !cacheRef.current.has(d)).map(loadDir);
    void Promise.all(pending).then(() => {
      const scroll = (tries: number) => {
        if (cancelled) return;
        const row = bodyRef.current?.querySelector(`[data-path="${CSS.escape(target)}"]`);
        if (row) { row.scrollIntoView({ block: "nearest" }); return; }
        if (tries > 0) requestAnimationFrame(() => scroll(tries - 1));
      };
      requestAnimationFrame(() => scroll(6));
    });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [revealTarget]);

  // The directory a create should land in: a folder targets itself, a file its
  // parent, and no target means the root.
  const containerOf = (entry: { path: string; isDir: boolean } | null) =>
    entry ? (entry.isDir ? entry.path : parentPath(entry.path)) : "";

  const doCreate = async (kind: "newFile" | "newFolder", dir: string, name: string) => {
    const path = joinPath(dir, name);
    try {
      if (kind === "newFile") await createFile(root, path);
      else await createDir(root, path);
      if (dir !== "") expand(dir);
      await loadDir(dir);
      if (kind === "newFile") onSelect(path);
      else expand(path);
    } catch (e) {
      toastError(e, "Couldn't create");
    }
  };

  const doRename = async (orig: string, name: string) => {
    const dir = parentPath(orig);
    const dest = joinPath(dir, name);
    if (dest === orig) return;
    try {
      await renamePath(root, orig, dest);
      await loadDir(dir);
      // The owner retargets open tabs/selection (and their unsaved buffers).
      onRenamed(orig, dest);
    } catch (e) {
      toastError(e, "Rename failed");
    }
  };

  const doDelete = async (path: string) => {
    const dir = parentPath(path);
    try {
      await trashPath(root, path);
      await loadDir(dir);
      onDeleted(path);
    } catch (e) {
      toastError(e, "Delete failed");
    }
  };

  // ── dropping in from Finder ───────────────────────────────────────────────
  // Files land in the folder under the cursor (blank space below the rows is
  // the root). Copies, never moves: the original stays where the user had it.

  const [importing, setImporting] = useState(false);
  // The drop listener is installed once, so it would otherwise read `importing`
  // from the render that installed it.
  const importingRef = useRef(false);

  const doImport = async (paths: string[], dir: string) => {
    if (importingRef.current) return;
    importingRef.current = true;
    setImporting(true);
    try {
      // Real names on disk, not just the ones the tree has cached: a collision
      // with an unloaded sibling still has to be renamed around.
      const existing = await listDir(root, dir).catch(() => [] as DirEntry[]);
      const taken = new Set(existing.map((e) => e.name.toLowerCase()));
      const renamed: string[] = [];
      const added: string[] = [];
      for (const src of paths) {
        const want = dropName(src);
        const name = uniqueName(taken, want);
        try {
          await importFile(root, src, joinPath(dir, name));
        } catch (e) {
          toastError(e, `Couldn't add ${want}`);
          continue;
        }
        taken.add(name.toLowerCase());
        added.push(joinPath(dir, name));
        if (name !== want) renamed.push(name);
      }
      if (added.length === 0) return;
      if (dir !== "") expand(dir);
      await loadDir(dir);
      if (renamed.length > 0) toastInfo(`Renamed to keep what was there: ${nameList(renamed)}`);
      // One file is an "open this" gesture; a batch is not, and stealing the
      // editor for an arbitrary member of it would be noise.
      if (added.length === 1) onSelect(added[0]);
    } finally {
      importingRef.current = false;
      setImporting(false);
    }
  };

  const dropDir = useFileDrop(
    (x, y) => dirAtPoint(bodyRef.current, x, y),
    (paths, dir) => { void doImport(paths, dir); },
  );

  const addGitignore = async (path: string) => {
    // A first-time add creates .gitignore at the root — refresh so it shows.
    if (await ignorePath(root, path)) await loadDir("");
  };

  const openMenu = (e: React.MouseEvent, entry: { path: string; isDir: boolean } | null) => {
    e.preventDefault();
    e.stopPropagation();
    const dir = containerOf(entry);
    const items: MenuEntry[] = [
      { label: "New File…", onClick: () => setDialog({ kind: "newFile", dir }) },
      { label: "New Folder…", onClick: () => setDialog({ kind: "newFolder", dir }) },
    ];
    if (entry) {
      items.push(
        { kind: "separator" },
        { label: "Rename…", onClick: () => setDialog({ kind: "rename", orig: entry.path }) },
        { label: "Delete", danger: true, onClick: () => setConfirmDel({ path: entry.path, isDir: entry.isDir }) },
        { kind: "separator" },
        { label: "Add to .gitignore", onClick: () => addGitignore(entry.path) },
        { label: revealLabel, onClick: () => reveal(root, entry.path) },
        { label: "Copy Path", onClick: () => copyAbsPath(root, entry.path) },
        { label: "Copy Relative Path", onClick: () => copyRelPath(entry.path) },
      );
    } else {
      items.push(
        { kind: "separator" },
        { label: revealLabel, onClick: () => reveal(root, "") },
      );
    }
    setMenu({ x: e.clientX, y: e.clientY, items });
  };

  // Recursively emit the visible rows for a directory's contents.
  const renderDir = (dir: string, depth: number): React.ReactNode[] => {
    const entries = cache.get(dir);
    const rows: React.ReactNode[] = [];
    if (dir !== "" && !open.has(dir)) return rows;
    const err = dir === "" ? rootError : errors.get(dir);
    if (err) {
      rows.push(<div key={`${dir}!err`} className="tree-row error" style={{ paddingLeft: 8 + depth * 12 }}>{err}</div>);
      return rows;
    }
    if (!entries) {
      rows.push(<div key={`${dir}!load`} className="tree-row" style={{ paddingLeft: 8 + depth * 12 }}>loading…</div>);
      return rows;
    }
    for (const c of entries) {
      const path = joinPath(dir, c.name);
      const isOpen = open.has(path);
      const pad = { paddingLeft: 8 + depth * 12 };
      if (c.isDir) {
        rows.push(
          <div
            key={path}
            className={`tree-row dir${dropDir === path ? " drop-into" : ""}`}
            style={pad}
            data-path={path}
            data-drop-dir={path}
            onClick={() => toggle(path)}
            onContextMenu={(e) => openMenu(e, { path, isDir: true })}
          >
            <span className="tree-twistie-slot">
              {(c.hasChildren || isOpen) ? <Twistie open={isOpen} /> : null}
            </span>
            <span className="tree-icon file-icon" style={{ color: "var(--blue)" }}>
              <FileIcon kind="folder" open={isOpen} />
            </span>
            <span className="tree-name">{c.name}</span>
          </div>,
        );
        if (isOpen) rows.push(...renderDir(path, depth + 1));
      } else {
        const { kind, color } = fileIcon(c.name);
        rows.push(
          <div
            key={path}
            className={`tree-row file ${selected === path ? "on" : ""}`}
            style={pad}
            data-path={path}
            data-drop-dir={dir}
            onClick={() => onSelect(path)}
            onContextMenu={(e) => openMenu(e, { path, isDir: false })}
          >
            <span className="tree-twistie-slot" />
            <span className="tree-icon file-icon" style={{ color }}>
              <FileIcon kind={kind} />
            </span>
            <span className="tree-name">{c.name}</span>
          </div>,
        );
      }
    }
    return rows;
  };

  // Find-in-files: debounced 150 ms against the backend search primitive, with
  // a token guarding stale slow responses (same pattern as DocsTree search).
  const [hits, setHits] = useState<BackendSearchHit[]>([]);
  const [searchFailed, setSearchFailed] = useState(false);
  const searchToken = useRef(0);
  useEffect(() => {
    const q = query.trim();
    const token = ++searchToken.current;
    setSearchFailed(false);
    if (!q) {
      setHits([]);
      return;
    }
    const t = window.setTimeout(async () => {
      try {
        const res = await searchFiles(root, "", { query: q });
        if (searchToken.current === token) setHits(res);
      } catch {
        if (searchToken.current === token) {
          setHits([]);
          setSearchFailed(true);
        }
      }
    }, 150);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, rootKey]);

  // Hits grouped by file, preserving the backend's order.
  const hitGroups = useMemo(() => {
    const m = new Map<string, BackendSearchHit[]>();
    for (const h of hits) {
      const g = m.get(h.path);
      if (g) g.push(h);
      else m.set(h.path, [h]);
    }
    return [...m.entries()];
  }, [hits]);

  const searching = query.trim() !== "";

  return (
    <>
      <div className="docs-search">
        <input
          className="docs-search-input"
          placeholder="Search in files…"
          value={query}
          onChange={(e) => onQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Escape") onQuery(""); }}
        />
      </div>
      <div className="files-root-label">
        {/* The tree's own root. Which checkout that is belongs to the bar above
            (it has the room to name the branch too), so this row stays a path. */}
        <span className="files-root-name" title="Repository root">/</span>
        <span className="spacer" style={{ flex: 1 }} />
        <button className="files-tool-btn" title="New File" onClick={() => setDialog({ kind: "newFile", dir: "" })}>
          <NewFileGlyph />
        </button>
        <button className="files-tool-btn" title="New Folder" onClick={() => setDialog({ kind: "newFolder", dir: "" })}>
          <NewFolderGlyph />
        </button>
        <button className="files-tool-btn" title="Refresh" onClick={() => { setCache(new Map()); setErrors(new Map()); setRootError(""); loadDir(""); [...open].forEach(loadDir); }}>
          <RefreshGlyph />
        </button>
        <button className="files-tool-btn" title="Collapse folders" onClick={() => setOpen(new Set())}>
          <CollapseGlyph />
        </button>
      </div>
      <div
        ref={bodyRef}
        className={`files-tree-body${dropDir !== null ? " drop-active" : ""}${dropDir === "" ? " drop-into" : ""}`}
        data-drop-dir=""
        onContextMenu={(e) => { if (e.target === e.currentTarget) openMenu(e, null); }}
      >
        {searching ? (
          <div className="files-search-results">
            {searchFailed && <div className="docs-search-none">Search failed</div>}
            {!searchFailed && hitGroups.length === 0 && <div className="docs-search-none">No matches</div>}
            {hitGroups.map(([path, group]) => {
              const { kind, color } = fileIcon(baseName(path));
              return (
                <div key={path} className="files-search-group">
                  <div className="files-search-file" title={path} onClick={() => onOpenHit(path, group[0].line)}>
                    <span className="tree-icon file-icon" style={{ color }}>
                      <FileIcon kind={kind} />
                    </span>
                    <span className="files-search-path">{path}</span>
                  </div>
                  {group.map((h) => (
                    <div
                      key={`${h.line}:${h.col}`}
                      className="files-search-line"
                      onClick={() => onOpenHit(path, h.line)}
                    >
                      <span className="files-search-ln">{h.line}</span>
                      <span className="files-search-text">{h.text.trim()}</span>
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        ) : (
          renderDir("", 0)
        )}
      </div>

      {(dropDir !== null || importing) && (
        <div className="tree-drop-hint">
          {importing ? "Adding…" : `Drop into ${dropDir ? `${dropDir}/` : "/"}`}
        </div>
      )}

      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}

      {dialog && dialog.kind !== "rename" && (
        <PromptDialog
          title={dialog.kind === "newFile" ? "New File" : "New Folder"}
          body={dialog.dir ? `in ${dialog.dir}/` : undefined}
          placeholder={dialog.kind === "newFile" ? "name.ext" : "folder name"}
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
          title={confirmDel.isDir ? "Delete folder?" : "Delete file?"}
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

// ── Toolbar glyphs (match the stroked-SVG convention in icons.tsx) ──────
const gp = {
  width: 15, height: 15, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor",
  strokeWidth: 1.9, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, "aria-hidden": true,
};
function NewFileGlyph() {
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
function RefreshGlyph() {
  return (
    <svg {...gp}>
      <polyline points="21 3 21 9 15 9" />
      <path d="M20 13a8 8 0 1 1-2.3-6.7L21 9" />
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
