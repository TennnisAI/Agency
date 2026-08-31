import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { CloneProgress, Project, RunInfo, RepoReadiness, addProject, closeProject, deleteProject, inspectRepo, listProjects, listRuns, setProjectColor } from "../api";
import { projectAccent, runName } from "../agents";
import { useRuns } from "../store/runs";
import { toastError } from "../lib/toast";
import { pinnedFirst, runStatus } from "../lib/runstate";
import ConfirmDialog from "./ConfirmDialog";
import ProjectColorPicker from "./ProjectColorPicker";
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
  onSelectionGone,
  onToggleSidebar,
  onOpenSettings,
  updateAvailable = false,
}: {
  selectedId: string | null;
  focusedRunId: string | null;
  onSelect: (p: Project) => void;
  onSelectRun: (p: Project, run: RunInfo) => void;
  onHome: () => void;
  /** The selected project is gone from the list; drop back to the overview. */
  onSelectionGone: () => void;
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
  // Which of the confirm dialog's two actions is running, and the step the
  // backend is on: both stop every agent in the project, and "Delete worktrees"
  // also unlinks each one, so on a busy project they run for seconds.
  const [busy, setBusy] = useState<"close" | "delete" | null>(null);
  const [progress, setProgress] = useState<CloneProgress | null>(null);
  const [error, setError] = useState("");
  const [setup, setSetup] = useState<{ path: string; name: string; readiness: RepoReadiness; existing: boolean } | null>(null);
  const [cloning, setCloning] = useState(false);
  // Workspace-creation dialog; `intent` (e.g. "daily-note") is re-emitted via
  // an `agency:workspace-ready` event once the workspace exists, so the flow
  // that needed it (⌘⇧D on first use) can resume.
  const [wsCreate, setWsCreate] = useState<{ intent: string | null } | null>(null);
  // Open color picker: the project being recolored plus the icon rect it hangs off.
  const [recolor, setRecolor] = useState<{ project: Project; anchor: DOMRect } | null>(null);

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
  }
  useEffect(() => { refresh(); }, []);

  // Closing or deleting a project drops it out of the list, but the main view
  // was still pointed at it: every call the panels made then came back
  // "unknown project: 0a0a78d9-9063-442b-a2f9-9373382e199c" and the source
  // panel and history just sat there red. Fall back to the overview. This is
  // the one place all the removal routes meet — the row's Close/Delete dialog
  // and Settings hiding the workspace, which closes it — so the guard belongs
  // on the list itself, not on each caller.
  const goneRef = useRef(onSelectionGone);
  goneRef.current = onSelectionGone;
  useEffect(() => {
    if (selectedId && !projects.some((p) => p.id === selectedId)) goneRef.current();
  }, [projects, selectedId]);

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
      const lists = await Promise.all(ids.map((id) => listRuns(id).catch(() => [] as RunInfo[])));
      if (cancelled) return;
      // Pinned runs first here too: a pin is meant to hold a run's place
      // wherever it is listed, not only on the board.
      setProjectRuns(Object.fromEntries(ids.map((id, i) => [id, pinnedFirst(lists[i])])));
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

  // Double-clicking a project's icon opens the palette over it. The rect is
  // captured here rather than from a ref because every row shares one picker.
  function openRecolor(e: React.MouseEvent, p: Project) {
    e.stopPropagation();
    setRecolor({ project: p, anchor: e.currentTarget.getBoundingClientRect() });
  }

  async function pickColor(color: string) {
    if (!recolor) return;
    const { project } = recolor;
    setRecolor(null);
    // Paint the new color straight away; refresh() then re-reads the row so a
    // rejected write can't leave the sidebar showing a color the DB never took.
    setProjects((ps) => ps.map((p) => (p.id === project.id ? { ...p, color } : p)));
    try {
      await setProjectColor(project.id, color);
    } catch (e) {
      toastError(e, "Couldn't change the project color");
    }
    await refresh();
  }

  async function confirmPending(kind: "close" | "delete") {
    if (!pending || busy) return;
    const { project } = pending;
    setBusy(kind);
    setProgress(null);
    try {
      if (kind === "close") await closeProject(project.id, setProgress);
      else await deleteProject(project.id, setProgress);
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't close project");
    } finally {
      // Always drop the dialog — a failure must not leave it frozen open.
      setBusy(null);
      setProgress(null);
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
            <span
              className={`proj-icon ws-icon${workspace ? " recolorable" : ""}`}
              aria-hidden
              title={workspace ? "Double-click to change color" : undefined}
              style={workspace ? { background: projectAccent(workspace) } : undefined}
              onDoubleClick={(e) => { if (workspace) openRecolor(e, workspace); }}
            >◈</span>
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
              <span
                className="proj-icon recolorable"
                aria-hidden
                title="Double-click to change color"
                style={{ background: projectAccent(p) }}
                onDoubleClick={(e) => openRecolor(e, p)}
              >{p.name.slice(0, 1).toUpperCase()}</span>
              <span className="tree-name tl">{p.name}</span>
              {/* No "needs a commit" badge here. A repository with no commits
                  is not a broken project, it is a new one: agents run in the
                  checkout either way, and a folder with no repository at all
                  is more limited still and never got a badge. The one thing it
                  does block, cutting a worktree, is caught by `useSpawnAgent`
                  at the moment of the spawn, which raises the setup dialog with
                  "Create initial commit" on it — the explanation and the fix,
                  where the user is already looking. The badge said less, and
                  said it forever: nothing tells this component about a commit
                  made in Source Control, so it sat next to a History panel
                  already showing the commit until the app restarted. */}
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
      {recolor && (
        <ProjectColorPicker
          anchor={recolor.anchor}
          current={recolor.project.color ?? null}
          onPick={(c) => { void pickColor(c); }}
          onClose={() => setRecolor(null)}
        />
      )}
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
          busy={busy !== null}
          progress={progress}
          progressLabel={busy === "delete" ? "Deleting worktrees…" : "Closing…"}
          onAlt={() => confirmPending("delete")}
          onConfirm={() => confirmPending("close")}
          onCancel={() => setPending(null)}
        />
      )}
    </aside>
  );
}
