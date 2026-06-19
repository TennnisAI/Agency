import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { addProject, listProjects, removeProject, Project } from "../api";

interface Props {
  selectedId: string | null;
  onSelect: (project: Project) => void;
}

export default function ProjectSidebar({ selectedId, onSelect }: Props) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [name, setName] = useState("");
  const [repoPath, setRepoPath] = useState("");

  async function refresh() {
    setProjects(await listProjects());
  }

  useEffect(() => {
    refresh();
  }, []);

  async function handleAdd() {
    if (!name.trim() || !repoPath.trim()) return;
    await addProject(name.trim(), repoPath.trim());
    setName("");
    setRepoPath("");
    await refresh();
  }

  async function handleRemove(id: string) {
    await removeProject(id);
    await refresh();
  }

  async function handleBrowse() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === "string") {
      setRepoPath(selected);
      // Default the project name to the folder name if not set yet.
      if (!name.trim()) {
        const base = selected.split("/").filter(Boolean).pop();
        if (base) setName(base);
      }
    }
  }

  return (
    <aside className="sidebar">
      <h2>Projects</h2>
      <ul className="project-list">
        {projects.map((p) => (
          <li
            key={p.id}
            className={p.id === selectedId ? "selected" : ""}
            onClick={() => onSelect(p)}
          >
            <span>{p.name}</span>
            <button
              onClick={(e) => {
                e.stopPropagation();
                handleRemove(p.id);
              }}
            >
              ×
            </button>
          </li>
        ))}
      </ul>
      <div className="add-project">
        <input placeholder="name" value={name} onChange={(e) => setName(e.target.value)} />
        <div className="repo-row">
          <input
            placeholder="/path/to/repo"
            value={repoPath}
            onChange={(e) => setRepoPath(e.target.value)}
          />
          <button className="browse" onClick={handleBrowse}>
            Browse…
          </button>
        </div>
        <button onClick={handleAdd}>Add project</button>
      </div>
    </aside>
  );
}
