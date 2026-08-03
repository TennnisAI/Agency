import { useEffect, useMemo, useRef, useState } from "react";
import { FileRoot, Issue, IssuePatch, IssueStatus, Project, createIssue, deleteIssue, getWorkspace, updateIssue } from "../api";
import { planReorder } from "../lib/issueRank";
import { Corpus, LinkEdge, buildLinkIndex, mentionsOf } from "../lib/links";
import { requestNavigate } from "../lib/navigate";
import { useRuns } from "../store/runs";
import { useIssues } from "../hooks/useIssues";
import { useCrossRefs } from "../hooks/useCrossRefs";
import { useDocs } from "../hooks/useDocs";
import { ISSUE_STATUSES, PENDING_ISSUE_KEY, PENDING_QUICKADD_KEY, STATUS_LABELS, compareIssues, isClosed, issueLabel, issuesExpandedKey } from "../lib/issues";
import { pickDefaultAgent } from "../lib/defaultAgent";
import { loadFold, saveFold, usePaneWidth } from "../hooks/usePaneWidth";
import IssueRow from "./IssueRow";
import IssueDetail from "./IssueDetail";
import ConfirmDialog from "./ConfirmDialog";
import Resizer from "./Resizer";
import { toastError } from "../lib/toast";

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
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<Issue | null>(null);
  // done/cancelled fold away by default; an open group stays open.
  const [openClosed, setOpenClosed] = useState<Set<string>>(new Set());
  // Quick-add rests as a + button and expands into an inline input on demand.
  const [quickOpen, setQuickOpen] = useState(false);
  const quickRef = useRef<HTMLInputElement>(null);

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

  useEffect(() => { setSelectedId(null); setQuick(""); setQuickOpen(false); }, [project.id]);
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

  const groups = useMemo(
    () =>
      ISSUE_STATUSES.map((status) => ({
        status,
        issues: issues.filter((i) => i.status === status).sort(compareIssues),
      })),
    [issues],
  );
  // Mouse handlers commit against the freshest grouping, not their closure's.
  const groupsRef = useRef(groups);
  groupsRef.current = groups;
  const visible = useMemo(
    () => groups.flatMap((g) => (isClosed(g.status) && !openClosed.has(g.status) ? [] : g.issues)),
    [groups, openClosed],
  );
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
  const { cross } = useCrossRefs(active);
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

  // Linear-flavored keys: `c` captures, arrows move, Enter opens, 0-4 set
  // priority. Skipped while typing in any field or when a dialog is up.
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
            {groups.map(({ status, issues: group }) => {
              if (group.length === 0) return null;
              const closed = isClosed(status);
              const open = !closed || openClosed.has(status);
              return (
                <section key={status} className="issue-group">
                  <button
                    className="issue-group-head"
                    onClick={() => {
                      if (!closed) return;
                      setOpenClosed((prev) => {
                        const next = new Set(prev);
                        if (next.has(status)) next.delete(status); else next.add(status);
                        return next;
                      });
                    }}
                  >
                    {closed && <span className="issue-group-chev">{open ? "▾" : "▸"}</span>}
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
                      drag={{
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
            expanded={wide}
            onToggleExpand={() => setExpanded(!expanded)}
            onPatch={(p) => patch(selected, p)}
            onDelete={() => setConfirmDelete(selected)}
            onOpenRun={openRun}
            onOpenMention={openMention}
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
