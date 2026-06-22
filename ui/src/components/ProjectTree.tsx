import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Project, RunInfo, addProject, closeProject, deleteProject, listProjects, listRuns } from "../api";
import ConfirmDialog from "./ConfirmDialog";

function statusClass(s: RunInfo["status"]): string {
  return s.state === "running" ? "running" : "exited";
}

type Pending =
  | { kind: "close" | "remove"; project: Project }
  | null;

export default function ProjectTree({
  selectedId,
  onSelect,
}: {
  selectedId: string | null;
  onSelect: (p: Project) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [expanded, setExpanded] = useState<Record<string, RunInfo[] | undefined>>({});
  const [pending, setPending] = useState<Pending>(null);
  const [error, setError] = useState("");

  async function refresh() {
    setProjects(await listProjects());
  }
  useEffect(() => { refresh(); }, []);

  async function toggle(p: Project) {
    setExpanded((e) => ({ ...e, [p.id]: e[p.id] ? undefined : [] }));
    if (!expanded[p.id]) {
      try {
        const runs = await listRuns(p.id);
        setExpanded((e) => ({ ...e, [p.id]: runs }));
      } catch { /* ignore */ }
    }
  }

  async function handleAdd() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    const name = sel.split("/").filter(Boolean).pop() ?? sel;
    try {
      await addProject(name, sel);
      setError("");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
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
            <div className={`tree-row ${p.id === selectedId ? "selected" : ""}`} onClick={() => onSelect(p)}>
              <span className="chev" onClick={(e) => { e.stopPropagation(); toggle(p); }}>
                {expanded[p.id] !== undefined ? "▾" : "▸"}
              </span>
              <span className="proj-icon" aria-hidden>{p.name.slice(0, 1).toUpperCase()}</span>
              <span className="tree-name tl">{p.name}</span>
              <button className="row-act" title="Close project" onClick={(e) => { e.stopPropagation(); setPending({ kind: "close", project: p }); }}>⏻</button>
              <button className="row-act danger" title="Remove project" onClick={(e) => { e.stopPropagation(); setPending({ kind: "remove", project: p }); }}>×</button>
            </div>
            {expanded[p.id] !== undefined && (
              <ul className="tree-children">
                {(expanded[p.id] ?? []).map((r) => (
                  <li key={r.id} className="tree-child">
                    <span className={`dot ${statusClass(r.status)}`} />
                    <span className="tree-child-name tl">{r.agent}: {r.prompt || r.branch}</span>
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
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
