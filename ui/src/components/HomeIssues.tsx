import { useState } from "react";
import {
  Issue,
  IssuePatch,
  IssueStatus,
  Project,
  RepoReadiness,
  RunInfo,
  agentInstalled,
  deleteIssue,
  inspectRepo,
  startIssueRun,
  updateIssue,
} from "../api";
import { projectAccent } from "../agents";
import {
  ISSUE_STATUSES,
  PENDING_ISSUE_KEY,
  PRIORITY_LABELS,
  STATUS_LABELS,
  compareIssues,
  isClosed,
  issueLabel,
  todayIssues,
} from "../lib/issues";
import { dateStamp } from "../lib/dailyNote";
import { pickDefaultAgent } from "../lib/defaultAgent";
import { toastError } from "../lib/toast";
import { useRuns } from "../store/runs";
import IssueRow from "./IssueRow";
import ConfirmDialog from "./ConfirmDialog";
import RepoSetupDialog from "./RepoSetupDialog";
import InstallAgentDialog from "./InstallAgentDialog";
import PillSelect from "./PillSelect";
import { Stat } from "./HomeView";

type StatusFilter = "open" | IssueStatus;
type Sort = "board" | "due" | "updated";

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

  const [q, setQ] = useState("");
  const [fStatus, setFStatus] = useState<StatusFilter>("open");
  const [fPriority, setFPriority] = useState(-1);
  const [fProject, setFProject] = useState("");
  const [sort, setSort] = useState<Sort>("board");

  const [confirmDelete, setConfirmDelete] = useState<{ project: Project; issue: Issue } | null>(null);
  const [pendingSpawn, setPendingSpawn] = useState<{
    project: Project;
    issue: Issue;
    agentId: string;
    readiness: RepoReadiness;
    opts?: { base: string; mergeTarget: string };
  } | null>(null);
  const [missingAgent, setMissingAgent] = useState<{ project: Project; agentId: string } | null>(null);

  const needle = q.trim().toLowerCase();
  const matches = (i: Issue) =>
    (fStatus === "open" ? !isClosed(i.status) : i.status === fStatus) &&
    (fPriority < 0 || i.priority === fPriority) &&
    (!needle || i.title.toLowerCase().includes(needle) || i.body.toLowerCase().includes(needle));

  const shownProjects = projects.filter((p) => !fProject || p.id === fProject);
  const compare = (a: Issue, b: Issue): number => {
    if (sort === "due") {
      const da = a.due ?? "9999";
      const db = b.due ?? "9999";
      if (da !== db) return da < db ? -1 : 1;
      return compareIssues(a, b);
    }
    if (sort === "updated") return b.updatedAt - a.updatedAt;
    return ISSUE_STATUSES.indexOf(a.status) - ISSUE_STATUSES.indexOf(b.status) || compareIssues(a, b);
  };
  const issuesOf = (p: Project) => (issuesBy[p.id] ?? []).filter(matches).sort(compare);

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
      if (r.state === "ready" && !r.dirty) {
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
    />
  );

  const filtered = fStatus !== "open" || fPriority >= 0 || !!fProject || !!needle;
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
        <PillSelect<Sort>
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
