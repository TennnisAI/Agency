import { useState } from "react";
import { RunStoreProvider, useRuns } from "./store/runs";
import TitleBar from "./components/TitleBar";
import StatusBar from "./components/StatusBar";
import ProjectTree from "./components/ProjectTree";
import AgentsView from "./components/AgentsView";
import Settings from "./components/Settings";
import CommandPalette from "./components/CommandPalette";
import { useShortcuts } from "./hooks/useShortcuts";
import { Project } from "./api";

function Shell() {
  const { selectedProjectId, setSelectedProject, createAgent, setTab, focusedRunId, setApproveRun } = useRuns();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [project, setProject] = useState<Project | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showSettings, setShowSettings] = useState(false);

  useShortcuts({
    onNewTask: () => createAgent("claude"),
    onSource: () => setTab("source"),
    onApprove: () => { if (focusedRunId) setApproveRun(focusedRunId); },
    onPalette: () => setPaletteOpen(true),
  });

  function selectProject(p: Project) {
    setProject(p);
    setSelectedProject(p.id);
  }

  return (
    <div className="shell">
      <TitleBar onToggleSidebar={() => setSidebarOpen((s) => !s)} onOpenSettings={() => setShowSettings(true)} onOpenPalette={() => setPaletteOpen(true)} />
      <div className="body">
        {sidebarOpen && <ProjectTree selectedId={selectedProjectId} onSelect={selectProject} />}
        {project ? <AgentsView project={project} /> : <main className="board empty">Select or add a project to begin.</main>}
      </div>
      <StatusBar projectName={project?.name ?? null} />
      {showSettings && <Settings onClose={() => setShowSettings(false)} />}
      {paletteOpen && <CommandPalette onClose={() => setPaletteOpen(false)} />}
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
