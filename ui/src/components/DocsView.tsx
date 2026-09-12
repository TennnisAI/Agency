import { useEffect, useMemo, useRef, useState } from "react";
import { FileRoot, Project, createFile, openTermPath, writeFile } from "../api";
import { SearchHit, stripExt } from "../lib/docsIndex";
import { buildLinkIndex, mentionsOf, noteId, resolveTarget } from "../lib/links";
import { requestNavigate } from "../lib/navigate";
import { attachmentLink } from "../lib/noteLink";
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";
import { useDocs } from "../hooks/useDocs";
import { useCrossRefs } from "../hooks/useCrossRefs";
import { useReportOpenFile } from "../hooks/useOpenFile";
import DocsTree from "./DocsTree";
import FileTabs from "./FileTabs";
import DocsEditor, { DocsEditorHandle } from "./DocsEditor";
import DocsSidePanel from "./DocsSidePanel";
import AgentSidePanel from "./AgentSidePanel";
import CheckoutBar from "./CheckoutBar";
import DocsQuickSwitcher from "./DocsQuickSwitcher";
import ConfirmDialog from "./ConfirmDialog";
import { toastError, toastInfo } from "../lib/toast";
import {
  TabState, closeTab, deserializeTabs, emptyTabs, openTab, removeTab, renameTab,
  retargetPath, serializeTabs,
} from "../lib/fileTabs";
import { baseName, joinPath } from "../lib/filePath";
import { adjacentDailyPath, isDailyNotePath } from "../lib/dailyNote";
import { recordActivation } from "../lib/recency";
import { terminalHasFocus } from "../lib/terminalFocus";

// Tab memory is per working tree, like the Files tab's. The project checkout
// keeps the bare project id it has always had, since the handoff below is
// addressed to it; each agent worktree gets a key of its own.
const viewKeyOf = (projectId: string, root: FileRoot) =>
  root.kind === "project" ? projectId : `${projectId}:run:${root.id}`;
const tabsKey = (viewKey: string) => `docs:tabs:${viewKey}`;
// The open note, kept as its own key because it doubles as a handoff: the
// palette, the menu, and cross-domain navigation stamp it before switching to
// the Docs tab so a freshly mounting view restores straight to that note. They
// all select the project first, which drops the run selection, so they stamp
// the checkout's key.
const lastNoteKey = (viewKey: string) => `docs:last:${viewKey}`;

// Which side-panel tab is showing. Global (not per project): it's a working
// mode — "I'm writing" vs "I'm pairing with an agent" — not a per-note fact.
const SIDE_TAB_KEY = "docs:side-tab";
type SideTab = "note" | "agents";

function loadSideTab(): SideTab {
  try {
    return localStorage.getItem(SIDE_TAB_KEY) === "agents" ? "agents" : "note";
  } catch {
    return "note";
  }
}

/**
 * The Docs tab: an Obsidian-lite over the project's `docs` folder, in the same
 * working tree the Files tab shows: the selected agent's worktree, else the
 * project checkout. It used to stay on the checkout whichever agent was
 * selected (AGE-231), so Files showed a worktree's notes and Docs, one tab
 * over, showed the checkout's, and an agent's edits to its notes were invisible
 * in Docs until they merged.
 */
export default function DocsView({ project, root, onOpenCheckout, onOpenFile }: {
  project: Project;
  root: FileRoot;
  // Open Source Control on the tree the checkout bar names.
  onOpenCheckout: () => void;
  // Open a file in the Files tab, rooted where these notes are.
  onOpenFile: (repoRel: string) => void;
}) {
  const treePane = usePaneWidth("docs-tree", 240, 180, 480);
  const notePane = usePaneWidth("docs-side", 240, 180, 420);
  // The agents tab hosts a live terminal, so it remembers its own (wider)
  // width — switching tabs shouldn't squeeze an agent into an outline column.
  const agentsPane = usePaneWidth("docs-side-agents", 420, 280, 900);
  const [sideTab, setSideTabState] = useState<SideTab>(loadSideTab);
  const sidePane = sideTab === "agents" ? agentsPane : notePane;
  const sideMin = sideTab === "agents" ? 280 : 180;
  const sideMax = sideTab === "agents" ? 900 : 420;
  const setSideTab = (t: SideTab) => {
    setSideTabState(t);
    try { localStorage.setItem(SIDE_TAB_KEY, t); } catch { /* storage unavailable */ }
  };
  const viewKey = viewKeyOf(project.id, root);
  const { docsDir, index, refresh, createDocsDir } = useDocs(project.id, true, root);
  const { cross } = useCrossRefs(true);
  // Tab state travels WITH the project and tree it belongs to, so a switch can
  // never persist one tree's tabs under another's key (same guard as
  // FilesView). The key is only stamped once the restore below has run.
  const [projectTabs, setProjectTabs] = useState<{ key: string | null; tabs: TabState }>(
    () => ({ key: null, tabs: emptyTabs() }),
  );
  // Tabs that have mounted an editor this session. Restored tabs are cold — no
  // file read, no CodeMirror instance — until first activation.
  const [warm, setWarm] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [sideOpen, setSideOpen] = useState(true);
  const [switcher, setSwitcher] = useState(false);
  const editorRefs = useRef(new Map<string, DocsEditorHandle>());
  const restoredRef = useRef(false);
  // Notes asked for before the restore ran (the index has to load first). They
  // are folded in there rather than dropped.
  const pendingRef = useRef<string[]>([]);

  // Never another tree's tabs: the frame between a switch and its restore
  // renders empty instead of stale state.
  const tabs = projectTabs.key === viewKey ? projectTabs.tabs : emptyTabs();
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const selected = tabs.active;
  // What an agent hears when the user says "this note" (AGE-200). Repo-relative
  // like every other path the backend takes, so the docs folder is on the front.
  useReportOpenFile(root, docsDir != null && selected ? joinPath(docsDir, selected) : null);

  // Every mutation is scoped to the current tree; one sneaking in before the
  // restore is dropped rather than corrupting the outgoing tree's state.
  const updateTabs = (fn: (s: TabState) => TabState) =>
    setProjectTabs((r) => (r.key === viewKey ? { ...r, tabs: fn(r.tabs) } : r));

  const dropWarm = (gone: (path: string) => boolean) =>
    setWarm((w) => new Set([...w].filter((p) => !gone(p))));

  // Cmd+P opens the quick switcher — active while the Docs tab is mounted,
  // including from inside the editor. (Verified free of menu/shortcut clashes.)
  // Not from inside the agents panel's terminal, though: Ctrl+P is the shell's
  // previous-command there.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (terminalHasFocus()) return;
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "p" && !e.shiftKey && !e.altKey) {
        e.preventDefault();
        setSwitcher((s) => !s);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  /** Open a note in a tab (or activate the one it already has). */
  const openNote = (path: string) => {
    if (restoredRef.current) updateTabs((s) => openTab(s, path));
    else pendingRef.current.push(path);
    setWarm((w) => (w.has(path) ? w : new Set(w).add(path)));
    // Feed the palette's recents. Only user-driven opens land here — the
    // restore below folds tabs in silently.
    recordActivation(`note:${project.id}:${path}`, index?.docs.get(path)?.title ?? path, path);
  };
  const openRef = useRef(openNote);
  openRef.current = openNote;

  const closeNote = (path: string) => {
    // The editor flushes any pending autosave as it unmounts, so closing a tab
    // never loses edits and needs no confirmation.
    updateTabs((s) => closeTab(s, path));
    dropWarm((p) => p === path);
  };
  const closeRef = useRef(closeNote);
  closeRef.current = closeNote;

  // Reset on project or tree switch; tabs are restored once the index has
  // loaded (so stale paths can be validated and dropped).
  useEffect(() => {
    setProjectTabs({ key: null, tabs: emptyTabs() });
    setWarm(new Set());
    setQuery("");
    editorRefs.current.clear();
    pendingRef.current = [];
    restoredRef.current = false;
  }, [viewKey]);

  useEffect(() => {
    if (restoredRef.current || !index) return;
    restoredRef.current = true;
    let raw: string | null = null;
    let stored: string | null = null;
    try {
      raw = localStorage.getItem(tabsKey(viewKey));
      stored = localStorage.getItem(lastNoteKey(viewKey));
    } catch { /* storage unavailable */ }
    const saved = deserializeTabs(raw);
    let s: TabState = {
      ...emptyTabs(),
      open: saved.open.filter((p) => index.docs.has(p)),
    };
    s.active = saved.active !== null && s.open.includes(saved.active) ? saved.active : null;
    // The last-note stamp wins: it's both this view's own last selection and the
    // handoff other views use to point the Docs tab at a specific note.
    if (stored && index.docs.has(stored)) s = openTab(s, stored);
    // Pending opens are explicit user intent (a palette hit, a just-created
    // note), so they aren't index-checked — the note may be newer than it.
    for (const p of pendingRef.current) s = openTab(s, p);
    pendingRef.current = [];
    if (!s.active) {
      // First visit (or every stored note is gone): open the first by title.
      const first = [...index.docs.values()].sort((a, b) => a.title.localeCompare(b.title))[0];
      if (first) s = openTab(s, first.path);
    }
    setProjectTabs({ key: viewKey, tabs: s });
    setWarm((w) => (s.active ? new Set(w).add(s.active) : w));
  }, [index, viewKey]);

  // Persist per tree, only once the state actually belongs to it (before the
  // restore, `key` is null and a write here would clobber the stored tabs).
  useEffect(() => {
    if (projectTabs.key !== viewKey) return;
    try {
      localStorage.setItem(tabsKey(viewKey), serializeTabs(projectTabs.tabs));
      const active = projectTabs.tabs.active;
      if (active) localStorage.setItem(lastNoteKey(viewKey), active);
      else localStorage.removeItem(lastNoteKey(viewKey));
    } catch { /* storage unavailable */ }
  }, [projectTabs, viewKey]);

  // ⌘W closes the active note. Same binding as the Files tab (menu.rs claims no
  // ⌘W accelerator), minus the agents panel's terminal where Ctrl+W is
  // delete-word and has to reach the shell.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (terminalHasFocus()) return;
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "w") {
        const active = tabsRef.current.active;
        if (active) {
          e.preventDefault();
          closeRef.current(active);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // "Today's note" (⌘⇧D / menu / palette) lands here when this view is already
  // mounted; a fresh mount is covered by the docs:last localStorage restore.
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  const onCheckout = root.kind === "project";
  useEffect(() => {
    const onOpen = (e: Event) => {
      const d = (e as CustomEvent<{ projectId: string; path: string }>).detail;
      if (!d || d.projectId !== project.id) return;
      // Every sender selects the project before dispatching, which drops the
      // run selection, so the note is meant for the checkout. A worktree view
      // still mounted for that one frame would open it among the worktree's
      // tabs; it leaves it to the checkout's restore, which reads the stamp.
      if (!onCheckout) return;
      void refreshRef.current().then(() => openRef.current(d.path));
    };
    window.addEventListener("agency:open-note", onOpen);
    return () => window.removeEventListener("agency:open-note", onOpen);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.id, onCheckout]);

  // A wikilink pointed at a note that doesn't exist; confirm before creating.
  const [pendingCreate, setPendingCreate] = useState<string | null>(null);

  // Attachments that just landed in the vault while a note was open, and the
  // note they landed for. Offered rather than inserted: dropping a file into a
  // folder in the tree is a filing gesture, and it is only sometimes also
  // "and put it in what I'm writing".
  const [pendingLink, setPendingLink] = useState<{ note: string; paths: string[] } | null>(null);

  /** Open an attachment row: `repoRel` is relative to the tree's root. */
  const openAttachment = (repoRel: string) => {
    // The workspace hides the Files tab (it would duplicate Docs), so there is
    // no in-app viewer to route to there. Hand it to the OS instead, the same
    // way a clicked path in a terminal goes.
    if (project.kind === "workspace") {
      openTermPath(root, repoRel).catch((e) => toastError(e, "Couldn't open"));
      return;
    }
    // Not requestNavigate: a link lands on the project checkout, and a
    // screenshot dropped into a worktree's notes isn't there.
    onOpenFile(repoRel);
  };

  const insertLinks = (note: string, paths: string[]) => {
    const editor = editorRefs.current.get(note);
    if (!editor) return; // the note was closed while the dialog was up
    editor.insertAtCursor(paths.map((p) => attachmentLink(note, p)).join("\n"));
  };

  const navigate = (target: string, heading: string | null) => {
    if (!index) return;
    const res = resolveTarget(index, cross, target);
    switch (res.kind) {
      case "note":
        openNote(res.path);
        // Best-effort: give a freshly opened tab a beat to mount and load.
        if (heading) setTimeout(() => editorRefs.current.get(res.path)?.scrollToHeading(heading), 150);
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
      openNote(path);
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
    openNote(hit.path);
    // A line hit scrolls once the editor has the note open; heading text is a
    // best-effort anchor, so only title hits (-1) skip it.
    if (hit.line >= 0) {
      const heading = /^#{1,6}\s+(.+)$/.exec(hit.snippet)?.[1];
      if (heading) setTimeout(() => editorRefs.current.get(hit.path)?.scrollToHeading(heading), 150);
    }
  };

  // Tab captions match the tree: the note's H1 title, else its filename.
  const noteLabel = (path: string) =>
    index?.docs.get(path)?.title ?? stripExt(baseName(path));

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
      {/* Which working tree these notes come from. It follows the selected
          agent, like Files, so it changes under you as you move around. */}
      <CheckoutBar root={root} projectId={project.id} projectName={project.name} onOpen={onOpenCheckout} />
      <div className="files-body">
        <div className="files-tree docs-tree" style={{ width: treePane.width }}>
          <DocsTree
            root={root}
            docsDir={docsDir}
            rootLabel={docsDir === "" ? project.name : undefined}
            index={index}
            selected={selected}
            query={query}
            onQuery={setQuery}
            onSelect={(p) => { setQuery(""); if (p) openNote(p); }}
            onOpenFile={(p) => openAttachment(joinPath(docsDir, p))}
            onAttached={(paths) => { if (selected) setPendingLink({ note: selected, paths }); }}
            onOpenHit={openHit}
            onRenamed={(from, to) => {
              updateTabs((s) => renameTab(s, from, to));
              setWarm((w) => new Set([...w].map((p) => retargetPath(p, from, to))));
            }}
            onDeleted={(path) => {
              updateTabs((s) => removeTab(s, path));
              dropWarm((p) => p === path || p.startsWith(path + "/"));
            }}
            refresh={refresh}
          />
        </div>
        <Resizer size={treePane.width} min={180} max={480} onChange={treePane.setWidth} />
        <div className="files-editor">
          {tabs.open.length > 0 && (
            <FileTabs
              open={tabs.open}
              active={tabs.active}
              labelFor={noteLabel}
              onActivate={openNote}
              onClose={closeNote}
            />
          )}
          {tabs.open.filter((p) => warm.has(p)).map((p) => (
            <div
              key={`${viewKey}:${p}`}
              className="files-editor-pane"
              style={{ display: p === tabs.active ? "flex" : "none" }}
            >
              <DocsEditor
                ref={(h) => { if (h) editorRefs.current.set(p, h); else editorRefs.current.delete(p); }}
                root={root}
                docsDir={docsDir}
                path={p}
                diskText={index?.docs.get(p)?.text}
                index={index}
                cross={cross}
                onSaved={() => void refresh()}
                onNavigate={navigate}
                onTagClick={(tag) => setQuery(tag.startsWith("#") ? tag : `#${tag}`)}
                onFilter={(k, v) => setQuery(v ? (/\s/.test(v) ? `${k}:"${v}"` : `${k}:${v}`) : `${k}:`)}
                sideOpen={sideOpen}
                onToggleSide={() => setSideOpen((o) => !o)}
                daily={index && isDailyNotePath(p) ? {
                  prev: adjacentDailyPath(index.docs.keys(), p, "prev"),
                  next: adjacentDailyPath(index.docs.keys(), p, "next"),
                  onOpen: openNote,
                } : null}
              />
            </div>
          ))}
          {!tabs.active && <div className="diff-empty">Select or create a note.</div>}
        </div>
        {sideOpen && (selected || sideTab === "agents") && (
          <>
            <Resizer size={sidePane.width} min={sideMin} max={sideMax} onChange={sidePane.setWidth} side="right" />
            <div className="side-pane docs-side" style={{ width: sidePane.width }}>
              <div className="docs-side-tabs">
                <button className={sideTab === "note" ? "on" : ""} onClick={() => setSideTab("note")}>▥ Note</button>
                <button className={sideTab === "agents" ? "on" : ""} onClick={() => setSideTab("agents")}>▦ Agents</button>
              </div>
              {sideTab === "agents" ? (
                <AgentSidePanel project={project} />
              ) : (
                <DocsSidePanel
                  index={index}
                  selected={selected}
                  mentions={mentions}
                  onJumpToHeading={(text) => { if (selected) editorRefs.current.get(selected)?.scrollToHeading(text); }}
                  onOpen={openNote}
                  onOpenMention={(m) => {
                    if (m.fromKind === "issue") {
                      requestNavigate({ kind: "issue", projectId: m.fromProjectId, issueId: m.fromId });
                    }
                  }}
                  onFilter={(k, v) => setQuery(v ? (/\s/.test(v) ? `${k}:"${v}"` : `${k}:${v}`) : `${k}:`)}
                  onAddProperty={() => { if (selected) editorRefs.current.get(selected)?.addProperty(); }}
                />
              )}
            </div>
          </>
        )}
      </div>

      {switcher && (
        <DocsQuickSwitcher
          index={index}
          onOpen={openNote}
          onCreate={(name) => void createNote(name)}
          onClose={() => setSwitcher(false)}
        />
      )}

      {pendingLink && (
        <ConfirmDialog
          title="Link it from this note?"
          body={
            pendingLink.paths.length === 1
              ? `Insert a link to ${baseName(pendingLink.paths[0])} at the cursor in "${noteLabel(pendingLink.note)}".`
              : `Insert links to ${pendingLink.paths.length} files at the cursor in "${noteLabel(pendingLink.note)}".`
          }
          confirmLabel="Insert"
          onConfirm={() => { const l = pendingLink; setPendingLink(null); insertLinks(l.note, l.paths); }}
          onCancel={() => setPendingLink(null)}
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
