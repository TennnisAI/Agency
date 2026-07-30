import { useEffect, useRef, useState } from "react";
import { FileRoot } from "../api";
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";
import FileTree from "./FileTree";
import FileTabs from "./FileTabs";
import FileEditor, { FileEditorHandle } from "./FileEditor";
import ConfirmDialog from "./ConfirmDialog";
import {
  TabState, closeTab, deserializeTabs, emptyTabs, openTab, removeTab, renameTab,
  retargetPath, serializeTabs, setDirtyTab,
} from "../lib/fileTabs";
import { bufferKey, dropBuffer, hasBuffer, stashBuffer, takeBuffer } from "../lib/editorBuffers";
import { consumePendingOpen, onOpenFile } from "../lib/openFile";
import { baseName } from "../lib/filePath";

const tabsKey = (root: FileRoot) => `files:tabs:${root.kind}:${root.id}`;

export default function FilesView({ root, projectName }: { root: FileRoot | null; projectName: string }) {
  const treePane = usePaneWidth("files-tree", 280, 180, 560);
  // Tab state travels WITH the root key it belongs to. The persistence effect
  // below only writes when the state's own key matches the rendered root, so
  // it can never save one root's tabs under another's storage key — and
  // StrictMode's double-run mount effects can't wipe storage with the initial
  // empty state (a plain ref guard set mid-effect fails exactly that way: the
  // second effect pass sees the ref already stamped while `tabs` is still
  // empty, and overwrites the store before the loader re-reads it).
  const [rootTabs, setRootTabs] = useState<{ key: string | null; tabs: TabState }>(
    () => ({ key: null, tabs: emptyTabs() }),
  );
  // Tabs that have mounted an editor this session. Restored tabs are cold —
  // no file read, no CodeMirror instance — until first activation.
  const [warm, setWarm] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [confirmClose, setConfirmClose] = useState<string | null>(null);
  const editorRefs = useRef(new Map<string, FileEditorHandle | null>());
  const rootKey = root ? `${root.kind}:${root.id}` : null;

  // What this render works with: never another root's tabs (the frame between
  // a root switch and its load effect renders empty instead of stale state).
  const tabs = rootTabs.key === rootKey ? rootTabs.tabs : emptyTabs();
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;

  // Every mutation is scoped to the current root; a transition sneaking in
  // between root switch and load is dropped rather than corrupting the
  // outgoing root's state.
  const updateTabs = (fn: (s: TabState) => TabState) =>
    setRootTabs((r) => (r.key === rootKey ? { ...r, tabs: fn(r.tabs) } : r));

  // Persist per root, only once the state actually belongs to it.
  useEffect(() => {
    if (!root || rootTabs.key !== rootKey) return;
    try {
      localStorage.setItem(tabsKey(root), serializeTabs(rootTabs.tabs));
    } catch { /* storage unavailable */ }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rootTabs, rootKey]);

  // (Re)load on root switch. Dirty dots come back from stashed buffers, so an
  // edited-but-cold tab is honest about its unsaved state.
  useEffect(() => {
    editorRefs.current.clear();
    setQuery("");
    setConfirmClose(null);
    if (!root) {
      setRootTabs({ key: null, tabs: emptyTabs() });
      setWarm(new Set());
      return;
    }
    let raw: string | null = null;
    try {
      raw = localStorage.getItem(tabsKey(root));
    } catch { /* storage unavailable */ }
    let s = deserializeTabs(raw);
    for (const p of s.open) {
      if (hasBuffer(bufferKey(root, p))) s = setDirtyTab(s, p, true);
    }
    setRootTabs({ key: rootKey, tabs: s });
    setWarm(new Set(s.active ? [s.active] : []));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rootKey]);

  const openAtLine = (path: string, line?: number) => {
    updateTabs((s) => openTab(s, path));
    setWarm((w) => (w.has(path) ? w : new Set(w).add(path)));
    if (line !== undefined) {
      // Best-effort, same convention as DocsView heading jumps: give a fresh
      // editor a beat to mount and load before scrolling.
      window.setTimeout(() => editorRefs.current.get(path)?.scrollToLine(line), 150);
    }
  };
  const openRef = useRef(openAtLine);
  openRef.current = openAtLine;

  const doClose = (path: string) => {
    // Discard first: the editor unmounts dirty and would otherwise stash the
    // edits the user just chose to throw away.
    editorRefs.current.get(path)?.discard();
    if (root) dropBuffer(bufferKey(root, path));
    editorRefs.current.delete(path);
    updateTabs((s) => closeTab(s, path));
    setWarm((w) => {
      if (!w.has(path)) return w;
      const n = new Set(w);
      n.delete(path);
      return n;
    });
  };
  const requestClose = (path: string) => {
    if (tabsRef.current.dirty.has(path)) setConfirmClose(path);
    else doClose(path);
  };
  const requestCloseRef = useRef(requestClose);
  requestCloseRef.current = requestClose;

  // ⌘W closes the active tab. Bound only while the Files tab is mounted, and
  // menu.rs claims no ⌘W accelerator, so there is nothing to clash with.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "w") {
        const active = tabsRef.current.active;
        if (active) {
          e.preventDefault();
          requestCloseRef.current(active);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Quick-open lands here: live while mounted, pending slot for the mount race.
  useEffect(() => {
    const unsubscribe = onOpenFile((req) => openRef.current(req.path, req.line));
    const pending = consumePendingOpen();
    if (pending) openRef.current(pending.path, pending.line);
    return unsubscribe;
  }, [rootKey]);

  // A rename retargets tabs and migrates unsaved buffers to the new keys; the
  // live editor stashes first so its edits survive the remount at the new path.
  const onRenamed = (from: string, to: string) => {
    if (root) {
      for (const p of tabsRef.current.open) {
        const np = retargetPath(p, from, to);
        if (np === p) continue;
        const editor = editorRefs.current.get(p);
        editor?.stashIfDirty();
        // The old-path editor is about to unmount; without the discard it
        // would re-stash under the old key, leaving a stale buffer that a
        // rename-back would resurrect.
        editor?.discard();
        editorRefs.current.delete(p);
        const stashed = takeBuffer(bufferKey(root, p));
        if (stashed !== null) stashBuffer(bufferKey(root, np), stashed);
      }
    }
    updateTabs((s) => renameTab(s, from, to));
    setWarm((w) => new Set([...w].map((p) => retargetPath(p, from, to))));
  };

  const onDeleted = (path: string) => {
    if (root) {
      for (const p of tabsRef.current.open) {
        if (p !== path && !p.startsWith(path + "/")) continue;
        editorRefs.current.get(p)?.discard(); // deleted on disk — nothing to keep
        editorRefs.current.delete(p);
        dropBuffer(bufferKey(root, p));
      }
    }
    updateTabs((s) => removeTab(s, path));
    setWarm((w) => new Set([...w].filter((p) => p !== path && !p.startsWith(path + "/"))));
  };

  if (!root) {
    return <div className="board empty">Open a project to browse its files.</div>;
  }

  const rootLabel = root.kind === "run" ? "Agent worktree" : `${projectName} · main`;

  return (
    <div className="files-view">
      <div className="files-tree" style={{ width: treePane.width }}>
        <FileTree
          root={root}
          rootLabel={rootLabel}
          selected={tabs.active}
          query={query}
          onQuery={setQuery}
          onSelect={(p) => { if (p) openAtLine(p); }}
          onOpenHit={openAtLine}
          onRenamed={onRenamed}
          onDeleted={onDeleted}
        />
      </div>
      <Resizer size={treePane.width} min={180} max={560} onChange={treePane.setWidth} />
      <div className="files-editor">
        {tabs.open.length > 0 && (
          <FileTabs
            open={tabs.open}
            active={tabs.active}
            dirty={tabs.dirty}
            onActivate={(p) => openAtLine(p)}
            onClose={requestClose}
          />
        )}
        {tabs.open.filter((p) => warm.has(p)).map((p) => (
          <div
            key={`${rootKey}:${p}`}
            className="files-editor-pane"
            style={{ display: p === tabs.active ? "flex" : "none" }}
          >
            <FileEditor
              ref={(h) => { editorRefs.current.set(p, h); }}
              root={root}
              path={p}
              onDirtyChange={(d) => updateTabs((s) => setDirtyTab(s, p, d))}
            />
          </div>
        ))}
        {!tabs.active && <div className="diff-empty">Select a file to view.</div>}
      </div>

      {confirmClose !== null && (
        <ConfirmDialog
          title="Close without saving?"
          body={`"${baseName(confirmClose)}" has unsaved changes. Close the tab and discard them?`}
          confirmLabel="Close"
          danger
          onConfirm={() => { const p = confirmClose; setConfirmClose(null); doClose(p); }}
          onCancel={() => setConfirmClose(null)}
        />
      )}
    </div>
  );
}
