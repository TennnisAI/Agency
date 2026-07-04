import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { RunStoreProvider, useRuns } from "./store/runs";
import TitleBar from "./components/TitleBar";
import StatusBar from "./components/StatusBar";
import ProjectTree from "./components/ProjectTree";
import AgentsView from "./components/AgentsView";
import Settings from "./components/Settings";
import CommandPalette from "./components/CommandPalette";
import ConfirmDialog from "./components/ConfirmDialog";
import Resizer from "./components/Resizer";
import Toasts from "./components/Toasts";
import { useShortcuts } from "./hooks/useShortcuts";
import { usePaneWidth } from "./hooks/usePaneWidth";
import { Project, RunInfo, confirmQuit, listProjects, setUiState } from "./api";

function Shell() {
  const { selectedProjectId, setSelectedProject, createAgent, setTab, focusedRunId, setApproveRun, setFocusedRun, setView, runs } = useRuns();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [project, setProject] = useState<Project | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showSettings, setShowSettings] = useState(false);
  // Sessions the quit would stop (from the backend's quit-requested event);
  // null = no quit confirmation showing.
  const [quitPrompt, setQuitPrompt] = useState<number | null>(null);
  const sidebar = usePaneWidth("sidebar", 266, 200, 460);

  // New task defaults to the agent this project last used (default_agent is
  // updated on every run creation), refetched so it isn't stale from the
  // Project captured at selection time. Falls back to claude.
  async function newTaskDefaultAgent() {
    if (!selectedProjectId) return;
    const projects = await listProjects().catch(() => null);
    const current = projects?.find((p) => p.id === selectedProjectId);
    createAgent(current?.default_agent ?? project?.default_agent ?? "claude");
  }

  useShortcuts({
    onNewTask: () => { newTaskDefaultAgent(); },
    onSource: () => setTab("source"),
    onApprove: () => {
      // Approve/merge is an agent-only workflow; terminals have no branch to merge.
      const focused = runs.find((r) => r.id === focusedRunId);
      if (focused?.kind === "agent") setApproveRun(focused.id);
    },
    onPalette: () => setPaletteOpen(true),
    onSettings: () => setShowSettings(true),
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

  // Back to the all-projects overview (the no-project state).
  function goHome() {
    setProject(null);
    setSelectedProject(null);
    setShowSettings(false);
  }

  // Open a specific agent straight into its focus view. setSelectedProject
  // resets view/focus, so the focus + view calls must follow it; React batches
  // them in this handler, leaving the run focused.
  function openRun(p: Project, runId: string) {
    setProject(p);
    setSelectedProject(p.id);
    setFocusedRun(runId);
    setView("focus");
  }

  // Backend-driven navigation: quit confirmations and tray-menu clicks arrive
  // as Tauri events.
  useEffect(() => {
    const subs = [
      listen<number>("quit-requested", (e) => setQuitPrompt(e.payload)),
      listen<{ projectId: string; runId: string }>("tray-open-run", async (e) => {
        const p = (await listProjects().catch(() => [])).find((x) => x.id === e.payload.projectId);
        if (p) openRun(p, e.payload.runId);
      }),
      listen<string>("tray-open-project", async (e) => {
        const p = (await listProjects().catch(() => [])).find((x) => x.id === e.payload);
        if (p) selectProject(p);
      }),
    ];
    return () => { subs.forEach((s) => s.then((un) => un())); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="shell">
      <TitleBar onOpenPalette={() => setPaletteOpen(true)} />
      <div className="body">
        {/* The sidebar stays mounted while hidden (collapsed to width 0) so
            re-expanding is instant: remounting the tree used to leave the pane
            empty while projects/readiness refetched over IPC. */}
        <div
          className={`sidebar-wrap${sidebarOpen ? "" : " closed"}`}
          style={{ width: sidebarOpen ? sidebar.width : 0 }}
        >
          <div className="sidebar-fix" style={{ width: sidebar.width }}>
            <ProjectTree
              selectedId={selectedProjectId}
              focusedRunId={focusedRunId}
              onSelect={selectProject}
              onSelectRun={(p: Project, run: RunInfo) => openRun(p, run.id)}
              onHome={goHome}
              onToggleSidebar={() => setSidebarOpen(false)}
              onOpenSettings={() => setShowSettings(true)}
            />
          </div>
        </div>
        {sidebarOpen && (
          <Resizer size={sidebar.width} min={200} max={460} onChange={sidebar.setWidth} side="left" />
        )}
        {showSettings ? (
          <Settings onClose={() => setShowSettings(false)} />
        ) : (
          <AgentsView
            project={project}
            sidebarOpen={sidebarOpen}
            onToggleSidebar={() => setSidebarOpen(true)}
            onOpenRun={openRun}
            onOpenProject={selectProject}
          />
        )}
      </div>
      <StatusBar projectName={project?.name ?? null} focusedRunId={focusedRunId} />
      <Toasts />
      {paletteOpen && <CommandPalette onClose={() => setPaletteOpen(false)} />}
      {quitPrompt !== null && (
        <ConfirmDialog
          title="Quit Agency?"
          body={quitPrompt > 0
            ? `Quitting will stop ${quitPrompt} running session${quitPrompt === 1 ? "" : "s"}. Agents resume where they left off next time you open Agency.`
            : "All agents are idle — nothing will be interrupted."}
          confirmLabel="Quit"
          danger={quitPrompt > 0}
          onConfirm={() => { confirmQuit().catch(() => {}); }}
          onCancel={() => setQuitPrompt(null)}
        />
      )}
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
