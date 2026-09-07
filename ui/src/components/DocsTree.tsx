import { useEffect, useMemo, useRef, useState } from "react";
import {
  DirEntry, FileRoot, absPath, createFile, createDir, importFile, listDir, renamePath, searchFiles,
  trashPath, writeFile,
} from "../api";
import { DocsIndex, SearchHit, fmFilterPaths, mergeBodyHits, searchDocs, searchLocal, stripExt } from "../lib/docsIndex";
import { parseDocsQuery } from "../lib/docsQuery";
import { joinPath, parentPath, baseName } from "../lib/filePath";
import { TreeDir, allDirPaths, buildDocsTree } from "../lib/docsTree";
import { fileIcon } from "../lib/fileIcon";
import { FileIcon } from "./fileIcons";
import Menu, { MenuEntry } from "./git/Menu";
import PromptDialog from "./PromptDialog";
import ConfirmDialog from "./ConfirmDialog";
import { toastError, toastInfo } from "../lib/toast";
import { dirAtPoint, useFileDrop } from "../hooks/useFileDrop";
import { dropName, isMarkdown, nameList, uniqueName } from "../lib/fileDrop";
import { revealLabel, reveal, copyAbsPath } from "../lib/fileActions";
import { PathSink, createSinkTracker } from "../lib/pathDrop";

type Dialog =
  | { kind: "newNote"; dir: string }
  | { kind: "newFolder"; dir: string }
  | { kind: "rename"; orig: string };

function Twistie({ open }: { open: boolean }) {
  return (
    <svg className={`tree-twistie ${open ? "open" : ""}`} width="13" height="13" viewBox="0 0 24 24"
      fill="currentColor" aria-hidden>
      <path d="M7 4l10 8-10 8z" />
    </svg>
  );
}

/**
 * The Docs tab's tree + search pane. Renders directly from the index (no extra
 * IPC; the corpus poll keeps it fresh). Folders come from the note paths plus
 * the scan's folder list, so a folder holding no notes (new, or holding only
 * attachments) is a real row rather than something this view has to remember.
 *
 * A vault view, not a notes-only one: the files that aren't notes are dimmed
 * rows here, because a folder of screenshots reported as a folder and nothing
 * else read as empty and was not (AGE-121). Clicking one hands it to the Files
 * tab — this tree's editor is a markdown editor, and a PNG has no business in
 * it — and dropping one in is accepted rather than turned away.
 */
export default function DocsTree({
  root, docsDir, rootLabel, index, selected, query, onQuery, onSelect, onOpenFile, onAttached,
  onOpenHit, onRenamed, onDeleted, refresh,
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
  /** An attachment row was clicked: rel to the docs dir, for the Files tab. */
  onOpenFile: (path: string) => void;
  /** Attachments just landed in the vault (rel to the docs dir), so the view
   * can offer to link them from whatever note is open. */
  onAttached: (paths: string[]) => void;
  onOpenHit: (hit: SearchHit) => void;
  onRenamed: (from: string, to: string) => void;
  onDeleted: (path: string) => void;
  refresh: () => Promise<void>;
}) {
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [confirmDel, setConfirmDel] = useState<{ path: string; isDir: boolean } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);

  const rootKey = `${root.kind}:${root.id}:${docsDir}`;
  useEffect(() => {
    setOpen(new Set());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rootKey]);

  const tree = useMemo(() => buildDocsTree(index), [index]);
  // Every folder in the tree, so the toolbar button can open them all at once.
  const allDirs = useMemo(() => allDirPaths(tree), [tree]);

  // Drives the toolbar button's two directions. Measured against the live tree
  // rather than `open.size`, so paths left over from deleted folders don't make
  // a fully collapsed tree look open.
  const anyOpen = allDirs.some((p) => open.has(p));

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
        // Empty folders are part of the scan, so the new one comes back from
        // disk and survives the tab switch that used to make it disappear.
        await refresh();
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
      await refresh();
      onDeleted(path);
    } catch (e) {
      toastError(e, "Delete failed");
    }
  };

  // ── dropping in from Finder ───────────────────────────────────────────────
  // Everything lands in the folder under the cursor (blank space below the
  // rows is the docs root). Markdown becomes a note; anything else becomes an
  // attachment row — the tree used to refuse those and point at the Files tab,
  // which was the wrong answer for the screenshot the open note wants to link
  // to (AGE-121). Copies, never moves.

  const bodyRef = useRef<HTMLDivElement>(null);
  const [importing, setImporting] = useState(false);
  // The drop listener is installed once, so it would otherwise read `importing`
  // from the render that installed it.
  const importingRef = useRef(false);

  const doImport = async (paths: string[], dir: string) => {
    if (importingRef.current) return;
    importingRef.current = true;
    setImporting(true);
    try {
      // Real names on disk, not the index's: it knows notes and attachments
      // but not dotfiles, and a collision with one of those is still a
      // collision.
      const existing = await listDir(root, toRepo(dir)).catch(() => [] as DirEntry[]);
      const taken = new Set(existing.map((e) => e.name.toLowerCase()));
      const renamed: string[] = [];
      const notes: string[] = [];
      const files: string[] = [];
      for (const src of paths) {
        const want = dropName(src);
        const name = uniqueName(taken, want);
        try {
          await importFile(root, src, toRepo(joinPath(dir, name)));
        } catch (e) {
          toastError(e, `Couldn't add ${want}`);
          continue;
        }
        taken.add(name.toLowerCase());
        (isMarkdown(name) ? notes : files).push(joinPath(dir, name));
        if (name !== want) renamed.push(name);
      }
      if (notes.length === 0 && files.length === 0) return;
      if (dir !== "") setOpen((s) => new Set(s).add(dir));
      await refresh();
      if (renamed.length > 0) toastInfo(`Renamed to keep what was there: ${nameList(renamed)}`);
      // The corpus scan leaves non-markdown dotfiles out (`.DS_Store` as a row
      // is nobody's attachment), so one that just landed has no row to appear
      // in. Said out loud, because the file is on disk either way.
      const hidden = files.map(baseName).filter((n) => n.startsWith("."));
      if (hidden.length > 0) {
        toastInfo(`${nameList(hidden)} landed in the folder, but the tree doesn't list dotfiles. Use the Files tab.`);
      }
      // One note is an "open this" gesture; a batch is not. An attachment is
      // never one — it opens in another tab, so a drop would yank the user out
      // of the note they are writing.
      if (notes.length === 1 && files.length === 0) onSelect(notes[0]);
      if (files.length > 0) onAttached(files);
    } finally {
      importingRef.current = false;
      setImporting(false);
    }
  };

  const dropDir = useFileDrop(
    (x, y) => dirAtPoint(bodyRef.current, x, y),
    (paths, dir) => { void doImport(paths, dir); },
  );

  // ── dragging a row out to an agent ───────────────────────────────────────
  // Pointer-based (mousedown → 5px threshold → track → commit on mouseup), NOT
  // HTML5 drag-and-drop, for the reason the Files tree spells out: Tauri
  // intercepts drops at the NSView level, so an in-page HTML5 drag lifts and
  // its drop event never fires.
  //
  // Only outward. This tree has no drop target of its own — notes are moved
  // through Rename — so every drag either lands on a terminal or does nothing.

  const [sink, setSink] = useState<PathSink | null>(null);
  const sinkLive = useRef<PathSink | null>(null);
  // A completed drag must not read as a click on the row it started from.
  const suppressClick = useRef(false);

  const onRowMouseDown = (path: string, e: React.MouseEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, input, textarea")) return;
    // Suppresses the text selection WebKit would otherwise start dragging.
    e.preventDefault();
    const start = { x: e.clientX, y: e.clientY };
    const track = createSinkTracker();
    let started = false;
    let cancelled = false;

    const finish = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("keydown", onKey, true);
      track.clear();
      sinkLive.current = null;
      setSink(null);
    };
    const onMove = (ev: MouseEvent) => {
      if (cancelled) return;
      if (!started) {
        if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < 5) return;
        started = true;
        window.getSelection()?.removeAllRanges();
      }
      ev.preventDefault();
      sinkLive.current = track.over(ev.clientX, ev.clientY);
      setSink(sinkLive.current);
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      cancelled = true;
      track.clear();
      sinkLive.current = null;
      setSink(null);
    };
    const onUp = () => {
      const target = sinkLive.current;
      finish();
      if (started) {
        suppressClick.current = true;
        window.setTimeout(() => { suppressClick.current = false; }, 0);
      }
      if (target && !cancelled) void dropOnSink(target, path);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    // Capture phase so an Escape mid-drag can't reach anything else.
    window.addEventListener("keydown", onKey, true);
  };

  // Absolute, as a Finder drop would be: the agent may be sitting anywhere,
  // and a vault outside the repo has no relative path that reaches it at all.
  const dropOnSink = async (target: PathSink, path: string) => {
    try {
      target.accept([await absPath(root, toRepo(path))]);
    } catch (e) {
      toastError(e, "Couldn't resolve that path");
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
        <div key={sub.path} className={`tree-row dir${dropDir === sub.path ? " drop-into" : ""}`} style={pad}
          data-drop-dir={sub.path}
          onMouseDown={(e) => onRowMouseDown(sub.path, e)}
          onClick={() => { if (!suppressClick.current) toggle(sub.path); }}
          onContextMenu={(e) => openMenu(e, { path: sub.path, isDir: true })}>
          <span className="tree-twistie-slot">
            {/* Nothing to disclose in a folder with nothing in it yet (same
                rule as the Files tree). */}
            {sub.notes.length > 0 || sub.files.length > 0 || sub.dirs.size > 0 || isOpen
              ? <Twistie open={isOpen} />
              : null}
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
          data-drop-dir={dir.path}
          onMouseDown={(e) => onRowMouseDown(note.path, e)}
          onClick={() => { if (!suppressClick.current) onSelect(note.path); }}
          onContextMenu={(e) => openMenu(e, { path: note.path, isDir: false })}>
          <span className="tree-twistie-slot" />
          <span className="tree-icon file-icon" style={{ color: "var(--sub0)" }}>
            <NoteGlyph />
          </span>
          <span className="tree-name" title={note.path}>{note.title}</span>
        </div>,
      );
    }
    for (const file of dir.files) {
      const icon = fileIcon(file.name);
      rows.push(
        <div key={file.path}
          className="tree-row file attachment"
          style={pad}
          data-drop-dir={dir.path}
          onMouseDown={(e) => onRowMouseDown(file.path, e)}
          onClick={() => { if (!suppressClick.current) onOpenFile(file.path); }}
          onContextMenu={(e) => openMenu(e, { path: file.path, isDir: false })}>
          <span className="tree-twistie-slot" />
          <span className="tree-icon file-icon" style={{ color: icon.color }}>
            <FileIcon kind={icon.kind} />
          </span>
          <span className="tree-name" title={file.path}>{file.name}</span>
        </div>,
      );
    }
    return rows;
  };

  // Search: tag/title hits are answered instantly from the index; body hits
  // come from the backend search primitive, debounced 150ms. The token guards
  // against a stale slow response landing over a newer query's results; the
  // in-memory substring scan remains as the error fallback.
  const [hits, setHits] = useState<SearchHit[]>([]);
  const searchToken = useRef(0);
  useEffect(() => {
    const q = query.trim();
    const token = ++searchToken.current;
    if (!index || !q) {
      setHits([]);
      return;
    }
    const local = searchLocal(index, q);
    setHits(local);
    if (q.startsWith("#")) return; // tag queries are fully local
    const { filters, text } = parseDocsQuery(q);
    if (filters.length > 0 && !text) return; // filter-only queries are fully local
    const t = window.setTimeout(async () => {
      let merged: SearchHit[];
      try {
        // The backend only sees the free-text remainder; frontmatter filters
        // restrict which docs its body hits may come from.
        const body = await searchFiles(root, docsDir, { query: text, globs: ["*.md", "*.markdown"] });
        const allowed = filters.length > 0 ? fmFilterPaths(index, filters) : undefined;
        merged = mergeBodyHits(index, local, body, allowed);
      } catch {
        merged = searchDocs(index, q);
      }
      if (searchToken.current === token) setHits(merged);
    }, 150);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [index, query, root.kind, root.id, docsDir]);

  const searching = query.trim() !== "";

  return (
    <>
      <div className="docs-search">
        <input
          className="docs-search-input"
          placeholder="Search notes…  (#tag, key:value)"
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
        <button
          className="files-tool-btn"
          title={anyOpen ? "Collapse folders" : "Expand folders"}
          disabled={allDirs.length === 0}
          onClick={() => setOpen(anyOpen ? new Set() : new Set(allDirs))}
        >
          {anyOpen ? <CollapseGlyph /> : <ExpandGlyph />}
        </button>
      </div>
      <div
        ref={bodyRef}
        className={`files-tree-body${dropDir !== null ? " drop-active" : ""}${dropDir === "" ? " drop-into" : ""}`}
        data-drop-dir=""
        onContextMenu={(e) => { if (e.target === e.currentTarget) openMenu(e, null); }}
      >
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
        ) : index && index.docs.size === 0 && index.dirs.length === 0 && index.attachments.length === 0 ? (
          <div className="docs-search-none">No notes yet. Create one.</div>
        ) : (
          renderDir(tree, 0)
        )}
      </div>

      {(dropDir !== null || sink !== null || importing) && (
        <div className="tree-drop-hint">
          {importing
            ? "Adding…"
            : sink
              ? `Add the path to ${sink.label}`
              : `Drop files into ${dropDir ? `${dropDir}/` : docsDir ? `${docsDir}/` : "/"}`}
        </div>
      )}

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
          title={
            confirmDel.isDir
              ? "Delete folder?"
              : isMarkdown(baseName(confirmDel.path)) ? "Delete note?" : "Delete file?"
          }
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
// The collapse chevrons flipped outward.
function ExpandGlyph() {
  return (
    <svg {...gp}>
      <polyline points="8 8 12 4 16 8" />
      <polyline points="8 16 12 20 16 16" />
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
