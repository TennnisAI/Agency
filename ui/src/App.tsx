import { useState } from "react";
import { RunStoreProvider, useRuns } from "./store/runs";
import TitleBar from "./components/TitleBar";
import StatusBar from "./components/StatusBar";
import ProjectTree from "./components/ProjectTree";
import AgentsView from "./components/AgentsView";
import Settings from "./components/Settings";
import { Project } from "./api";

function Shell() {
  const { selectedProjectId, setSelectedProject } = useRuns();
  const [project, setProject] = useState<Project | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showSettings, setShowSettings] = useState(false);

  function selectProject(p: Project) {
    setProject(p);
    setSelectedProject(p.id);
  }

  return (
    <div className="shell">
      <TitleBar onToggleSidebar={() => setSidebarOpen((s) => !s)} onOpenSettings={() => setShowSettings(true)} />
      <div className="body">
        {sidebarOpen && <ProjectTree selectedId={selectedProjectId} onSelect={selectProject} />}
        {project ? <AgentsView project={project} /> : <main className="board empty">Select or add a project to begin.</main>}
      </div>
      <StatusBar projectName={project?.name ?? null} />
      {showSettings && <Settings onClose={() => setShowSettings(false)} />}
    </div>
  );
}

export default function App() {
  return (
    <RunStoreProvider>
      <Shell />
    </RunStoreProvider>
  );
}
