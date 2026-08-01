import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Project, RunInfo, RepoReadiness, addProject, closeProject, deleteProject, inspectRepo, listProjects, listRuns } from "../api";
import { projectAccent, runName } from "../agents";
import { useRuns } from "../store/runs";
import { toastError } from "../lib/toast";
import { runStatus } from "../lib/runstate";
import ConfirmDialog from "./ConfirmDialog";
import RepoSetupDialog from "./RepoSetupDialog";
import CloneDialog from "./CloneDialog";
import SidebarToggle from "./SidebarToggle";
import WorkspaceCreateDialog from "./WorkspaceCreateDialog";
import { WORKSPACE_HIDDEN_EVENT, workspaceHidden } from "../lib/workspacePref";

type Pending = { project: Project } | null;

// "active" = the project has at least one live agent or terminal, i.e. at least
// one row would appear under it in the tree.
type Filter = "all" | "active";

export default function ProjectTree({
  selectedId,
  focusedRunId,
  onSelect,
  onSelectRun,
  onHome,
  onToggleSidebar,
  onOpenSettings,
  updateAvailable = false,
}: {
  selectedId: string | null;
  focusedRunId: string | null;
  onSelect: (p: Project) => void;
  onSelectRun: (p: Project, run: RunInfo) => void;
  onHome: () => void;
  onToggleSidebar: () => void;
  onOpenSettings: () => void;
  /** Marks the Settings button with a dot — a newer release is on GitHub. */
  updateAvailable?: boolean;
}) {
  const { runs } = useRuns();
  const [projects, setProjects] = useState<Project[]>([]);
  const [openIds, setOpenIds] = useState<Set<string>>(() => new Set());
  const [projectRuns, setProjectRuns] = useState<Record<string, RunInfo[]>>({});
  const [filter, setFilter] = useState<Filter>("all");
  const [pending, setPending] = useState<Pending>(null);
  const [error, setError] = useState("");
  const [setup, setSetup] = useState<{ path: string; name: string; readiness: RepoReadiness; existing: boolean } | null>(null);
  const [cloning, setCloning] = useState(false);
  const [readiness, setReadiness] = useState<Record<string, RepoReadiness>>({});
  // Workspace-creation dialog; `intent` (e.g. "daily-note") is re-emitted via
  // an `agency:workspace-ready` event once the workspace exists, so the flow
  // that needed it (⌘⇧D on first use) can resume.
  const [wsCreate, setWsCreate] = useState<{ intent: string | null } | null>(null);

  // The pinned workspace is a project row flagged `kind: "workspace"` — shown
  // above the list, never part of the active filter, not closable. Users who
  // don't want it can hide it entirely (Settings ▸ Workspace).
  const workspace = projects.find((p) => p.kind === "workspace") ?? null;
  const repoProjects = projects.filter((p) => p.kind !== "workspace");
  const [wsHidden, setWsHidden] = useState(workspaceHidden());
  useEffect(() => {
    const onChange = () => { setWsHidden(workspaceHidden()); void refresh(); };
    window.addEventListener(WORKSPACE_HIDDEN_EVENT, onChange);
    return () => window.removeEventListener(WORKSPACE_HIDDEN_EVENT, onChange);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Guarded against out-of-order responses: back-to-back refreshes (e.g. the
  // workspace toggle fires one before and one after the backend settles) must
  // not let a stale project list land last and hide a just-revived row.
  const refreshSeq = useRef(0);
  async function refresh() {
    const seq = ++refreshSeq.current;
    const ps = await listProjects();
    if (seq !== refreshSeq.current) return;
    setProjects(ps);
    const entries = await Promise.all(
      ps.map(async (p) => [p.id, await inspectRepo(p.repo_path).catch(() => null)] as const),
    );
    if (seq !== refreshSeq.current) return;
    setReadiness(Object.fromEntries(entries.filter(([, r]) => r) as [string, RepoReadiness][]));
  }
  useEffect(() => { refresh(); }, []);

  // Open the workspace, creating it first if it doesn't exist yet (it is lazy
  // by design — no surprise folders at app launch). `intent` is forwarded to
  // `agency:workspace-ready` so the caller's flow can resume after creation.
  function openWorkspace(intent: string | null) {
    if (workspace) {
      onSelect(workspace);
      if (intent) {
        window.dispatchEvent(new CustomEvent("agency:workspace-ready", { detail: { intent } }));
      }
    } else {
      setWsCreate({ intent });
    }
  }

  async function workspaceCreated(ws: Project) {
    const intent = wsCreate?.intent ?? null;
    setWsCreate(null);
    await refresh();
    onSelect(ws);
    if (intent) {
      window.dispatchEvent(new CustomEvent("agency:workspace-ready", { detail: { intent } }));
    }
  }

  // The "Add Project…" menu item lives in the native menu bar; it dispatches a
  // DOM event that triggers the same add flow as the sidebar's + button. Kept
  // in a ref so the once-bound listener always calls the latest handleAdd.
  // `agency:create-workspace` is the same pattern for flows (⌘⇧D, palette)
  // that need the workspace to exist.
  const addRef = useRef(handleAdd);
  addRef.current = handleAdd;
  const openWorkspaceRef = useRef(openWorkspace);
  openWorkspaceRef.current = openWorkspace;
  useEffect(() => {
    const add = () => addRef.current();
    const clone = () => setCloning(true);
    const ws = (e: Event) =>
      openWorkspaceRef.current((e as CustomEvent<{ intent?: string }>).detail?.intent ?? null);
    window.addEventListener("agency:add-project", add);
    window.addEventListener("agency:clone-project", clone);
    window.addEventListener("agency:create-workspace", ws);
    return () => {
      window.removeEventListener("agency:add-project", add);
      window.removeEventListener("agency:clone-project", clone);
      window.removeEventListener("agency:create-workspace", ws);
    };
  }, []);

  // Keep the agents shown under each project in sync with the shared run store.
  // Re-fetching whenever the project list or the global `runs` change (discard,
  // spawn, or the store's poll) means the tree can't keep showing an agent the
  // Agents rail has already dropped. Every project is fetched, not just the
  // expanded ones, because the "active" filter needs a run count for all of
  // them — the shared store only ever holds the selected project's runs.
  useEffect(() => {
    let cancelled = false;
    const ids = projects.map((p) => p.id);
    if (ids.length === 0) { setProjectRuns({}); return; }
    (async () => {
      const lists = await Promise.all(ids.map((id) => listRuns(id).catch(() => [])));
      if (cancelled) return;
      setProjectRuns(Object.fromEntries(ids.map((id, i) => [id, lists[i]])));
    })();
    return () => { cancelled = true; };
  }, [projects, runs]);

  const visible = filter === "active"
    ? repoProjects.filter((p) => (projectRuns[p.id] ?? []).length > 0)
    : repoProjects;

  function toggle(p: Project) {
    setOpenIds((s) => {
      const n = new Set(s);
      if (n.has(p.id)) n.delete(p.id); else n.add(p.id);
      return n;
    });
  }

  async function handleAdd() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    const name = sel.split("/").filter(Boolean).pop() ?? sel;
    setError("");
    try {
      const r = await inspectRepo(sel);
      if (r.state === "ready" && !r.dirty) {
        const created = await addProject(name, sel);
        await refresh();
        onSelect(created);
      } else {
        setSetup({ path: sel, name, readiness: r, existing: false });
      }
    } catch (e) {
      setError(String(e));
    }
  }

  // A freshly cloned repo already has commits and a clean tree, so it's ready to
  // add straight away — no setup dialog needed. Auto-open it like a fresh add.
  async function handleCloned(path: string) {
    setCloning(false);
    const name = path.split("/").filter(Boolean).pop() ?? path;
    setError("");
    try {
      const created = await addProject(name, path);
      await refresh();
      onSelect(created);
    } catch (e) {
      setError(String(e));
    }
  }

  async function finishSetup() {
    if (!setup) return;
    try {
      // Only create the record for the add flow; the badge flow's project already exists.
      const created = setup.existing ? null : await addProject(setup.name, setup.path);
      await refresh();
      // Auto-open a freshly added project; the badge (commit) flow leaves the
      // current selection alone.
      if (created) onSelect(created);
    } catch (e) {
      setError(String(e));
    }
    setSetup(null);
  }

  async function confirmPending(kind: "close" | "delete") {
    if (!pending) return;
    const { project } = pending;
    try {
      if (kind === "close") await closeProject(project.id);
      else await deleteProject(project.id);
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't close project");
    } finally {
      // Always drop the dialog — a failure must not leave it frozen open.
      setPending(null);
    }
  }

  return (
    <aside className="tree">
      <div className="tree-head">
        <SidebarToggle open onToggle={onToggleSidebar} />
        <button className="eyebrow eyebrow-link" title="All projects overview" onClick={onHome}>
          PROJECTS
        </button>
        <span className="spacer" />
        <button
          className={`icon-add icon-filter${filter === "active" ? " on" : ""}`}
          title={filter === "active"
            ? "Showing active projects only. Click to show all."
            : "Show only active projects (those with an agent or terminal)"}
          aria-label="Show only active projects"
          aria-pressed={filter === "active"}
          onClick={() => setFilter((f) => (f === "active" ? "all" : "active"))}
        >◉</button>
        <button className="icon-add" title="Clone repository" aria-label="Clone repository" onClick={() => setCloning(true)}>⤓</button>
        <button className="icon-add" title="Add project" aria-label="Add project" onClick={handleAdd}>+</button>
      </div>
      {error && <div className="git-error">{error}</div>}
      <ul className="tree-list">
        {/* Pinned workspace: always first, outside the active filter. Before
            first use it's an invitation — clicking sets it up. */}
        {!wsHidden && (
        <li className="tree-workspace">
          <div
            className={`tree-row ${workspace && workspace.id === selectedId ? (focusedRunId ? "selected ancestor" : "selected") : ""}${workspace ? "" : " ws-absent"}`}
            title={workspace ? workspace.repo_path : "Create your workspace: a home for journaling, planning, and notes"}
            onClick={() => openWorkspace(null)}
          >
            <span className="chev" onClick={(e) => { e.stopPropagation(); if (workspace) toggle(workspace); }}>
              {workspace && openIds.has(workspace.id) ? "▾" : "▸"}
            </span>
            <span className="proj-icon ws-icon" aria-hidden style={workspace ? { background: projectAccent(workspace) } : undefined}>◈</span>
            <span className="tree-name tl">{workspace?.name ?? "Workspace"}</span>
          </div>
          {workspace && openIds.has(workspace.id) && (
            <ul className="tree-children">
              {(projectRuns[workspace.id] ?? []).map((r) => (
                <li
                  key={r.id}
                  className={`tree-child ${r.id === focusedRunId ? "active" : ""}`}
                  style={{ "--sel-accent": projectAccent(workspace) } as React.CSSProperties}
                  title={`Open ${r.agent}: ${runName(r)}`}
                  onClick={(e) => { e.stopPropagation(); onSelectRun(workspace, r); }}
                >
                  <span className={`dot ${runStatus(r).cls}`} />
                  <span className="tree-child-name tl">{r.agent}: {runName(r)}</span>
                </li>
              ))}
            </ul>
          )}
        </li>
        )}
        {filter === "active" && visible.length === 0 && repoProjects.length > 0 && (
          <li className="tree-empty">No active projects.</li>
        )}
        {visible.map((p) => (
          <li key={p.id}>
            <div
              className={`tree-row ${p.id === selectedId ? (focusedRunId ? "selected ancestor" : "selected") : ""}`}
              onClick={() => onSelect(p)}
            >
              <span className="chev" onClick={(e) => { e.stopPropagation(); toggle(p); }}>
                {openIds.has(p.id) ? "▾" : "▸"}
              </span>
              <span className="proj-icon" aria-hidden style={{ background: projectAccent(p) }}>{p.name.slice(0, 1).toUpperCase()}</span>
              <span className="tree-name tl">{p.name}</span>
              {readiness[p.id]?.state === "noCommits" && (
                <button
                  className="row-badge warn"
                  title="Needs a commit before agents can run"
                  aria-label="Needs a commit before agents can run"
                  onClick={(e) => { e.stopPropagation(); setSetup({ path: p.repo_path, name: p.name, readiness: readiness[p.id]!, existing: true }); }}
                >{"⚠︎"} commit</button>
              )}
              <button className="row-act" title="Close project" aria-label="Close project" onClick={(e) => { e.stopPropagation(); setPending({ project: p }); }}>×</button>
            </div>
            {openIds.has(p.id) && (
              <ul className="tree-children">
                {(projectRuns[p.id] ?? []).map((r) => (
                  <li
                    key={r.id}
                    className={`tree-child ${r.id === focusedRunId ? "active" : ""}`}
                    style={{ "--sel-accent": projectAccent(p) } as React.CSSProperties}
                    title={`Open ${r.agent}: ${runName(r)}`}
                    onClick={(e) => { e.stopPropagation(); onSelectRun(p, r); }}
                  >
                    <span className={`dot ${runStatus(r).cls}`} />
                    <span className="tree-child-name tl">{r.agent}: {runName(r)}</span>
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
      <div className="tree-foot">
        <button
          className="tree-settings"
          aria-label={updateAvailable ? "Settings (an update is available)" : "Settings"}
          onClick={onOpenSettings}
        >
          <span className="tree-settings-gear">{"⚙︎"}</span> Settings
          {updateAvailable && <span className="tree-settings-dot" aria-hidden="true" />}
        </button>
      </div>
      {setup && (
        <RepoSetupDialog
          readiness={setup.readiness}
          context="add"
          repoPath={setup.path}
          onResolved={finishSetup}
          onCancel={() => setSetup(null)}
        />
      )}
      {cloning && (
        <CloneDialog onCloned={handleCloned} onCancel={() => setCloning(false)} />
      )}
      {wsCreate && (
        <WorkspaceCreateDialog
          onCreated={(ws) => { void workspaceCreated(ws); }}
          onCancel={() => setWsCreate(null)}
        />
      )}
      {pending && (
        <ConfirmDialog
          title="Close project?"
          body={`Stop all agents in "${pending.project.name}" and remove it from the sidebar. Everything on disk is kept; add the project again to pick up where you left off. "Delete worktrees & close" also deletes the agents' worktrees and branches, including unmerged work. Your repository files are never touched.`}
          confirmLabel="Close project"
          altLabel="Delete worktrees & close"
          altDanger
          onAlt={() => confirmPending("delete")}
          onConfirm={() => confirmPending("close")}
          onCancel={() => setPending(null)}
        />
      )}
    </aside>
  );
}
