import { useEffect, useState } from "react";
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
        <input
          placeholder="/path/to/repo"
          value={repoPath}
          onChange={(e) => setRepoPath(e.target.value)}
        />
        <button onClick={handleAdd}>Add project</button>
      </div>
    </aside>
  );
}
