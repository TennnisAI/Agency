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

type Pending =
  | { kind: "close" | "remove"; project: Project }
  | null;

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
}: {
  selectedId: string | null;
  focusedRunId: string | null;
  onSelect: (p: Project) => void;
  onSelectRun: (p: Project, run: RunInfo) => void;
  onHome: () => void;
  onToggleSidebar: () => void;
  onOpenSettings: () => void;
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

  async function refresh() {
    const ps = await listProjects();
    setProjects(ps);
    const entries = await Promise.all(
      ps.map(async (p) => [p.id, await inspectRepo(p.repo_path).catch(() => null)] as const),
    );
    setReadiness(Object.fromEntries(entries.filter(([, r]) => r) as [string, RepoReadiness][]));
  }
  useEffect(() => { refresh(); }, []);

  // The "Add Project…" menu item lives in the native menu bar; it dispatches a
  // DOM event that triggers the same add flow as the sidebar's + button. Kept
  // in a ref so the once-bound listener always calls the latest handleAdd.
  const addRef = useRef(handleAdd);
  addRef.current = handleAdd;
  useEffect(() => {
    const add = () => addRef.current();
    const clone = () => setCloning(true);
    window.addEventListener("agency:add-project", add);
    window.addEventListener("agency:clone-project", clone);
    return () => {
      window.removeEventListener("agency:add-project", add);
      window.removeEventListener("agency:clone-project", clone);
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
    ? projects.filter((p) => (projectRuns[p.id] ?? []).length > 0)
    : projects;

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

  async function confirmPending() {
    if (!pending) return;
    const { kind, project } = pending;
    try {
      if (kind === "close") await closeProject(project.id);
      else await deleteProject(project.id);
      await refresh();
    } catch (e) {
      toastError(e, kind === "close" ? "Couldn't close project" : "Couldn't remove project");
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
            ? "Showing active projects only — click to show all"
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
        {filter === "active" && visible.length === 0 && projects.length > 0 && (
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
              <button className="row-act" title="Close project" aria-label="Close project" onClick={(e) => { e.stopPropagation(); setPending({ kind: "close", project: p }); }}>⏻</button>
              <button className="row-act danger" title="Remove project" aria-label="Remove project" onClick={(e) => { e.stopPropagation(); setPending({ kind: "remove", project: p }); }}>×</button>
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
        <button className="tree-settings" aria-label="Settings" onClick={onOpenSettings}>
          <span className="tree-settings-gear">{"⚙︎"}</span> Settings
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
      {pending && (
        <ConfirmDialog
          title={pending.kind === "close" ? "Close project?" : "Remove project?"}
          body={pending.kind === "close"
            ? `Stop all running agents in "${pending.project.name}". Their setup is kept — reopen to re-run them.`
            : `Permanently remove "${pending.project.name}" and delete all its agents and worktrees. This cannot be undone.`}
          confirmLabel={pending.kind === "close" ? "Close project" : "Remove project"}
          danger={pending.kind === "remove"}
          onConfirm={confirmPending}
          onCancel={() => setPending(null)}
        />
      )}
    </aside>
  );
}
