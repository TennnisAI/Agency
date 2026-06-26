import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Project, RunInfo, RepoReadiness, addProject, closeProject, deleteProject, inspectRepo, listProjects, listRuns } from "../api";
import { projectColor, runName } from "../agents";
import { useRuns } from "../store/runs";
import ConfirmDialog from "./ConfirmDialog";
import RepoSetupDialog from "./RepoSetupDialog";

function statusClass(s: RunInfo["status"]): string {
  return s.state === "running" ? "running" : "exited";
}

type Pending =
  | { kind: "close" | "remove"; project: Project }
  | null;

export default function ProjectTree({
  selectedId,
  focusedRunId,
  onSelect,
  onSelectRun,
}: {
  selectedId: string | null;
  focusedRunId: string | null;
  onSelect: (p: Project) => void;
  onSelectRun: (p: Project, run: RunInfo) => void;
}) {
  const { runs } = useRuns();
  const [projects, setProjects] = useState<Project[]>([]);
  const [openIds, setOpenIds] = useState<Set<string>>(() => new Set());
  const [childRuns, setChildRuns] = useState<Record<string, RunInfo[]>>({});
  const [pending, setPending] = useState<Pending>(null);
  const [error, setError] = useState("");
  const [setup, setSetup] = useState<{ path: string; name: string; readiness: RepoReadiness; existing: boolean } | null>(null);
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

  // Keep the agents shown under each expanded project in sync with the shared
  // run store. Re-fetching whenever a project is opened or the global `runs`
  // change (discard, spawn, or the store's poll) means the tree can't keep
  // showing an agent the Agents rail has already dropped.
  useEffect(() => {
    let cancelled = false;
    const ids = [...openIds];
    if (ids.length === 0) { setChildRuns({}); return; }
    (async () => {
      const lists = await Promise.all(ids.map((id) => listRuns(id).catch(() => [])));
      if (cancelled) return;
      setChildRuns(Object.fromEntries(ids.map((id, i) => [id, lists[i]])));
    })();
    return () => { cancelled = true; };
  }, [openIds, runs]);

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
        await addProject(name, sel);
        await refresh();
      } else {
        setSetup({ path: sel, name, readiness: r, existing: false });
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function finishSetup() {
    if (!setup) return;
    try {
      // Only create the record for the add flow; the badge flow's project already exists.
      if (!setup.existing) await addProject(setup.name, setup.path);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
    setSetup(null);
  }

  async function confirmPending() {
    if (!pending) return;
    const { kind, project } = pending;
    if (kind === "close") await closeProject(project.id);
    else await deleteProject(project.id);
    setPending(null);
    await refresh();
  }

  return (
    <aside className="tree">
      <div className="tree-head">
        <span className="eyebrow">PROJECTS</span>
        <button className="icon-add" title="Add project" onClick={handleAdd}>+</button>
      </div>
      {error && <div className="git-error">{error}</div>}
      <ul className="tree-list">
        {projects.map((p) => (
          <li key={p.id}>
            <div
              className={`tree-row ${p.id === selectedId ? (focusedRunId ? "selected ancestor" : "selected") : ""}`}
              onClick={() => onSelect(p)}
            >
              <span className="chev" onClick={(e) => { e.stopPropagation(); toggle(p); }}>
                {openIds.has(p.id) ? "▾" : "▸"}
              </span>
              <span className="proj-icon" aria-hidden style={{ background: projectColor(p.id) }}>{p.name.slice(0, 1).toUpperCase()}</span>
              <span className="tree-name tl">{p.name}</span>
              {readiness[p.id]?.state === "noCommits" && (
                <button
                  className="row-badge warn"
                  title="Needs a commit before agents can run"
                  onClick={(e) => { e.stopPropagation(); setSetup({ path: p.repo_path, name: p.name, readiness: readiness[p.id]!, existing: true }); }}
                >⚠ commit</button>
              )}
              <button className="row-act" title="Close project" onClick={(e) => { e.stopPropagation(); setPending({ kind: "close", project: p }); }}>⏻</button>
              <button className="row-act danger" title="Remove project" onClick={(e) => { e.stopPropagation(); setPending({ kind: "remove", project: p }); }}>×</button>
            </div>
            {openIds.has(p.id) && (
              <ul className="tree-children">
                {(childRuns[p.id] ?? []).map((r) => (
                  <li
                    key={r.id}
                    className={`tree-child ${r.id === focusedRunId ? "active" : ""}`}
                    title={`Open ${r.agent}: ${runName(r)}`}
                    onClick={(e) => { e.stopPropagation(); onSelectRun(p, r); }}
                  >
                    <span className={`dot ${statusClass(r.status)}`} />
                    <span className="tree-child-name tl">{r.agent}: {runName(r)}</span>
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
      {setup && (
        <RepoSetupDialog
          readiness={setup.readiness}
          context="add"
          repoPath={setup.path}
          onResolved={finishSetup}
          onCancel={() => setSetup(null)}
        />
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
