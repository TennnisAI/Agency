import { useState } from "react";
import ProjectSidebar from "./components/ProjectSidebar";
import TaskBoard from "./components/TaskBoard";
import Settings from "./components/Settings";
import { Project } from "./api";

export default function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  return (
    <div className="app">
      <div className="sidebar-wrap">
        <ProjectSidebar selectedId={project?.id ?? null} onSelect={setProject} />
        <button className="settings-btn" onClick={() => setShowSettings(true)}>
          Settings
        </button>
      </div>
      {project ? (
        <TaskBoard project={project} />
      ) : (
        <main className="board empty">Select or add a project to begin.</main>
      )}
      {showSettings && <Settings onClose={() => setShowSettings(false)} />}
    </div>
  );
}
