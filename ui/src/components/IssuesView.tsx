import { useEffect, useMemo, useRef, useState } from "react";
import { Issue, IssuePatch, IssueStatus, Project, createIssue, deleteIssue, updateIssue } from "../api";
import { planReorder } from "../lib/issueRank";
import { useRuns } from "../store/runs";
import { useIssues } from "../hooks/useIssues";
import { ISSUE_STATUSES, PENDING_ISSUE_KEY, PENDING_QUICKADD_KEY, STATUS_LABELS, compareIssues, isClosed, issueLabel } from "../lib/issues";
import { pickDefaultAgent } from "../lib/defaultAgent";
import IssueRow from "./IssueRow";
import IssueDetail from "./IssueDetail";
import ConfirmDialog from "./ConfirmDialog";
import { toastError } from "../lib/toast";

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

    const onMove = (ev: MouseEvent) => {
      if (!dragLive.current) {
        if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < 5) return;
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
      if (!d) return;
      suppressClick.current = true;
      window.setTimeout(() => { suppressClick.current = false; }, 0);
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

  return (
    <div className="issues-wrap">
      <div className="issues-main">
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
          <div className={`issues-list${drag ? " reordering" : ""}`}>
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
        <IssueDetail
          issue={selected}
          label={issueLabel(project, selected)}
          runs={runsFor(selected)}
          onPatch={(p) => patch(selected, p)}
          onDelete={() => setConfirmDelete(selected)}
          onOpenRun={openRun}
          onClose={() => setSelectedId(null)}
        />
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
