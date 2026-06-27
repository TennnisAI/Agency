import { useEffect, useState } from "react";
import { RunStoreProvider, useRuns } from "./store/runs";
import TitleBar from "./components/TitleBar";
import StatusBar from "./components/StatusBar";
import ProjectTree from "./components/ProjectTree";
import AgentsView from "./components/AgentsView";
import Settings from "./components/Settings";
import CommandPalette from "./components/CommandPalette";
import Resizer from "./components/Resizer";
import { useShortcuts } from "./hooks/useShortcuts";
import { usePaneWidth } from "./hooks/usePaneWidth";
import { Project, RunInfo, setUiState } from "./api";

function Shell() {
  const { selectedProjectId, setSelectedProject, createAgent, setTab, focusedRunId, setApproveRun, setFocusedRun, setView, runs } = useRuns();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [project, setProject] = useState<Project | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showSettings, setShowSettings] = useState(false);
  const sidebar = usePaneWidth("sidebar", 266, 200, 460);

  useShortcuts({
    onNewTask: () => createAgent("claude"),
    onSource: () => setTab("source"),
    onApprove: () => {
      // Approve/merge is an agent-only workflow; terminals have no branch to merge.
      const focused = runs.find((r) => r.id === focusedRunId);
      if (focused?.kind === "agent") setApproveRun(focused.id);
    },
    onPalette: () => setPaletteOpen(true),
  });

  useEffect(() => {
    const report = () => setUiState(document.hasFocus(), focusedRunId).catch(() => {});
    report();
    window.addEventListener("focus", report);
    window.addEventListener("blur", report);
    return () => {
      window.removeEventListener("focus", report);
      window.removeEventListener("blur", report);
    };
  }, [focusedRunId]);

  function selectProject(p: Project) {
    setProject(p);
    setSelectedProject(p.id);
  }

  // Open a specific agent straight into its focus view. setSelectedProject
  // resets view/focus, so the focus + view calls must follow it; React batches
  // them in this handler, leaving the run focused.
  function selectRun(p: Project, run: RunInfo) {
    setProject(p);
    setSelectedProject(p.id);
    setFocusedRun(run.id);
    setView("focus");
  }

  return (
    <div className="shell">
      <TitleBar sidebarOpen={sidebarOpen} onToggleSidebar={() => setSidebarOpen((s) => !s)} onOpenSettings={() => setShowSettings(true)} onOpenPalette={() => setPaletteOpen(true)} />
      <div className="body">
        {sidebarOpen && (
          <>
            <div style={{ width: sidebar.width, flexShrink: 0, display: "flex", minHeight: 0 }}>
              <ProjectTree selectedId={selectedProjectId} focusedRunId={focusedRunId} onSelect={selectProject} onSelectRun={selectRun} />
            </div>
            <Resizer size={sidebar.width} min={200} max={460} onChange={sidebar.setWidth} side="left" />
          </>
        )}
        {project ? <AgentsView project={project} /> : <main className="board empty">Select or add a project to begin.</main>}
      </div>
      <StatusBar projectName={project?.name ?? null} focusedRunId={focusedRunId} />
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
