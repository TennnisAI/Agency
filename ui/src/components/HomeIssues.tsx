import { useEffect, useState } from "react";
import {
  Issue,
  IssuePatch,
  Project,
  RepoReadiness,
  RunInfo,
  TaskHit,
  agentInstalled,
  createIssue,
  deleteIssue,
  inspectRepo,
  startIssueRun,
  updateIssue,
} from "../api";
import { projectAccent } from "../agents";
import {
  ISSUE_STATUSES,
  IssueSort,
  PENDING_ISSUE_KEY,
  PRIORITY_LABELS,
  STATUS_LABELS,
  StatusFilter,
  isClosed,
  issueLabel,
  issueSorter,
  matchesFilters,
  searchTerms,
  todayIssues,
} from "../lib/issues";
import { dateStamp } from "../lib/dailyNote";
import { pickDefaultAgent } from "../lib/defaultAgent";
import { parentPath } from "../lib/filePath";
import { requestNavigate } from "../lib/navigate";
import { TaskExclusion, TaskGroup, isTaskExcluded, openTaskCount, promoteBody } from "../lib/tasks";
import { toastError } from "../lib/toast";
import { useAllTasks } from "../hooks/useAllTasks";
import { useGitlessProjects } from "../hooks/useRepoReadiness";
import { useRuns } from "../store/runs";
import Menu, { MenuEntry } from "./git/Menu";
import IssueRow from "./IssueRow";
import ConfirmDialog from "./ConfirmDialog";
import RepoSetupDialog from "./RepoSetupDialog";
import InstallAgentDialog from "./InstallAgentDialog";
import PillSelect from "./PillSelect";
import { Stat } from "./HomeView";

// Notes shown in the Tasks section before the explicit "N more" line.
// Collapsed groups are one row each, so this can be generous.
const TASK_GROUP_CAP = 30;

// Persisted Tasks-section state: which sources the user hid, and which note
// groups are expanded (groups start collapsed — plan docs carry hundreds of
// checkboxes and an expanded wall of them buries the board).
const TASKS_EXCLUDED_KEY = "home:tasksExcluded";
const TASKS_OPEN_KEY = "home:tasksOpen";
const TASKS_COLLAPSED_KEY = "home:tasksCollapsed";

function loadJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function saveJson(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* storage unavailable */
  }
}

// The cross-project issue board (one-stop Phase 6): Today on top, filters and
// search across every project, real IssueRows — inline edits and dispatch
// work without entering the project. Data arrives from HomeView's poll.
export default function HomeIssues({
  projects,
  issuesBy,
  runsBy,
  folded,
  onToggleFold,
  onOpenProject,
  onOpenRun,
  refresh,
}: {
  projects: Project[];
  issuesBy: Record<string, Issue[]>;
  runsBy: Record<string, RunInfo[]>;
  folded: Set<string>;
  onToggleFold: (id: string) => void;
  onOpenProject: (p: Project) => void;
  onOpenRun: (p: Project, runId: string) => void;
  refresh: () => Promise<void> | void;
}) {
  const { setTab } = useRuns();
  const today = dateStamp(new Date());
  // Which projects sit in a folder with no git repository, so their issue rows
  // can hide the entries that need a branch. Cached per folder rather than read
  // per poll: see useGitlessProjects.
  const gitless = useGitlessProjects(projects);

  const [q, setQ] = useState("");
  const [fStatus, setFStatus] = useState<StatusFilter>("open");
  const [fPriority, setFPriority] = useState(-1);
  const [fProject, setFProject] = useState("");
  const [sort, setSort] = useState<IssueSort>("board");

  const [confirmDelete, setConfirmDelete] = useState<{ project: Project; issue: Issue } | null>(null);
  const [pendingSpawn, setPendingSpawn] = useState<{
    project: Project;
    issue: Issue;
    agentId: string;
    readiness: RepoReadiness;
    opts?: { base: string; mergeTarget: string };
  } | null>(null);
  const [missingAgent, setMissingAgent] = useState<{ project: Project; agentId: string } | null>(null);

  const terms = searchTerms(q);
  const filters = { terms, status: fStatus, priority: fPriority };

  const shownProjects = projects.filter((p) => !fProject || p.id === fProject);
  const compare = issueSorter(sort);
  const issuesOf = (p: Project) =>
    (issuesBy[p.id] ?? []).filter((i) => matchesFilters(i, issueLabel(p, i), filters)).sort(compare);

  const byProject = new Map(projects.map((p) => [p.id, p] as const));
  // Today spans the filtered set; each entry keeps its project for labels.
  const todays = todayIssues(shownProjects.flatMap((p) => issuesOf(p)), today);

  const totalOpen = projects.reduce(
    (n, p) => n + (issuesBy[p.id] ?? []).filter((i) => !isClosed(i.status)).length,
    0,
  );
  const active = projects.reduce(
    (n, p) =>
      n +
      (issuesBy[p.id] ?? []).filter((i) => i.status === "in_progress" || i.status === "in_review").length,
    0,
  );
  // Busiest boards first; ties break alphabetically.
  const ordered = [...shownProjects].sort(
    (a, b) => issuesOf(b).length - issuesOf(a).length || a.name.localeCompare(b.name),
  );

  // Navigating resets the tab to "agents" (setSelectedProject), so re-assert
  // the Issues tab after — React batches both in this handler. The clicked
  // issue rides sessionStorage; IssuesView selects it once its list loads.
  function openIssue(p: Project, issue: Issue) {
    sessionStorage.setItem(PENDING_ISSUE_KEY, issue.id);
    onOpenProject(p);
    setTab("issues");
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
    try {
      await deleteIssue(issue.id);
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't delete issue");
    }
  }

  // Dispatch straight from the board: the same installed/readiness checks as
  // the per-project flow, then jump into the new run's focus view.
  async function dispatch(
    project: Project,
    issue: Issue,
    agentId: string,
    opts?: { base: string; mergeTarget: string },
  ) {
    try {
      const installed = await agentInstalled(agentId).catch(() => true);
      if (!installed) {
        setMissingAgent({ project, agentId });
        return;
      }
      const r = await inspectRepo(project.repo_path);
      // A folder with no repository has nothing to set up: there is no branch to
      // cut from, so the agent takes the issue on in the folder as it stands.
      // Without that arm the board opened the setup dialog on "Initialize
      // repository", whose only other button is Cancel, so dispatching into a
      // scratch folder from here had no way through. The per-project flow makes
      // the same short-circuit in useSpawnAgent.
      const spawnable = r.state === "notARepo" || (r.state === "ready" && !r.dirty);
      if (spawnable) {
        const run = await startIssueRun(issue.id, agentId, opts?.base, opts?.mergeTarget);
        onOpenRun(project, run.id);
      } else {
        setPendingSpawn({ project, issue, agentId, readiness: r, opts });
      }
    } catch (e) {
      toastError(e, "Couldn't start agent");
    }
  }

  async function dispatchDefault(project: Project, issue: Issue) {
    const agent = await pickDefaultAgent(project.id, project.default_agent);
    await dispatch(project, issue, agent);
  }

  // ── Tasks: checkboxes across every note, checked in place ────────────
  const { groups: taskData, toggle: toggleTaskBox } = useAllTasks(true);
  // Optimistic strikethrough while the write + poll round-trips.
  const [struck, setStruck] = useState<Set<string>>(new Set());
  const taskKey = (g: TaskGroup, t: TaskHit) => `${g.project.id}:${g.path}:${t.line}`;
  // A refreshed scan drops checked tasks, so stale strike keys can go too.
  useEffect(() => setStruck(new Set()), [taskData]);

  const [taskExclusions, setTaskExclusions] = useState<TaskExclusion[]>(
    () => loadJson<TaskExclusion[]>(TASKS_EXCLUDED_KEY, []),
  );
  const [tasksOpen, setTasksOpen] = useState<Set<string>>(
    () => new Set(loadJson<string[]>(TASKS_OPEN_KEY, [])),
  );
  const [taskMenu, setTaskMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  // The whole card folds to its header line; "show all" lifts the group cap
  // for this visit (a persisted lift would quietly bring the wall back).
  const [tasksCollapsed, setTasksCollapsed] = useState<boolean>(
    () => loadJson<boolean>(TASKS_COLLAPSED_KEY, false),
  );
  const [showAllTaskGroups, setShowAllTaskGroups] = useState(false);

  const groupKey = (g: TaskGroup) => `${g.project.id}:${g.path}`;
  const setOpenSet = (next: Set<string>) => {
    setTasksOpen(next);
    saveJson(TASKS_OPEN_KEY, [...next]);
  };
  const toggleTasksCollapsed = () => {
    setTasksCollapsed(!tasksCollapsed);
    saveJson(TASKS_COLLAPSED_KEY, !tasksCollapsed);
  };
  const setExclusions = (next: TaskExclusion[]) => {
    setTaskExclusions(next);
    saveJson(TASKS_EXCLUDED_KEY, next);
  };
  const addExclusion = (rule: TaskExclusion) => setExclusions([...taskExclusions, rule]);

  // The "N hidden" button lists every hidden source; clicking one unhides it.
  function openHiddenMenu(e: React.MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const items: MenuEntry[] = [
      ...taskExclusions.map((r, i): MenuEntry => {
        const name = byProject.get(r.projectId)?.name ?? "removed project";
        return {
          label: r.prefix === "" ? `Unhide all ${name} tasks` : `Unhide ${r.prefix} (${name})`,
          onClick: () => setExclusions(taskExclusions.filter((_, j) => j !== i)),
        };
      }),
      { kind: "separator" },
      { label: "Unhide everything", onClick: () => setExclusions([]) },
    ];
    setTaskMenu({ x: e.clientX, y: e.clientY, items });
  }

  const taskGroups = (taskData ?? [])
    .map((g) => ({ ...g, tasks: g.tasks.filter((t) => !t.checked) }))
    .filter(
      (g) =>
        g.tasks.length > 0 &&
        (!fProject || g.project.id === fProject) &&
        !isTaskExcluded(g.project.id, g.path, taskExclusions),
    );
  const allOpen = taskGroups.length > 0 && taskGroups.every((g) => tasksOpen.has(groupKey(g)));

  function openTaskMenu(e: React.MouseEvent, g: TaskGroup) {
    e.preventDefault();
    e.stopPropagation();
    const folder = parentPath(g.path);
    const items: MenuEntry[] = [
      {
        label: "Open note",
        onClick: () => requestNavigate({ kind: "note", projectId: g.project.id, path: g.path }),
      },
      { kind: "separator" },
      { label: `Hide "${g.title}"`, onClick: () => addExclusion({ projectId: g.project.id, prefix: g.path }) },
      ...(folder
        ? [{ label: `Hide folder "${folder}/"`, onClick: () => addExclusion({ projectId: g.project.id, prefix: folder }) }]
        : []),
      { label: `Hide all ${g.project.name} tasks`, onClick: () => addExclusion({ projectId: g.project.id, prefix: "" }) },
    ];
    setTaskMenu({ x: e.clientX, y: e.clientY, items });
  }

  async function checkTask(g: TaskGroup, t: TaskHit) {
    const key = taskKey(g, t);
    setStruck((s) => new Set(s).add(key));
    try {
      await toggleTaskBox(g, t, true);
    } catch (e) {
      setStruck((s) => { const n = new Set(s); n.delete(key); return n; });
      toastError(e, "Couldn't update task");
    }
  }

  async function promoteTask(g: TaskGroup, t: TaskHit) {
    try {
      const issue = await createIssue(g.project.id, t.text, promoteBody(g.path), "todo");
      await refresh();
      openIssue(g.project, issue);
    } catch (e) {
      toastError(e, "Couldn't create issue");
    }
  }

  const row = (project: Project, issue: Issue) => (
    <IssueRow
      key={issue.id}
      issue={issue}
      label={issueLabel(project, issue)}
      runs={(runsBy[project.id] ?? []).filter((r) => r.issueId === issue.id)}
      selected={false}
      onSelect={() => openIssue(project, issue)}
      onStart={() => { dispatchDefault(project, issue); }}
      onSpawnAgent={(agentId, opts) => { dispatch(project, issue, agentId, opts); }}
      onPatch={(p) => patch(issue, p)}
      onDelete={() => setConfirmDelete({ project, issue })}
      terms={terms}
      gitless={gitless.has(project.id)}
    />
  );

  const filtered = fStatus !== "open" || fPriority >= 0 || !!fProject || terms.length > 0;
  const shownCount = ordered.reduce((n, p) => n + issuesOf(p).length, 0);

  return (
    <div className="home">
      <div className="home-head">
        <div>
          <div className="eyebrow">ALL PROJECTS</div>
          <h1 className="home-title">Issues</h1>
        </div>
        <div className="home-stats">
          <Stat value={projects.length} label={projects.length === 1 ? "project" : "projects"} />
          <Stat value={totalOpen} label="open" />
          <Stat value={active} label="with agents" accent={active > 0} />
        </div>
      </div>

      <div className="issues-filterbar">
        <span className="filter-search">
          <span className="filter-search-glyph">⌕</span>
          <input
            placeholder="Search issues…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Escape") (e.target as HTMLInputElement).blur(); }}
          />
          {q && (
            <button className="filter-clear" title="Clear search" onClick={() => setQ("")}>✕</button>
          )}
        </span>
        <div className="spacer" />
        <PillSelect<StatusFilter>
          value={fStatus}
          defaultValue="open"
          title="Status"
          onChange={setFStatus}
          options={[
            { value: "open", label: "Open" },
            { value: "all", label: "All statuses" },
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
        <PillSelect
          value={fProject}
          defaultValue=""
          title="Project"
          onChange={setFProject}
          options={[
            { value: "", label: "All projects" },
            ...projects.map((p) => ({ value: p.id, label: p.name })),
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
      </div>

      <section className="today-group">
        <div className="today-head">
          <span className="today-title">Today</span>
          <span className="today-meta">
            {todays.length === 0
              ? "due, scheduled, and in-progress issues land here"
              : `${todays.length} to work on`}
          </span>
        </div>
        {todays.length === 0 ? (
          <div className="today-empty">Nothing due or in progress. Set a due date to plan your day.</div>
        ) : (
          <div className="home-issues">
            {todays.map((issue) => {
              const p = byProject.get(issue.projectId);
              return p ? row(p, issue) : null;
            })}
          </div>
        )}
      </section>

      {(taskGroups.length > 0 || taskExclusions.length > 0) && (
        <section className={`today-group tasks-group${tasksCollapsed ? " collapsed" : ""}`}>
          <div className="today-head">
            <button
              className="task-collapse"
              title={tasksCollapsed ? "Expand tasks" : "Collapse tasks"}
              onClick={toggleTasksCollapsed}
            >
              <span className="task-note-chev">{tasksCollapsed ? "▸" : "▾"}</span>
              <span className="today-title">Tasks</span>
              <span className="today-meta">{openTaskCount(taskGroups)} unchecked in notes</span>
            </button>
            {!tasksCollapsed && (
              <span className="task-head-tools">
                {taskExclusions.length > 0 && (
                  <button
                    className="task-head-btn"
                    title="List the hidden sources"
                    onClick={openHiddenMenu}
                  >
                    {taskExclusions.length} hidden
                  </button>
                )}
                {taskGroups.length > 0 && (
                  <button
                    className="task-head-btn"
                    onClick={() =>
                      setOpenSet(allOpen ? new Set() : new Set(taskGroups.map(groupKey)))
                    }
                  >
                    {allOpen ? "Collapse all" : "Expand all"}
                  </button>
                )}
              </span>
            )}
          </div>
          {!tasksCollapsed &&
            (showAllTaskGroups ? taskGroups : taskGroups.slice(0, TASK_GROUP_CAP)).map((g) => {
            const gk = groupKey(g);
            const isOpen = tasksOpen.has(gk);
            return (
              <div key={gk} className="task-note">
                <button
                  className="task-note-head"
                  title={g.path}
                  onClick={() => {
                    const next = new Set(tasksOpen);
                    if (isOpen) next.delete(gk);
                    else next.add(gk);
                    setOpenSet(next);
                  }}
                  onContextMenu={(e) => openTaskMenu(e, g)}
                >
                  <span className="task-note-chev">{isOpen ? "▾" : "▸"}</span>
                  <span className="proj-icon" aria-hidden style={{ background: projectAccent(g.project) }}>
                    {g.project.name.slice(0, 1).toUpperCase()}
                  </span>
                  <span className="task-note-title">{g.title}</span>
                  <span className="task-note-count">{g.tasks.length}</span>
                  <span className="task-note-meta">{g.project.name}</span>
                  <span
                    className="task-note-dots"
                    title="Open or hide this source"
                    onClick={(e) => openTaskMenu(e, g)}
                  >
                    ⋯
                  </span>
                </button>
                {isOpen &&
                  g.tasks.map((t) => {
                    const key = taskKey(g, t);
                    const isStruck = struck.has(key);
                    return (
                      <div key={key} className={`task-row${isStruck ? " struck" : ""}`}>
                        <button
                          className={`task-check${isStruck ? " checked" : ""}`}
                          role="checkbox"
                          aria-checked={isStruck}
                          title="Check off in the note"
                          onClick={() => { if (!isStruck) void checkTask(g, t); }}
                        />
                        <span className="task-text">{t.text}</span>
                        <button
                          className="task-promote"
                          title="Promote to issue"
                          onClick={() => void promoteTask(g, t)}
                        >
                          ▧ promote
                        </button>
                      </div>
                    );
                  })}
              </div>
            );
          })}
          {!tasksCollapsed && taskGroups.length > TASK_GROUP_CAP && (
            <button
              className="task-more"
              onClick={() => setShowAllTaskGroups((s) => !s)}
            >
              {showAllTaskGroups
                ? "Show fewer"
                : `Show ${taskGroups.length - TASK_GROUP_CAP} more notes`}
            </button>
          )}
          {!tasksCollapsed && taskGroups.length === 0 && (
            <div className="today-empty">All visible tasks are done or hidden.</div>
          )}
          {taskMenu && (
            <Menu x={taskMenu.x} y={taskMenu.y} items={taskMenu.items} onClose={() => setTaskMenu(null)} />
          )}
        </section>
      )}

      {shownCount === 0 && filtered && (
        <div className="board empty">No issues match the current filters.</div>
      )}

      {ordered.map((p) => {
        const open = issuesOf(p);
        const isFolded = folded.has(p.id);
        return (
          <section key={p.id} className="home-group">
            <div className="home-group-head">
              <button
                className="home-fold"
                title={isFolded ? "Expand project" : "Collapse project"}
                onClick={() => onToggleFold(p.id)}
              >
                {isFolded ? "▸" : "▾"}
              </button>
              <button className="home-group-title" onClick={() => { onOpenProject(p); setTab("issues"); }} title={`Open ${p.name} issues`}>
                <span className="proj-icon" aria-hidden style={{ background: projectAccent(p) }}>
                  {p.name.slice(0, 1).toUpperCase()}
                </span>
                <span className="home-group-name">{p.name}</span>
                <span className="home-group-meta">
                  {open.length === 0
                    ? filtered ? "no matching issues" : "no open issues"
                    : `${open.length} ${filtered ? "matching" : "open"} issue${open.length === 1 ? "" : "s"}`}
                </span>
                <span className="home-group-open">open →</span>
              </button>
            </div>
            {!isFolded && open.length > 0 && (
              <div className="home-issues">{open.map((issue) => row(p, issue))}</div>
            )}
          </section>
        );
      })}

      {confirmDelete && (
        <ConfirmDialog
          title="Delete issue?"
          body={`Delete "${issueLabel(confirmDelete.project, confirmDelete.issue)} ${confirmDelete.issue.title}". Its number is never reused. This cannot be undone.`}
          confirmLabel="Delete"
          danger
          onConfirm={() => { doDelete(confirmDelete.issue); }}
          onCancel={() => setConfirmDelete(null)}
        />
      )}

      {pendingSpawn && (
        <RepoSetupDialog
          readiness={pendingSpawn.readiness}
          context="spawn"
          repoPath={pendingSpawn.project.repo_path}
          onResolved={async () => {
            const { project, issue, agentId, opts } = pendingSpawn;
            setPendingSpawn(null);
            try {
              const run = await startIssueRun(issue.id, agentId, opts?.base, opts?.mergeTarget);
              onOpenRun(project, run.id);
            } catch (e) {
              toastError(e, "Couldn't start agent");
            }
          }}
          onCancel={() => setPendingSpawn(null)}
        />
      )}

      {missingAgent && (
        <InstallAgentDialog
          agent={missingAgent.agentId}
          projectId={missingAgent.project.id}
          onInstalling={(run) => {
            const p = missingAgent.project;
            setMissingAgent(null);
            onOpenRun(p, run.id);
          }}
          onCancel={() => setMissingAgent(null)}
        />
      )}
    </div>
  );
}
