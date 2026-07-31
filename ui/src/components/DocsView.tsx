import { useEffect, useMemo, useRef, useState } from "react";
import { FileRoot, Project, createFile, writeFile } from "../api";
import { SearchHit } from "../lib/docsIndex";
import { buildLinkIndex, mentionsOf, noteId, resolveTarget } from "../lib/links";
import { requestNavigate } from "../lib/navigate";
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";
import { useDocs } from "../hooks/useDocs";
import { useCrossRefs } from "../hooks/useCrossRefs";
import DocsTree from "./DocsTree";
import DocsEditor, { DocsEditorHandle } from "./DocsEditor";
import DocsSidePanel from "./DocsSidePanel";
import DocsQuickSwitcher from "./DocsQuickSwitcher";
import ConfirmDialog from "./ConfirmDialog";
import { toastError, toastInfo } from "../lib/toast";
import { joinPath } from "../lib/filePath";
import { adjacentDailyPath, isDailyNotePath } from "../lib/dailyNote";
import { recordActivation } from "../lib/recency";

const lastNoteKey = (projectId: string) => `docs:last:${projectId}`;

/**
 * The Docs tab: an Obsidian-lite over the project's `docs` folder. Always
 * rooted at the project's main checkout — docs are a project-level artifact
 * and shouldn't shift with the focused agent.
 */
export default function DocsView({ project }: { project: Project }) {
  const treePane = usePaneWidth("docs-tree", 240, 180, 480);
  const sidePane = usePaneWidth("docs-side", 240, 180, 420);
  const root: FileRoot = { kind: "project", id: project.id };
  const { docsDir, index, refresh, createDocsDir } = useDocs(project.id, true);
  const { cross } = useCrossRefs(true);
  const [selected, setSelectedState] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [sideOpen, setSideOpen] = useState(true);
  const [switcher, setSwitcher] = useState(false);
  const editorRef = useRef<DocsEditorHandle | null>(null);
  const restoredRef = useRef(false);

  // Cmd+P opens the quick switcher — active while the Docs tab is mounted,
  // including from inside the editor. (Verified free of menu/shortcut clashes.)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "p" && !e.shiftKey && !e.altKey) {
        e.preventDefault();
        setSwitcher((s) => !s);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const setSelected = (path: string | null) => {
    setSelectedState(path);
    try {
      if (path) localStorage.setItem(lastNoteKey(project.id), path);
      else localStorage.removeItem(lastNoteKey(project.id));
    } catch { /* storage unavailable */ }
    // Feed the palette's recents. Only user-driven opens land here — the
    // last-note restore uses setSelectedState directly and stays silent.
    if (path) {
      recordActivation(`note:${project.id}:${path}`, index?.docs.get(path)?.title ?? path, path);
    }
  };

  // Reset on project switch; the last-open note is restored once the index has
  // loaded (so a stale path can be validated and dropped).
  useEffect(() => {
    setSelectedState(null);
    setQuery("");
    restoredRef.current = false;
  }, [project.id]);

  useEffect(() => {
    if (restoredRef.current || !index) return;
    restoredRef.current = true;
    let stored: string | null = null;
    try { stored = localStorage.getItem(lastNoteKey(project.id)); } catch { /* ignore */ }
    if (stored && index.docs.has(stored)) {
      setSelectedState(stored);
      return;
    }
    // First visit (or the stored note is gone): open the first note by title.
    const first = [...index.docs.values()].sort((a, b) => a.title.localeCompare(b.title))[0];
    if (first) setSelectedState(first.path);
  }, [index, project.id]);

  // "Today's note" (⌘⇧D / menu / palette) lands here when this view is already
  // mounted; a fresh mount is covered by the docs:last localStorage restore.
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    const onOpen = (e: Event) => {
      const d = (e as CustomEvent<{ projectId: string; path: string }>).detail;
      if (!d || d.projectId !== project.id) return;
      void refreshRef.current().then(() => setSelected(d.path));
    };
    window.addEventListener("agency:open-note", onOpen);
    return () => window.removeEventListener("agency:open-note", onOpen);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.id]);

  // A wikilink pointed at a note that doesn't exist; confirm before creating.
  const [pendingCreate, setPendingCreate] = useState<string | null>(null);

  const navigate = (target: string, heading: string | null) => {
    if (!index) return;
    const res = resolveTarget(index, cross, target);
    switch (res.kind) {
      case "note":
        setSelected(res.path);
        if (heading) setTimeout(() => editorRef.current?.scrollToHeading(heading), 150);
        return;
      case "issue":
        requestNavigate({ kind: "issue", projectId: res.ref.project.id, issueId: res.ref.issue.id });
        return;
      case "run":
        requestNavigate({ kind: "run", projectId: res.ref.project.id, runId: res.ref.run.id });
        return;
      // Inside a known key namespace, a miss means "no such issue", never
      // "create AGE-99.md" (a note there would shadow the tracker).
      case "unresolvedIssue":
        toastInfo(`No issue ${res.label} in any project.`);
        return;
      case "unresolvedRun":
        toastInfo("That run doesn't exist anymore.");
        return;
      case "unresolvedNote":
        setPendingCreate(target);
    }
  };

  const createNote = async (target: string) => {
    if (docsDir == null) return; // "" is valid: the workspace vault root
    // "folder/Note" creates in that folder (if it exists); plain names land at
    // the docs root.
    const path = `${target}.md`;
    try {
      await createFile(root, joinPath(docsDir, path));
      await writeFile(root, joinPath(docsDir, path), `# ${target.split("/").pop()}\n\n`);
      await refresh();
      setSelected(path);
    } catch (e) {
      toastError(e, "Couldn't create note");
    }
  };

  // Issues mentioning the open note, for the side panel's Mentions section.
  // The table spans just this corpus — "issues linking to this note" needs
  // cross-project issue bodies (from useCrossRefs), not other corpora.
  const linkTable = useMemo(
    () => (index && cross ? buildLinkIndex([{ project, index }], cross) : null),
    [index, cross, project],
  );
  const mentions = useMemo(
    () => (linkTable && selected ? mentionsOf(linkTable, "note", noteId(project.id, selected)) : []),
    [linkTable, selected, project.id],
  );

  const openHit = (hit: SearchHit) => {
    setSelected(hit.path);
    // A line hit scrolls once the editor has the note open; heading text is a
    // best-effort anchor, so only title hits (-1) skip it.
    if (hit.line >= 0) {
      const heading = /^#{1,6}\s+(.+)$/.exec(hit.snippet)?.[1];
      if (heading) setTimeout(() => editorRef.current?.scrollToHeading(heading), 150);
    }
  };

  if (docsDir === undefined) {
    return <div className="board empty">loading…</div>;
  }

  if (docsDir === null) {
    return (
      <div className="board empty docs-empty">
        <div className="docs-empty-title">This project has no docs folder.</div>
        <div className="docs-empty-sub">
          Notes live as markdown files in <code>docs/</code> at the repo root, editable here, by agents, or by any other tool.
        </div>
        <button className="btn-primary" onClick={() => void createDocsDir()}>Create docs/</button>
      </div>
    );
  }

  return (
    <div className="files-view docs-view">
      <div className="files-tree docs-tree" style={{ width: treePane.width }}>
        <DocsTree
          root={root}
          docsDir={docsDir}
          rootLabel={docsDir === "" ? project.name : undefined}
          index={index}
          selected={selected}
          query={query}
          onQuery={setQuery}
          onSelect={(p) => { setQuery(""); setSelected(p); }}
          onOpenHit={openHit}
          onRenamed={(from, to) => {
            if (selected === from) setSelected(to);
            else if (selected && selected.startsWith(from + "/")) setSelected(to + selected.slice(from.length));
          }}
          onDeleted={(path) => {
            if (selected === path || (selected && selected.startsWith(path + "/"))) setSelected(null);
          }}
          refresh={refresh}
        />
      </div>
      <Resizer size={treePane.width} min={180} max={480} onChange={treePane.setWidth} />
      <div className="files-editor">
        {selected ? (
          <DocsEditor
            key={`${project.id}:${selected}`}
            ref={editorRef}
            root={root}
            docsDir={docsDir}
            path={selected}
            diskText={index?.docs.get(selected)?.text}
            index={index}
            cross={cross}
            onSaved={() => void refresh()}
            onNavigate={navigate}
            onTagClick={(tag) => setQuery(tag.startsWith("#") ? tag : `#${tag}`)}
            sideOpen={sideOpen}
            onToggleSide={() => setSideOpen((o) => !o)}
            daily={index && isDailyNotePath(selected) ? {
              prev: adjacentDailyPath(index.docs.keys(), selected, "prev"),
              next: adjacentDailyPath(index.docs.keys(), selected, "next"),
              onOpen: setSelected,
            } : null}
          />
        ) : (
          <div className="diff-empty">Select or create a note.</div>
        )}
      </div>
      {sideOpen && selected && (
        <>
          <Resizer size={sidePane.width} min={180} max={420} onChange={sidePane.setWidth} side="right" />
          <div className="docs-side" style={{ width: sidePane.width }}>
            <DocsSidePanel
              index={index}
              selected={selected}
              mentions={mentions}
              onJumpToHeading={(text) => editorRef.current?.scrollToHeading(text)}
              onOpen={(path) => setSelected(path)}
              onOpenMention={(m) => {
                if (m.fromKind === "issue") {
                  requestNavigate({ kind: "issue", projectId: m.fromProjectId, issueId: m.fromId });
                }
              }}
              onFilter={(k, v) => setQuery(/\s/.test(v) ? `${k}:"${v}"` : `${k}:${v}`)}
            />
          </div>
        </>
      )}

      {switcher && (
        <DocsQuickSwitcher
          index={index}
          onOpen={(path) => setSelected(path)}
          onCreate={(name) => void createNote(name)}
          onClose={() => setSwitcher(false)}
        />
      )}

      {pendingCreate !== null && (
        <ConfirmDialog
          title="Create note?"
          body={`"${pendingCreate}" doesn't exist yet. Create ${pendingCreate}.md?`}
          confirmLabel="Create"
          onConfirm={() => { const t = pendingCreate; setPendingCreate(null); void createNote(t); }}
          onCancel={() => setPendingCreate(null)}
        />
      )}
    </div>
  );
}
