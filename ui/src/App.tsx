import { useState } from "react";
import ProjectSidebar from "./components/ProjectSidebar";
import TaskBoard from "./components/TaskBoard";
import { Project } from "./api";

export default function App() {
  const [project, setProject] = useState<Project | null>(null);
  return (
    <div className="app">
      <ProjectSidebar selectedId={project?.id ?? null} onSelect={setProject} />
      {project ? (
        <TaskBoard project={project} />
      ) : (
        <main className="board empty">Select or add a project to begin.</main>
      )}
    </div>
  );
}
