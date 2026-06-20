import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Project, RunInfo, addProject, listProjects, listRuns, removeProject } from "../api";

function statusClass(s: RunInfo["status"]): string {
  return s.state === "running" ? "running" : "exited";
}

export default function ProjectTree({
  selectedId,
  onSelect,
}: {
  selectedId: string | null;
  onSelect: (p: Project) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [expanded, setExpanded] = useState<Record<string, RunInfo[] | undefined>>({});
  const [name, setName] = useState("");
  const [repoPath, setRepoPath] = useState("");

  async function refresh() {
    setProjects(await listProjects());
  }
  useEffect(() => {
    refresh();
  }, []);

  async function toggle(p: Project) {
    setExpanded((e) => ({ ...e, [p.id]: e[p.id] ? undefined : [] }));
    if (!expanded[p.id]) {
      try {
        const runs = await listRuns(p.id);
        setExpanded((e) => ({ ...e, [p.id]: runs }));
      } catch {
        /* ignore */
      }
    }
  }

  async function handleBrowse() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel === "string") {
      setRepoPath(sel);
      if (!name.trim()) setName(sel.split("/").filter(Boolean).pop() ?? "");
    }
  }
  async function handleAdd() {
    if (!name.trim() || !repoPath.trim()) return;
    await addProject(name.trim(), repoPath.trim());
    setName("");
    setRepoPath("");
    await refresh();
  }

  return (
    <aside className="tree">
      <h2 className="tree-head">Projects</h2>
      <ul className="tree-list">
        {projects.map((p) => (
          <li key={p.id}>
            <div className={`tree-row ${p.id === selectedId ? "selected" : ""}`} onClick={() => onSelect(p)}>
              <span className="chev" onClick={(e) => { e.stopPropagation(); toggle(p); }}>
                {expanded[p.id] !== undefined ? "▾" : "▸"}
              </span>
              <span className="tree-name">{p.name}</span>
              <button className="ghost-x" onClick={(e) => { e.stopPropagation(); removeProject(p.id).then(refresh); }}>×</button>
            </div>
            {expanded[p.id] !== undefined && (
              <ul className="tree-children">
                {(expanded[p.id] ?? []).map((r) => (
                  <li key={r.id} className="tree-child">
                    <span className={`dot ${statusClass(r.status)}`} />
                    <span className="tree-child-name">{r.agent}: {r.prompt || r.branch}</span>
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
      <div className="add-project">
        <input placeholder="name" value={name} onChange={(e) => setName(e.target.value)} />
        <div className="repo-row">
          <input placeholder="/path/to/repo" value={repoPath} onChange={(e) => setRepoPath(e.target.value)} />
          <button className="browse" onClick={handleBrowse}>Browse…</button>
        </div>
        <button onClick={handleAdd}>Add project</button>
      </div>
    </aside>
  );
}
