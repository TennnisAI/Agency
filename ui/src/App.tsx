import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { RunStoreProvider, useRuns } from "./store/runs";
import TitleBar from "./components/TitleBar";
import StatusBar from "./components/StatusBar";
import ProjectTree from "./components/ProjectTree";
import AgentsView from "./components/AgentsView";
import Settings from "./components/Settings";
import AgentOnboarding from "./components/AgentOnboarding";
import CommandPalette from "./components/CommandPalette";
import ConfirmDialog from "./components/ConfirmDialog";
import Resizer from "./components/Resizer";
import Toasts from "./components/Toasts";
import { useShortcuts } from "./hooks/useShortcuts";
import { usePaneWidth } from "./hooks/usePaneWidth";
import { FileRoot, Project, RunInfo, agentOnboardingNeeded, archiveRun, checkForUpdate, confirmQuit, createDir, createFile, discardRun, getUpdateCheckEnabled, getWorkspace, inspectRepo, listProjects, readFile, setMenuContext, setUiState, writeFile } from "./api";
import { pickDefaultAgent } from "./lib/defaultAgent";
import { DAILY_TEMPLATE_PATH, JOURNAL_DIR, dailyNotePath, defaultDailyContent, renderDailyTemplate } from "./lib/dailyNote";
import { toastError, toastInfo } from "./lib/toast";
import { workspaceHidden } from "./lib/workspacePref";

const REPO_URL = "https://github.com/nic123/Agency";

function Shell() {
  const { selectedProjectId, setSelectedProject, createAgent, createTerminal, refreshRuns, setTab, focusedRunId, setApproveRun, setFocusedRun, setView, runs } = useRuns();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [project, setProject] = useState<Project | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showSettings, setShowSettings] = useState(false);
  // A newer release exists on GitHub. Dots the Settings button; the actual
  // download link lives in Settings ▸ Diagnostics.
  const [updateAvailable, setUpdateAvailable] = useState(false);
  // Menu-driven archive/discard of the focused agent, gated behind a confirm
  // dialog (matching the tile-level action). null = no confirmation showing.
  const [agentAction, setAgentAction] = useState<{ kind: "archive" | "discard"; run: RunInfo } | null>(null);
  // Sessions the quit would stop (from the backend's quit-requested event);
  // null = no quit confirmation showing.
  const [quitPrompt, setQuitPrompt] = useState<number | null>(null);
  const sidebar = usePaneWidth("sidebar", 266, 200, 460);

  // Picks the agent a menu/shortcut "New Agent" spawns — the shared pick
  // order in lib/defaultAgent (Settings default → project's last-used → claude).
  async function newTaskDefaultAgent() {
    if (!selectedProjectId) return;
    // A git-less workspace has no worktrees to spawn into; the in-view spawn
    // affordances are hidden, so catch the menu/shortcut path with a hint.
    if (project?.kind === "workspace") {
      const r = await inspectRepo(project.repo_path).catch(() => null);
      if (r?.state === "notARepo") {
        toastInfo("Agents need git — initialize a repository in the workspace first.");
        return;
      }
    }
    createAgent(await pickDefaultAgent(selectedProjectId, project?.default_agent));
  }

  // ⌘⇧D / File ▸ Today's Note / palette: open today's journal note in the
  // workspace, creating the note (from templates/daily.md when present) — and,
  // on very first use, the workspace itself via ProjectTree's create dialog,
  // which resumes this flow through the agency:workspace-ready event.
  async function openDailyNote() {
    // Hidden means "I don't use this" — respect it rather than resurrecting
    // the workspace from a stray shortcut press.
    if (workspaceHidden()) {
      toastInfo("The workspace is hidden — turn it back on in Settings ▸ Workspace.");
      return;
    }
    const ws = await getWorkspace().catch(() => null);
    if (!ws) {
      window.dispatchEvent(new CustomEvent("agency:create-workspace", { detail: { intent: "daily-note" } }));
      return;
    }
    const root: FileRoot = { kind: "project", id: ws.id };
    const now = new Date();
    const path = dailyNotePath(now);
    try {
      const existing = await readFile(root, path).catch(() => null);
      if (!existing) {
        let content = defaultDailyContent(now);
        const tpl = await readFile(root, DAILY_TEMPLATE_PATH).catch(() => null);
        if (tpl && !tpl.binary && !tpl.tooLarge && tpl.text.trim()) {
          content = renderDailyTemplate(tpl.text, now);
        }
        await createDir(root, JOURNAL_DIR).catch(() => { /* already exists */ });
        await createFile(root, path);
        await writeFile(root, path, content);
      }
    } catch (e) {
      toastError(e, "Couldn't open today's note");
      return;
    }
    // Stamp the last-open note so a freshly mounted DocsView restores straight
    // to it; the event covers the already-mounted case.
    try { localStorage.setItem(`docs:last:${ws.id}`, path); } catch { /* storage unavailable */ }
    selectProject(ws);
    setTab("docs");
    window.dispatchEvent(new CustomEvent("agency:open-note", { detail: { projectId: ws.id, path } }));
  }
  const dailyRef = useRef(openDailyNote);
  dailyRef.current = openDailyNote;

  useShortcuts({
    onNewTask: () => { newTaskDefaultAgent(); },
    onSource: () => setTab("source"),
    onDailyNote: () => { openDailyNote(); },
    onApprove: () => {
      // Approve/merge is an agent-only workflow; terminals have no branch to merge.
      const focused = runs.find((r) => r.id === focusedRunId);
      if (focused?.kind === "agent") setApproveRun(focused.id);
    },
    onPalette: () => setPaletteOpen(true),
    onSettings: () => setShowSettings(true),
  });

  // One passive release check per launch, when the user hasn't opted out. It
  // only lights the dot on Settings — Agency never downloads or installs
  // anything on its own, so this can't disturb running agents. Failures are
  // silent by design: being offline is not something to interrupt anyone about.
  useEffect(() => {
    let cancelled = false;
    getUpdateCheckEnabled()
      .then((enabled) => (enabled ? checkForUpdate() : null))
      .then((res) => {
        if (!cancelled && res?.updateAvailable) setUpdateAvailable(true);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

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

  // Keep the native menu's context items (New Agent/Terminal, Source, and the
  // Agent menu) enabled only when they'd actually do something, so they aren't
  // clickable no-ops. The Agent menu tracks a focused *agent* — a focused
  // terminal doesn't count (it has no branch to approve/merge). Depending on
  // the derived booleans (not `runs`, whose identity changes every poll) keeps
  // this to one IPC call per actual state change.
  const hasProject = !!selectedProjectId;
  const hasFocusedAgent = runs.some((r) => r.id === focusedRunId && r.kind === "agent");
  useEffect(() => {
    setMenuContext(hasProject, hasFocusedAgent).catch(() => {});
  }, [hasProject, hasFocusedAgent]);

  function selectProject(p: Project) {
    setShowSettings(false);
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
    setShowSettings(false);
    setProject(p);
    setSelectedProject(p.id);
    setFocusedRun(runId);
    setView("focus");
  }

  // Route a native-menu action (payload of the backend "menu" event) to the
  // same handlers the buttons/shortcuts use, so the menu bar never drifts from
  // the UI. Kept in a ref (below) so the event listener, bound once, always
  // calls the version closed over the latest state.
  function onMenu(action: string) {
    switch (action) {
      case "settings": setShowSettings(true); break;
      case "palette": setPaletteOpen(true); break;
      case "new-agent": newTaskDefaultAgent(); break;
      case "new-terminal": createTerminal(); break;
      // The full add flow (dir picker + repo-setup dialog) lives in ProjectTree;
      // signal it rather than duplicating that logic here.
      case "add-project": window.dispatchEvent(new CustomEvent("agency:add-project")); break;
      case "clone-project": window.dispatchEvent(new CustomEvent("agency:clone-project")); break;
      case "source": setTab("source"); break;
      case "daily-note": openDailyNote(); break;
      case "toggle-sidebar": setSidebarOpen((s) => !s); break;
      case "home": goHome(); break;
      case "approve": {
        // Approve/merge is agent-only — terminals have no branch to merge.
        const focused = runs.find((r) => r.id === focusedRunId);
        if (focused?.kind === "agent") setApproveRun(focused.id);
        break;
      }
      case "archive": {
        const focused = runs.find((r) => r.id === focusedRunId);
        if (focused?.kind === "agent") setAgentAction({ kind: "archive", run: focused });
        break;
      }
      case "discard": {
        const focused = runs.find((r) => r.id === focusedRunId);
        if (focused) setAgentAction({ kind: "discard", run: focused });
        break;
      }
      case "report-issue": openUrl(`${REPO_URL}/issues/new`).catch(() => {}); break;
      case "github": openUrl(REPO_URL).catch(() => {}); break;
    }
  }
  const menuRef = useRef(onMenu);
  menuRef.current = onMenu;

  // Backend-driven navigation: quit confirmations, tray-menu and app-menu
  // clicks arrive as Tauri events.
  useEffect(() => {
    const subs = [
      listen<number>("quit-requested", (e) => setQuitPrompt(e.payload)),
      listen<string>("menu", (e) => menuRef.current(e.payload)),
      listen<{ projectId: string; runId: string }>("tray-open-run", async (e) => {
        const p = (await listProjects().catch(() => [])).find((x) => x.id === e.payload.projectId);
        if (p) openRun(p, e.payload.runId);
      }),
      listen<string>("tray-open-project", async (e) => {
        const p = (await listProjects().catch(() => [])).find((x) => x.id === e.payload);
        if (p) selectProject(p);
      }),
    ];
    // Palette "Today's Note" and the post-creation resume of that flow arrive
    // as DOM events (the palette can't call into Shell directly).
    const daily = () => { void dailyRef.current(); };
    const ready = (e: Event) => {
      if ((e as CustomEvent<{ intent?: string }>).detail?.intent === "daily-note") {
        void dailyRef.current();
      }
    };
    window.addEventListener("agency:daily-note", daily);
    window.addEventListener("agency:workspace-ready", ready);
    // The .catch matters: Tauri's injected event plugin throws
    // ("listeners[eventId].handlerId") when an unlisten races a listener that
    // was already removed (e.g. across remounts). A failed cleanup of a dead
    // listener is a no-op — don't let it surface as an error toast.
    return () => {
      subs.forEach((s) => s.then((un) => un()).catch(() => {}));
      window.removeEventListener("agency:daily-note", daily);
      window.removeEventListener("agency:workspace-ready", ready);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Run the confirmed archive/discard, then clear focus (if it was this run)
  // and refresh so the tile disappears.
  async function runAgentAction() {
    if (!agentAction) return;
    const { kind, run } = agentAction;
    try {
      if (kind === "archive") await archiveRun(run.id);
      else await discardRun(run.id);
      if (focusedRunId === run.id) { setFocusedRun(null); setView("grid"); }
      await refreshRuns();
    } catch {
      /* the backend surfaces failures; keep the dialog dismissal simple */
    }
    setAgentAction(null);
  }

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
              updateAvailable={updateAvailable}
            />
          </div>
        </div>
        {sidebarOpen && (
          <Resizer size={sidebar.width} min={200} max={460} onChange={sidebar.setWidth} side="left" />
        )}
        {showSettings ? (
          <Settings
            onClose={() => setShowSettings(false)}
            onOpenTerminal={(runId) => project && openRun(project, runId)}
            projectId={selectedProjectId}
            projectName={project?.name ?? null}
          />
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
      {agentAction && (
        <ConfirmDialog
          title={agentAction.kind === "archive" ? "Archive agent?" : "Discard agent?"}
          body={agentAction.kind === "archive"
            ? `Move "${agentAction.run.agent}" to the archived list. You can restore it later.`
            : `Stop "${agentAction.run.agent}", remove its worktree, and delete the run. This cannot be undone.`}
          confirmLabel={agentAction.kind === "archive" ? "Archive" : "Discard"}
          danger={agentAction.kind === "discard"}
          onConfirm={() => { runAgentAction(); }}
          onCancel={() => setAgentAction(null)}
        />
      )}
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
  // null = still checking; true = show picker; false = main shell.
  const [needsOnboarding, setNeedsOnboarding] = useState<boolean | null>(null);

  useEffect(() => {
    agentOnboardingNeeded()
      .then(setNeedsOnboarding)
      .catch(() => setNeedsOnboarding(false));
  }, []);

  if (needsOnboarding === null) {
    // Brief check against the backend — show the chrome with a spinner rather
    // than a blank window so launch doesn't flash empty.
    return (
      <div className="shell">
        <TitleBar onOpenPalette={() => {}} />
        <div className="app-loading">
          <span className="spinner" />
        </div>
      </div>
    );
  }
  if (needsOnboarding) {
    return (
      <div className="shell">
        <TitleBar onOpenPalette={() => {}} />
        <AgentOnboarding onDone={() => setNeedsOnboarding(false)} />
        <Toasts />
      </div>
    );
  }

  return (
    <RunStoreProvider>
      <Shell />
    </RunStoreProvider>
  );
}
