import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  BackendSearchHit, DirEntry, FileRoot, absPath, listDir, searchFiles,
  createFile, createDir, copyPath, importFile, renamePath, trashPath,
} from "../api";
import { ancestorDirs, joinPath, parentPath, baseName } from "../lib/filePath";
import { fileIcon } from "../lib/fileIcon";
import { FileIcon } from "./fileIcons";
import Menu, { MenuEntry } from "./git/Menu";
import PromptDialog from "./PromptDialog";
import ConfirmDialog from "./ConfirmDialog";
import { dirAtPoint, useFileDrop } from "../hooks/useFileDrop";
import { dropName, nameList, uniqueName } from "../lib/fileDrop";
import { sameListing, visibleDirs } from "../lib/dirListing";
import { Transfer, transferProblem } from "../lib/fileTransfer";
import { toastError, toastInfo } from "../lib/toast";
import { revealLabel, reveal, copyAbsPath, copyRelPath, ignorePath } from "../lib/fileActions";
import { PathSink, createSinkTracker } from "../lib/pathDrop";

/** One row of the tree, as the menu, the keyboard cursor and the drag see it. */
type Entry = { path: string; isDir: boolean };

/** A move or copy in flight, and where it would land if released now. */
type Drag = {
  src: string;
  /** Directory under the cursor, "" for the tree root. */
  dir: string;
  mode: Transfer["mode"];
  /** Why this drop would be refused, or null. Shown in the hint. */
  problem: string | null;
};

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
  // What ⌘X/⌘C put aside, waiting for a ⌘V. A cut is not applied until the
  // paste, so the file stays where it is (and stays openable) until it lands.
  const [clip, setClip] = useState<Transfer | null>(null);
  // The row the keyboard acts on. Set by clicking or right-clicking a row, so
  // "copy this" means the row you just touched rather than the file that
  // happens to be open in the editor.
  const [cursor, setCursor] = useState<Entry | null>(null);

  const rootKey = `${root.kind}:${root.id}`;
  // The scrolling tree body: the drop target below, and what a reveal scrolls.
  const bodyRef = useRef<HTMLDivElement>(null);
  // The reveal walk below runs in a promise chain, where `cache` would be
  // whatever it was when the effect started.
  const cacheRef = useRef(cache);
  cacheRef.current = cache;
  // Ditto for the refresh sweep, which is installed once and would otherwise
  // re-list whatever was expanded when it was installed.
  const openRef = useRef(open);
  openRef.current = open;

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

  // Re-list a directory that is already on screen, writing to state only when
  // the listing actually changed. `loadDir`'s unconditional write is right for
  // a load the user asked for; on a two-second sweep it would re-render the
  // whole tree forever, losing hover and interrupting the row being dragged.
  //
  // A failed sweep leaves the last good listing standing rather than replacing
  // the rows with an error: the directory may simply have been deleted, and its
  // parent's sweep is what makes the row go away.
  const refreshDir = useCallback((path: string) => {
    return listDir(root, path)
      .then((entries) => {
        setCache((m) => {
          const prev = m.get(path);
          return prev && sameListing(prev, entries) ? m : new Map(m).set(path, entries);
        });
        setErrors((m) => {
          if (!m.has(path)) return m;
          const n = new Map(m);
          n.delete(path);
          return n;
        });
      })
      .catch(() => {});
  }, [root]);
  // `root` is a fresh object on every render of the view above, so `refreshDir`
  // is too. The sweep below must not be keyed on it: it would tear its own
  // interval down and rebuild it faster than the interval ever fires.
  const refreshRef = useRef(refreshDir);
  refreshRef.current = refreshDir;

  // Nothing tells the tree when the files under it change: an agent writing in
  // the worktree and the user renaming something in Finder both land behind its
  // back, and until AGE-162 the rows stayed as they were until the Files tab was
  // left and re-entered (which remounts this whole component). So the visible
  // rows are re-listed on a timer, and again the moment the window comes back —
  // the tick that matters after a detour through Finder.
  useEffect(() => {
    const sweep = () => {
      for (const d of visibleDirs(cacheRef.current, openRef.current)) void refreshRef.current(d);
    };
    // Nobody is reading the tree while another app is in front, and that is
    // exactly when the interesting changes are being made. The focus listener
    // below is what catches up on them.
    const tick = () => { if (document.hasFocus()) sweep(); };
    const t = window.setInterval(tick, 2000);
    window.addEventListener("focus", sweep);
    return () => {
      window.clearInterval(t);
      window.removeEventListener("focus", sweep);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rootKey]);

  // Re-root whenever the FileRoot changes (focused agent ↔ project main).
  useEffect(() => {
    setCache(new Map());
    setOpen(new Set());
    setErrors(new Map());
    setRootError("");
    setCursor(null);
    setClip(null);
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

  // The directory a create or a paste should land in: a folder targets itself, a
  // file its parent, and no target means the root.
  const containerOf = (entry: Entry | null) =>
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

  // ── moving and copying within the tree ───────────────────────────────────
  // One operation behind three gestures: the paste, the drag, and Duplicate.
  // A move is a rename, so it retargets open tabs the same way the rename
  // dialog does; a copy leaves the source alone and has nothing to retarget.

  // Whether a path in this tree is a folder, read off the listing that holds
  // it: the drag and the paste both know their source only as a path.
  const isDirPath = (path: string) =>
    (cacheRef.current.get(parentPath(path)) ?? []).some(
      (e) => e.name === baseName(path) && e.isDir,
    );

  /** Resolves true only if the file actually landed at its destination. */
  const doTransfer = async (src: string, dir: string, mode: Transfer["mode"]) => {
    const problem = transferProblem(src, dir, mode);
    if (problem) {
      toastInfo(problem);
      return false;
    }
    const want = baseName(src);
    // Read before the move: afterwards the listing that knew is already gone.
    const isDir = isDirPath(src);
    try {
      // Real names on disk, not the tree's cache: pasting into a folder nobody
      // has expanded still has to be renamed around what is already in it.
      const existing = await listDir(root, dir).catch(() => [] as DirEntry[]);
      const name = uniqueName(new Set(existing.map((e) => e.name.toLowerCase())), want);
      const dest = joinPath(dir, name);
      if (mode === "move") await renamePath(root, src, dest);
      else await copyPath(root, src, dest);
      if (dir !== "") expand(dir);
      await loadDir(dir);
      if (mode === "move") {
        await loadDir(parentPath(src));
        // The owner retargets open tabs/selection (and their unsaved buffers).
        onRenamed(src, dest);
      }
      // Follow the thing that just moved, so a second ⌘V goes where the eye is.
      setCursor({ path: dest, isDir });
      if (name !== want) toastInfo(`Renamed to keep what was there: ${name}`);
      return true;
    } catch (e) {
      toastError(e, mode === "move" ? "Move failed" : "Copy failed");
      return false;
    }
  };

  const doPaste = (dir: string) => {
    if (!clip) return;
    const { mode, path } = clip;
    void doTransfer(path, dir, mode).then((landed) => {
      // A cut is spent once it lands: its source no longer exists. A copy
      // stays, so the same file can be pasted into several places. A paste
      // that was refused or failed keeps the clipboard either way, so the
      // next attempt still has something to paste.
      if (landed && mode === "move") setClip(null);
    });
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

  // ── dragging a row onto a folder ─────────────────────────────────────────
  // Pointer-based (mousedown → 5px threshold → track → commit on mouseup), NOT
  // HTML5 drag-and-drop: Tauri's native drag-drop layer intercepts drops at the
  // NSView level on macOS, so an in-page HTML5 drag lifts but its drop event
  // never fires. Same reason the issue board reorders this way.
  //
  // Plain drag moves and ⌥-drag copies, as in Finder. The modifier is read at
  // each move rather than at the drop, so the hint below says which one is
  // about to happen.

  const [drag, setDrag] = useState<Drag | null>(null);
  // Mouse events outrun React renders, so the live value is a ref and `drag`
  // only mirrors it for the hint and the target highlight.
  const dragLive = useRef<Drag | null>(null);
  // The same drag, when it has left the tree and is over a terminal instead:
  // released there it types the path at the agent's prompt rather than moving
  // the file anywhere (AGE-200). Mutually exclusive with `drag` — a point is
  // either inside the tree or outside it.
  const [sink, setSink] = useState<PathSink | null>(null);
  const sinkLive = useRef<PathSink | null>(null);
  // A completed drag must not read as a click on the row it started from.
  const suppressClick = useRef(false);

  // A row released over a terminal: hand it the path the way a Finder drop
  // would, absolute, so it means the same thing whatever directory the agent
  // is sitting in. Resolved here rather than at mousedown because a drag that
  // ends in the tree never needs it.
  const dropOnSink = async (target: PathSink, path: string) => {
    try {
      target.accept([await absPath(root, path)]);
    } catch (e) {
      toastError(e, "Couldn't resolve that path");
    }
  };

  const onRowMouseDown = (entry: Entry, e: React.MouseEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, input, textarea")) return;
    setCursor(entry);
    // The keyboard shortcuts live on the tree body, so a grabbed row has to
    // bring focus with it. preventDefault below suppresses the focus WebKit
    // would otherwise move (and the text selection it would start dragging).
    bodyRef.current?.focus({ preventScroll: true });
    e.preventDefault();
    const start = { x: e.clientX, y: e.clientY };
    let started = false;
    // Escape sets this so the drag can't silently restart on the next
    // mousemove; the still-held button then releases as a no-op.
    let cancelled = false;

    const track = createSinkTracker();

    const onMove = (ev: MouseEvent) => {
      if (cancelled) return;
      if (!started) {
        if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < 5) return;
        started = true;
        window.getSelection()?.removeAllRanges();
      }
      ev.preventDefault();
      const dir = dirAtPoint(bodyRef.current, ev.clientX, ev.clientY);
      const mode: Transfer["mode"] = ev.altKey ? "copy" : "move";
      dragLive.current = dir === null
        ? null
        : { src: entry.path, dir, mode, problem: transferProblem(entry.path, dir, mode) };
      setDrag(dragLive.current);
      // Only once the pointer has left the tree: inside it, the tree's own
      // drop rules win, and a terminal underneath is not a target.
      if (dir === null) {
        sinkLive.current = track.over(ev.clientX, ev.clientY);
      } else {
        track.clear();
        sinkLive.current = null;
      }
      setSink(sinkLive.current);
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      cancelled = true;
      dragLive.current = null;
      sinkLive.current = null;
      track.clear();
      setDrag(null);
      setSink(null);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("keydown", onKey, true);
      const d = dragLive.current;
      const s = sinkLive.current;
      dragLive.current = null;
      sinkLive.current = null;
      track.clear();
      setDrag(null);
      setSink(null);
      if (started) {
        // Neither a completed nor a cancelled drag may open the file or toggle
        // the folder the pointer started on.
        suppressClick.current = true;
        window.setTimeout(() => { suppressClick.current = false; }, 0);
      }
      if (cancelled) return;
      if (s) {
        void dropOnSink(s, entry.path);
        return;
      }
      // A refused drop is a no-op: the hint already said why while it hovered.
      if (!d || d.problem) return;
      void doTransfer(d.src, d.dir, d.mode);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    // Capture phase so an Escape mid-drag can't reach anything else.
    window.addEventListener("keydown", onKey, true);
  };

  // ⌘X/⌘C/⌘V over the tree. Bound to the body, so they only fire while it holds
  // focus and never while the editor or a terminal does. macOS validates the
  // Edit menu's own Cut/Copy/Paste against the webview first, and those are
  // disabled when nothing editable has focus and nothing is selected — which is
  // exactly the state the tree is in, so the keystroke falls through to here.
  const onTreeKey = (e: React.KeyboardEvent) => {
    if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
    const key = e.key.toLowerCase();
    if (key === "c" || key === "x") {
      if (!cursor) return;
      e.preventDefault();
      setClip({ mode: key === "c" ? "copy" : "move", path: cursor.path });
    } else if (key === "v") {
      if (!clip) return;
      e.preventDefault();
      doPaste(containerOf(cursor));
    }
  };

  const addGitignore = async (path: string) => {
    // A first-time add creates .gitignore at the root — refresh so it shows.
    if (await ignorePath(root, path)) await loadDir("");
  };

  const openMenu = (e: React.MouseEvent, entry: Entry | null) => {
    e.preventDefault();
    e.stopPropagation();
    setCursor(entry);
    const dir = containerOf(entry);
    // Pasting is offered even when it can't be done, greyed out — the reason it
    // is greyed (a folder into itself, a move that goes nowhere) is worth more
    // than an item that silently isn't there.
    const paste: MenuEntry = {
      label: "Paste",
      hint: "⌘V",
      disabled: !clip || transferProblem(clip.path, dir, clip.mode) !== null,
      onClick: () => doPaste(dir),
    };
    const items: MenuEntry[] = [
      { label: "New File…", onClick: () => setDialog({ kind: "newFile", dir }) },
      { label: "New Folder…", onClick: () => setDialog({ kind: "newFolder", dir }) },
    ];
    if (entry) {
      items.push(
        { kind: "separator" },
        { label: "Cut", hint: "⌘X", onClick: () => setClip({ mode: "move", path: entry.path }) },
        { label: "Copy", hint: "⌘C", onClick: () => setClip({ mode: "copy", path: entry.path }) },
        paste,
        { label: "Duplicate", onClick: () => void doTransfer(entry.path, parentPath(entry.path), "copy") },
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
        paste,
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
      // Cursor and cut state are per-row chrome, not per-kind: a folder shows
      // them the same way a file does.
      const mark =
        (cursor?.path === path ? " cursor" : "") +
        (clip?.mode === "move" && clip.path === path ? " cut" : "");
      if (c.isDir) {
        rows.push(
          <div
            key={path}
            className={`tree-row dir${mark}${dropInto === path ? " drop-into" : ""}`}
            style={pad}
            data-path={path}
            data-drop-dir={path}
            onMouseDown={(e) => onRowMouseDown({ path, isDir: true }, e)}
            onClick={() => { if (!suppressClick.current) toggle(path); }}
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
            className={`tree-row file${mark}${selected === path ? " on" : ""}`}
            style={pad}
            data-path={path}
            data-drop-dir={dir}
            onMouseDown={(e) => onRowMouseDown({ path, isDir: false }, e)}
            onClick={() => { if (!suppressClick.current) onSelect(path); }}
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

  // Both kinds of drag paint the same target: one from Finder (`dropDir`), one
  // from inside the tree. A refused in-tree drop highlights nothing, so the row
  // under the cursor never looks like it would accept it. A drag that has left
  // for a terminal lights nothing here either: the target is over there, and the
  // terminal lights itself.
  const dropActive = drag !== null || dropDir !== null;
  const dropInto = drag ? (drag.problem ? null : drag.dir) : dropDir;

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
        className={`files-tree-body${dropActive ? " drop-active" : ""}${dropInto === "" ? " drop-into" : ""}`}
        data-drop-dir=""
        // Focusable so ⌘X/⌘C/⌘V can be scoped to the tree; -1 keeps it out of
        // the tab order, since the rows themselves are not tab stops either.
        tabIndex={-1}
        onKeyDown={onTreeKey}
        onMouseDown={(e) => { if (e.target === e.currentTarget) setCursor(null); }}
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

      {(dropActive || sink !== null || importing) && (
        <div className={`tree-drop-hint${drag?.problem ? " refused" : ""}`}>
          {importing
            ? "Adding…"
            : sink
              ? `Add the path to ${sink.label}`
              : drag
                ? drag.problem ?? `${drag.mode === "copy" ? "Copy" : "Move"} into ${dirLabel(drag.dir)}`
                : `Drop into ${dirLabel(dropDir ?? "")}`}
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

/** A directory as the drop hint names it: the root is "/". */
const dirLabel = (dir: string) => (dir ? `${dir}/` : "/");

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
