import { useEffect, useMemo, useRef, useState } from "react";
import { FileRoot, Issue, IssuePatch, IssueStatus, Project, createIssue, deleteIssue, getWorkspace, updateIssue } from "../api";
import { planReorder } from "../lib/issueRank";
import { Corpus, IssueRef, LinkEdge, buildLinkIndex, mentionsOf, resolveTarget } from "../lib/links";
import { IssueLink, hasLink, issueLinks, linkCandidates, withLink, withoutLink } from "../lib/issueLinks";
import { requestNavigate } from "../lib/navigate";
import { useRuns } from "../store/runs";
import { useIssues } from "../hooks/useIssues";
import { useCrossRefs } from "../hooks/useCrossRefs";
import { useDocs } from "../hooks/useDocs";
import {
  DEFAULT_COLLAPSED,
  ISSUE_STATUSES,
  IssueSort,
  PENDING_ISSUE_KEY,
  PENDING_QUICKADD_KEY,
  PRIORITY_LABELS,
  STATUS_LABELS,
  StatusFilter,
  filtersActive,
  issueLabel,
  issueSorter,
  issuesCollapsedKey,
  issuesExpandedKey,
  issuesSelectedKey,
  loadCollapsed,
  loadSelected,
  matchesFilters,
  saveCollapsed,
  saveSelected,
  searchTerms,
} from "../lib/issues";
import { pickDefaultAgent } from "../lib/defaultAgent";
import { loadFold, saveFold, usePaneWidth } from "../hooks/usePaneWidth";
import IssueRow from "./IssueRow";
import IssueDetail from "./IssueDetail";
import ConfirmDialog from "./ConfirmDialog";
import PillSelect from "./PillSelect";
import Resizer from "./Resizer";
import { toastError, toastInfo } from "../lib/toast";
import { FindRank, registerFindTarget } from "../lib/findBus";

// How narrow the list may get once the detail pane takes over the view.
const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 460;

// The project's issue board: a status-grouped list (Linear's default view),
// quick capture on top, detail pane on the right. Dispatching an issue to an
// agent goes through `onStartIssue`, which runs the same installed/readiness
// checks as the "+ Agent" flow.
export default function IssuesView({
  project,
  onStartIssue,
}: {
  project: Project;
  onStartIssue: (issue: Issue, agentId: string, opts?: { base: string; mergeTarget: string }) => Promise<void>;
}) {
  const { runs, tab, setTab, setView, setFocusedRun } = useRuns();
  const { issues, loaded, refresh } = useIssues(project.id, tab === "issues");
  const [quick, setQuick] = useState("");
  const [selectedId, setSelectedIdState] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<Issue | null>(null);
  // Any status group folds; done/cancelled are the ones that start folded.
  const [collapsed, setCollapsed] = useState<Set<IssueStatus>>(() => new Set(DEFAULT_COLLAPSED));
  // Quick-add rests as a + button and expands into an inline input on demand.
  const [quickOpen, setQuickOpen] = useState(false);
  const quickRef = useRef<HTMLInputElement>(null);
  // Search and filters. Same controls as the cross-project home board, and
  // they live only for this visit — a hidden filter left over from last time
  // reads as a broken board.
  const [q, setQ] = useState("");
  const [fStatus, setFStatus] = useState<StatusFilter>("all");
  const [fPriority, setFPriority] = useState(-1);
  const [sort, setSort] = useState<IssueSort>("board");
  const searchRef = useRef<HTMLInputElement>(null);

  // ⌘F / Edit ▸ Find over a board means "narrow it", so it lands in the search
  // field rather than opening a text-scanning bar over a list of rows. The
  // field is the registered host, which self-gates: on an empty tracker there
  // is no filter bar, the ref is null, and ⌘F falls through to the description.
  useEffect(() => registerFindTarget({
    host: () => searchRef.current,
    open: () => {
      searchRef.current?.focus();
      searchRef.current?.select();
    },
    canReplace: false,
    rank: FindRank.list,
  }), []);

  function clearFilters() {
    setQ("");
    setFStatus("all");
    setFPriority(-1);
    setSort("board");
  }

  // Expanded mode: the detail pane takes over the view and the list compresses
  // to a sidebar. Remembered per project, so coming back to a tracker restores
  // the reading layout it was left in.
  const expandKey = issuesExpandedKey(project.id);
  const [expanded, setExpandedState] = useState(false);
  useEffect(() => {
    setExpandedState(typeof localStorage === "undefined" ? false : loadFold(localStorage, expandKey, false));
  }, [expandKey]);
  const setExpanded = (next: boolean) => {
    setExpandedState(next);
    try {
      if (typeof localStorage !== "undefined") saveFold(localStorage, expandKey, next);
    } catch {
      /* storage unavailable */
    }
  };
  // The compressed list's width is a plain machine-wide pane pref, like every
  // other resizable pane — only the expanded/contracted choice is per project.
  const sidebar = usePaneWidth("issues-sidebar", 288, SIDEBAR_MIN, SIDEBAR_MAX);

  // Folded groups are remembered per project too: which parts of the board
  // are worth seeing is a property of the tracker, not of this visit.
  const collapsedKey = issuesCollapsedKey(project.id);
  useEffect(() => {
    setCollapsed(
      typeof localStorage === "undefined" ? new Set(DEFAULT_COLLAPSED) : loadCollapsed(localStorage, collapsedKey),
    );
  }, [collapsedKey]);
  function toggleGroup(status: IssueStatus) {
    const next = new Set(collapsed);
    if (next.has(status)) next.delete(status); else next.add(status);
    setCollapsed(next);
    try {
      if (typeof localStorage !== "undefined") saveCollapsed(localStorage, collapsedKey, next);
    } catch {
      /* storage unavailable */
    }
  }

  // Which issue is open is remembered per project as well, so a trip to the
  // agents, a diff or a note and back lands on the ticket being read. Only
  // deliberate opens and closes are recorded — a project switch clears the
  // pane through the raw setter below, and must not erase the selection
  // stored for the project being switched to.
  const selectedKey = issuesSelectedKey(project.id);
  const setSelectedId = (id: string | null) => {
    setSelectedIdState(id);
    try {
      if (typeof localStorage !== "undefined") saveSelected(localStorage, selectedKey, id);
    } catch {
      /* storage unavailable */
    }
  };
  // Which project's stored selection has been restored already: without it,
  // a pane the user just closed would spring back open on the next poll.
  const restoredFor = useRef<string | null>(null);
  useEffect(() => {
    // On the commit where the project changes, the list is still the previous
    // project's, and an empty list says nothing about whose it is: restore
    // only against issues that are demonstrably this project's, so a stale
    // list can't burn the one restore this board gets.
    if (issues.length === 0 || issues[0].projectId !== project.id) return;
    if (restoredFor.current === project.id) return;
    restoredFor.current = project.id;
    // A handoff from elsewhere (the home overview, the palette) is fresher
    // intent than the stored selection; the effect below applies it.
    if (sessionStorage.getItem(PENDING_ISSUE_KEY)) return;
    let stored: string | null = null;
    try {
      if (typeof localStorage !== "undefined") stored = loadSelected(localStorage, selectedKey, issues);
    } catch {
      /* storage unavailable */
    }
    if (stored) setSelectedIdState(stored);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issues, project.id, selectedKey]);

  useEffect(() => { setSelectedIdState(null); setQuick(""); setQuickOpen(false); clearFilters(); }, [project.id]);
  useEffect(() => { if (quickOpen) quickRef.current?.focus(); }, [quickOpen]);

  // The palette's "New Issue" lands here: open quick-add on tab activation.
  useEffect(() => {
    if (tab !== "issues") return;
    if (sessionStorage.getItem(PENDING_QUICKADD_KEY)) {
      sessionStorage.removeItem(PENDING_QUICKADD_KEY);
      setQuickOpen(true);
      quickRef.current?.focus();
    }
  }, [tab]);

  // An issue clicked on the home overview arrives via sessionStorage (project
  // selection resets tab state, so props can't carry it); select it once the
  // list contains it.
  useEffect(() => {
    const pending = sessionStorage.getItem(PENDING_ISSUE_KEY);
    if (pending && issues.some((i) => i.id === pending)) {
      sessionStorage.removeItem(PENDING_ISSUE_KEY);
      setSelectedId(pending);
    }
  }, [issues]);

  const terms = useMemo(() => searchTerms(q), [q]);
  const filters = useMemo(
    () => ({ terms, status: fStatus, priority: fPriority }),
    [terms, fStatus, fPriority],
  );
  const filtered = filtersActive(filters);
  // Anything the user has touched up top, including a sort — what "Reset" and
  // the match count answer to.
  const narrowed = filtered || sort !== "board";
  // Manual order only means something over the whole, board-sorted group: a
  // rank computed from a filtered subset would reshuffle the hidden rows.
  const reorderable = !narrowed;
  const groups = useMemo(() => {
    const sorter = issueSorter(sort);
    return ISSUE_STATUSES.map((status) => ({
      status,
      issues: issues
        .filter((i) => i.status === status && matchesFilters(i, issueLabel(project, i), filters))
        .sort(sorter),
    }));
  }, [issues, filters, sort, project]);
  // Mouse handlers commit against the freshest grouping, not their closure's.
  const groupsRef = useRef(groups);
  groupsRef.current = groups;
  // A hit inside a folded group would otherwise be a match the board never
  // shows, so filtering opens every group.
  const groupOpen = (status: IssueStatus) => filtered || !collapsed.has(status);
  const visible = useMemo(
    () => groups.flatMap((g) => (groupOpen(g.status) ? g.issues : [])),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [groups, collapsed, filtered],
  );
  const matched = groups.reduce((n, g) => n + g.issues.length, 0);
  const selected = issues.find((i) => i.id === selectedId) ?? null;
  const runsFor = (issue: Issue) => runs.filter((r) => r.issueId === issue.id);
  // Attachments are written beside the issue files, in the project's main
  // checkout — never a run's worktree.
  const fileRoot = useMemo<FileRoot>(() => ({ kind: "project", id: project.id }), [project.id]);

  // Mentions for the detail pane (one-stop Phase 7): notes and issues linking
  // to the selected issue. Corpora scanned: this project's docs plus the
  // workspace vault (where journal notes live) — other projects' docs are out
  // of scope until someone needs them.
  const [workspace, setWorkspace] = useState<Project | null>(null);
  useEffect(() => {
    let cancelled = false;
    getWorkspace().then((ws) => { if (!cancelled) setWorkspace(ws); }).catch(() => {});
    return () => { cancelled = true; };
  }, []);
  const wsId = workspace && workspace.id !== project.id ? workspace.id : null;
  const active = tab === "issues";
  const ownDocs = useDocs(project.id, active);
  const wsDocs = useDocs(wsId, active);
  const { cross, refresh: refreshCross } = useCrossRefs(active);
  const linkTable = useMemo(() => {
    if (!cross) return null;
    const corpora: Corpus[] = [];
    if (ownDocs.index) corpora.push({ project, index: ownDocs.index });
    if (workspace && wsId && wsDocs.index) corpora.push({ project: workspace, index: wsDocs.index });
    return buildLinkIndex(corpora, cross);
  }, [cross, ownDocs.index, wsDocs.index, project, workspace, wsId]);
  const mentions = useMemo(
    () => (linkTable && selected ? mentionsOf(linkTable, "issue", selected.id) : []),
    [linkTable, selected],
  );

  // The selected issue's `links:` relations, and what is left to link it to.
  // Both read from the cross-project rows, so an issue in another tracker is
  // as linkable as one in this project.
  const selectedLabel = selected ? issueLabel(project, selected) : "";
  const links = useMemo(
    () => (selected ? issueLinks(selected, selectedLabel, cross) : []),
    [selected, selectedLabel, cross],
  );
  const candidates = useMemo(
    () => linkCandidates(cross, project.id, selectedLabel, links),
    [cross, project.id, selectedLabel, links],
  );

  // Linking writes both sides: the relation is undirected, and an issue file
  // read on its own (by an agent, or in any markdown editor) should say what
  // it is linked to without having to grep the rest of the tracker. The other
  // side's row comes from the cross-project poll, so its refresh is what makes
  // the new link show up on the issue it was made from.
  async function addLink(ref: IssueRef) {
    if (!selected) return;
    const other = issueLabel(ref.project, ref.issue);
    try {
      await updateIssue(selected.id, { links: withLink(selected.links, other) });
      await updateIssue(ref.issue.id, { links: withLink(ref.issue.links, selectedLabel) });
      await Promise.all([refresh(), refreshCross()]);
    } catch (e) {
      toastError(e, `Couldn't link ${other}`);
    }
  }

  // Unlinking clears whichever sides hold it — including a link only the other
  // issue's file says (a hand edit, or a write that failed halfway).
  async function removeLink(link: IssueLink) {
    if (!selected) return;
    try {
      if (hasLink(selected.links, link.label)) {
        await updateIssue(selected.id, { links: withoutLink(selected.links, link.label) });
      }
      if (link.ref && hasLink(link.ref.issue.links, selectedLabel)) {
        await updateIssue(link.ref.issue.id, { links: withoutLink(link.ref.issue.links, selectedLabel) });
      }
      await Promise.all([refresh(), refreshCross()]);
    } catch (e) {
      toastError(e, `Couldn't unlink ${link.label}`);
    }
  }

  // A wikilink ⌘-clicked in a description. Notes resolve against this project's
  // docs; issues and runs against every project. Nothing is created from here —
  // an issue is no place to be told "⌘-click to create a note".
  function followLink(target: string) {
    const res = resolveTarget(ownDocs.index, cross, target);
    switch (res.kind) {
      case "note":
        // A "#heading" suffix opens the note; scrolling to the heading is a
        // docs-editor trick that cross-tab navigation doesn't carry.
        requestNavigate({ kind: "note", projectId: project.id, path: res.path });
        return;
      case "issue":
        requestNavigate({ kind: "issue", projectId: res.ref.project.id, issueId: res.ref.issue.id });
        return;
      case "run":
        requestNavigate({ kind: "run", projectId: res.ref.project.id, runId: res.ref.run.id });
        return;
      case "unresolvedIssue":
        toastInfo(`No issue ${res.label} in any project.`);
        return;
      case "unresolvedRun":
        toastInfo("That run doesn't exist anymore.");
        return;
      case "unresolvedNote":
        toastInfo(`No note "${target}" in this project.`);
    }
  }

  function openMention(m: LinkEdge) {
    if (m.fromKind === "note") {
      requestNavigate({ kind: "note", projectId: m.fromProjectId, path: m.fromId });
    } else {
      requestNavigate({ kind: "issue", projectId: m.fromProjectId, issueId: m.fromId });
    }
  }

  async function add(status: "todo" | "backlog") {
    const title = quick.trim();
    if (!title) return;
    setQuick("");
    try {
      const issue = await createIssue(project.id, title, "", status);
      await refresh();
      setSelectedId(issue.id);
    } catch (e) {
      toastError(e, "Couldn't create issue");
    }
  }

  async function patch(issue: Issue, p: IssuePatch) {
    try {
      await updateIssue(issue.id, p);
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't update issue");
    }
  }

  async function doDelete(issue: Issue) {
    setConfirmDelete(null);
    if (selectedId === issue.id) setSelectedId(null);
    try {
      await deleteIssue(issue.id);
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't delete issue");
    }
  }

  // Manual reorder within a status group. Pointer-based (mousedown →
  // 5px threshold → track → commit on mouseup), NOT HTML5 drag-and-drop:
  // Tauri's native drag-drop layer intercepts drops at the NSView level on
  // macOS, so an in-page HTML5 drag lifts but its drop event never fires.
  // The live position is a plain ref (mouse events outrun React renders);
  // `drag` state mirrors it for the seam indicator. The drop maps to a rank
  // plan (materialize / midpoint / renormalize — see lib/issueRank).
  const [drag, setDrag] = useState<{ status: IssueStatus; from: number; to: number } | null>(null);
  const dragLive = useRef<{ status: IssueStatus; from: number; to: number } | null>(null);
  // A completed drag must not read as a click on the row it ends over.
  const suppressClick = useRef(false);

  async function dropReorder(group: Issue[], from: number, to: number) {
    const plan = planReorder(group.map((i) => ({ id: i.id, rank: i.rank })), from, to);
    if (plan.length === 0) return;
    try {
      await Promise.all(plan.map((u) => updateIssue(u.id, { rank: u.rank })));
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't reorder");
    }
  }

  function onRowMouseDown(status: IssueStatus, from: number, e: React.MouseEvent) {
    if (e.button !== 0) return;
    // Grabs must start on the row itself — not its buttons and menus.
    if ((e.target as HTMLElement).closest("button, input, textarea")) return;
    // Retire the focused editor by hand: preventDefault below suppresses the
    // blur that would otherwise commit an edit in the detail pane, and the
    // click that follows swaps the pane to another issue.
    // (The description is a CodeMirror contenteditable, not a textarea.)
    const focused = document.activeElement as HTMLElement | null;
    if (focused && (focused.tagName === "TEXTAREA" || focused.tagName === "INPUT" || focused.isContentEditable)) {
      focused.blur();
    }
    // Stop WebKit starting a text selection on the key/title — the selection
    // begins at mousedown, long before the drag threshold; the click that
    // selects the row is unaffected.
    e.preventDefault();
    const start = { x: e.clientX, y: e.clientY };
    // Set once the 5px threshold is crossed; drives click suppression on
    // release even after a cancel (the pointer is no longer "just clicking").
    let started = false;
    // Escape sets this so the drag can't silently restart on the next
    // mousemove; the still-held button then releases as a no-op.
    let cancelled = false;

    const onMove = (ev: MouseEvent) => {
      if (cancelled) return;
      if (!dragLive.current) {
        if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < 5) return;
        started = true;
        dragLive.current = { status, from, to: from };
        window.getSelection()?.removeAllRanges();
      }
      ev.preventDefault();
      const row = (document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)
        ?.closest<HTMLElement>("[data-issue-idx]");
      if (row && row.dataset.issueStatus === status) {
        dragLive.current = { ...dragLive.current, to: Number(row.dataset.issueIdx) };
      }
      setDrag({ ...dragLive.current });
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      cancelled = true;
      dragLive.current = null;
      setDrag(null);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("keydown", onKey, true);
      const d = dragLive.current;
      dragLive.current = null;
      setDrag(null);
      if (started) {
        // Neither a completed nor a cancelled drag may read as a click on
        // the row the pointer ends over.
        suppressClick.current = true;
        window.setTimeout(() => { suppressClick.current = false; }, 0);
      }
      if (!d || cancelled) return;
      const group = groupsRef.current.find((g) => g.status === d.status)?.issues ?? [];
      dropReorder(group, d.from, d.to);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    // Capture phase so an Escape mid-drag can't reach anything else.
    window.addEventListener("keydown", onKey, true);
  }

  async function startDefault(issue: Issue) {
    const agent = await pickDefaultAgent(project.id, project.default_agent);
    await onStartIssue(issue, agent);
  }

  function openRun(runId: string) {
    setFocusedRun(runId);
    setView("focus");
    setTab("agents");
  }

  // Linear-flavored keys: `c` captures, `/` searches, arrows move, Enter
  // opens, 0-4 set priority. Skipped while typing in any field.
  useEffect(() => {
    if (tab !== "issues") return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "c") {
        e.preventDefault();
        setQuickOpen(true);
        quickRef.current?.focus();
      } else if (e.key === "/") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        if (visible.length === 0) return;
        e.preventDefault();
        const idx = visible.findIndex((i) => i.id === selectedId);
        const next = e.key === "ArrowDown"
          ? visible[Math.min(idx + 1, visible.length - 1)]
          : visible[Math.max(idx - 1, 0)];
        setSelectedId(next.id);
      } else if (e.key === "Enter" && selectedId == null && visible.length > 0) {
        e.preventDefault();
        setSelectedId(visible[0].id);
      } else if (/^[0-4]$/.test(e.key) && selected) {
        e.preventDefault();
        patch(selected, { priority: Number(e.key) });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, visible, selectedId, selected]);

  // The list only compresses while there is a detail pane to give the room to.
  const wide = expanded && selected != null;

  return (
    <div className={`issues-wrap${wide ? " expanded" : ""}`}>
      <div className="issues-main" style={wide ? { width: sidebar.width, flex: "0 0 auto" } : undefined}>
        <div className={`issues-quickadd${quickOpen ? " open" : ""}`}>
          <button
            className="quickadd-toggle"
            title={quickOpen ? "Close" : "Add an issue (c)"}
            // Keep the press from blurring the input first — the blur handler
            // would collapse the box and this click would re-open it.
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              if (quickOpen) {
                setQuick("");
                setQuickOpen(false);
                quickRef.current?.blur();
              } else {
                setQuickOpen(true);
              }
            }}
          >
            +
          </button>
          <input
            ref={quickRef}
            className="quickadd-input"
            placeholder="Add an issue…  (Enter → Todo, Shift+Enter → Backlog)"
            value={quick}
            tabIndex={quickOpen ? 0 : -1}
            onChange={(e) => setQuick(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") add(e.shiftKey ? "backlog" : "todo");
              if (e.key === "Escape") {
                setQuick("");
                setQuickOpen(false);
                (e.target as HTMLInputElement).blur();
              }
            }}
            onBlur={() => { if (!quick.trim()) setQuickOpen(false); }}
          />
        </div>
        {issues.length > 0 && (
          // Compressed to a sidebar there is no room for the pills, so the bar
          // keeps what a narrow list needs most — the search, and the way back
          // to the whole board if a filter is still on from before the expand.
          <div className={`issues-filterbar issues-toolbar${wide ? " compact" : ""}`}>
            <span className="filter-search">
              <span className="filter-search-glyph">⌕</span>
              <input
                ref={searchRef}
                placeholder="Search issues…"
                title="Search issues (/)"
                value={q}
                onChange={(e) => setQ(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key !== "Escape") return;
                  // First Escape empties a query, the next leaves the field.
                  if (q) setQ("");
                  else (e.target as HTMLInputElement).blur();
                }}
              />
              {q && <button className="filter-clear" title="Clear search" onClick={() => setQ("")}>✕</button>}
            </span>
            {narrowed && !wide && (
              <span className="filter-count">{matched} of {issues.length}</span>
            )}
            <div className="spacer" />
            {narrowed && (
              <button className="filter-reset" title="Clear search and filters" onClick={clearFilters}>
                Reset
              </button>
            )}
            {!wide && (
              <>
                <PillSelect<StatusFilter>
                  value={fStatus}
                  defaultValue="all"
                  title="Status"
                  onChange={setFStatus}
                  options={[
                    { value: "all", label: "All statuses" },
                    { value: "open", label: "Open" },
                    ...ISSUE_STATUSES.map((s) => ({ value: s, label: STATUS_LABELS[s] })),
                  ]}
                />
                <PillSelect
                  value={fPriority}
                  defaultValue={-1}
                  title="Priority"
                  onChange={setFPriority}
                  options={[
                    { value: -1, label: "Any priority" },
                    ...PRIORITY_LABELS.map((p, n) => ({ value: n, label: p })),
                  ]}
                />
                <PillSelect<IssueSort>
                  value={sort}
                  defaultValue="board"
                  title="Sort"
                  onChange={setSort}
                  options={[
                    { value: "board", label: "Board order" },
                    { value: "due", label: "Due date" },
                    { value: "updated", label: "Recently updated" },
                  ]}
                />
              </>
            )}
          </div>
        )}
        {loaded && issues.length === 0 ? (
          <div className="board empty issues-empty">
            <button
              className="quickadd-toggle big"
              title="Add an issue (c)"
              onClick={() => setQuickOpen(true)}
            >
              +
            </button>
            <div>Capture your first issues. Agents can pick up issues from here.</div>
          </div>
        ) : (
          <div className={`issues-list${drag ? " reordering" : ""}${wide ? " compact" : ""}`}>
            {issues.length > 0 && matched === 0 && (
              <div className="issues-nomatch">
                <div className="issues-nomatch-title">No issues match</div>
                <button className="ghost" onClick={clearFilters}>Clear search and filters</button>
              </div>
            )}
            {groups.map(({ status, issues: group }) => {
              if (group.length === 0) return null;
              const open = groupOpen(status);
              return (
                <section key={status} className="issue-group">
                  <button
                    className={`issue-group-head${filtered ? " static" : ""}`}
                    aria-expanded={open}
                    title={filtered ? undefined : `${open ? "Collapse" : "Expand"} ${STATUS_LABELS[status]}`}
                    onClick={() => {
                      // Filtering pins every group open, so folding is off.
                      if (filtered) return;
                      toggleGroup(status);
                    }}
                  >
                    {!filtered && <span className="issue-group-chev">{open ? "▾" : "▸"}</span>}
                    <span className="issue-group-name">{STATUS_LABELS[status]}</span>
                    <span className="issue-group-count">{group.length}</span>
                  </button>
                  {open && group.map((issue, idx) => (
                    <IssueRow
                      key={issue.id}
                      issue={issue}
                      label={issueLabel(project, issue)}
                      runs={runsFor(issue)}
                      selected={issue.id === selectedId}
                      onSelect={() => {
                        if (suppressClick.current) return;
                        setSelectedId(issue.id === selectedId ? null : issue.id);
                      }}
                      onStart={() => { startDefault(issue); }}
                      onSpawnAgent={(agentId, opts) => { onStartIssue(issue, agentId, opts); }}
                      onPatch={(p) => patch(issue, p)}
                      onDelete={() => setConfirmDelete(issue)}
                      terms={terms}
                      drag={!reorderable ? undefined : {
                        over:
                          drag && drag.status === status && drag.to === idx && drag.from !== idx
                            ? (drag.to > drag.from ? "below" : "above")
                            : null,
                        source: !!drag && drag.status === status && drag.from === idx,
                        idx,
                        status,
                        onMouseDown: (e) => onRowMouseDown(status, idx, e),
                      }}
                    />
                  ))}
                </section>
              );
            })}
          </div>
        )}
      </div>
      {selected && (
        <>
          {wide && (
            <Resizer
              size={sidebar.width}
              min={SIDEBAR_MIN}
              max={SIDEBAR_MAX}
              onChange={sidebar.setWidth}
              side="left"
            />
          )}
          <IssueDetail
            issue={selected}
            label={issueLabel(project, selected)}
            root={fileRoot}
            runs={runsFor(selected)}
            mentions={mentions}
            links={links}
            linkCandidates={candidates}
            index={ownDocs.index}
            cross={cross}
            expanded={wide}
            onToggleExpand={() => setExpanded(!expanded)}
            onPatch={(p) => patch(selected, p)}
            onStart={() => { startDefault(selected); }}
            onSpawnAgent={(agentId, opts) => { onStartIssue(selected, agentId, opts); }}
            onDelete={() => setConfirmDelete(selected)}
            onOpenRun={openRun}
            onOpenMention={openMention}
            onOpenIssue={(ref) =>
              requestNavigate({ kind: "issue", projectId: ref.project.id, issueId: ref.issue.id })}
            onAddLink={(ref) => { void addLink(ref); }}
            onRemoveLink={(link) => { void removeLink(link); }}
            onFollowLink={followLink}
            // A tag in a description narrows the board to the issues carrying
            // it — the same "show me these" the docs tab gives its search.
            onTagClick={(tag) => setQ(tag.startsWith("#") ? tag : `#${tag}`)}
            onClose={() => setSelectedId(null)}
          />
        </>
      )}
      {confirmDelete && (
        <ConfirmDialog
          title="Delete issue?"
          body={`Delete "${issueLabel(project, confirmDelete)} ${confirmDelete.title}". Its number is never reused. This cannot be undone.`}
          confirmLabel="Delete"
          danger
          onConfirm={() => { doDelete(confirmDelete); }}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
    </div>
  );
}
