import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { CloneProgress, RunInfo, createRun, createTerminal as createTerminalApi, getSettings, listRuns, projectTarget, rememberedModel, runScriptsLive } from "../api";
import { loadFold, saveFold } from "../hooks/usePaneWidth";
import { pinnedFirst } from "../lib/runstate";
import { toastError } from "../lib/toast";

type View = "grid" | "focus";
// What the add-menu hands to a spawn. `worktree: false` means "run in the
// project checkout on its current branch", which makes base/mergeTarget moot.
// Omitting it (the menu-bar shortcut, which has no picker) defers to the
// Settings default.
export type SpawnOpts = {
  base?: string;
  mergeTarget?: string;
  worktree?: boolean;
  // Model to launch on. `null` is the agent's own default, which is a real
  // choice; `undefined` means the caller has no picker, so the model the agent
  // last ran on is used (the same shape `worktree` uses for its Settings
  // default).
  model?: string | null;
};
type Tab = "agents" | "source" | "files" | "issues" | "docs" | "run";

interface RunStore {
  runs: RunInfo[];
  // A run script is running in the selected project's own checkout (each agent
  // carries its own flag on `RunInfo.runScriptsLive`). Refreshed on the same
  // tick as the runs, so the project's Run tab can show a dot too.
  projectRunLive: boolean;
  selectedProjectId: string | null;
  setSelectedProject: (id: string | null) => void;
  view: View;
  setView: (v: View) => void;
  focusedRunId: string | null;
  setFocusedRun: (id: string | null) => void;
  // The run whose working tree the app is actually in: source control, the
  // status bar's branch and the Files tab follow this, not `focusedRunId`.
  // The grid selects nothing — every tile is an equal there — so it goes null
  // and those views fall back to the project's own checkout, which is where
  // merged work sits waiting to be pushed.
  selectedRunId: string | null;
  // Whether the Agents tab shows the source-control panel on the right,
  // remembered across launches. It lives here rather than in AgentsView because
  // finishing with an agent (approve ▸ archive/delete) opens it: landing back in
  // the grid with the checkout's unpushed commits in sight is what keeps a
  // merge from being forgotten.
  sourcePanelOpen: boolean;
  setSourcePanelOpen: (open: boolean) => void;
  // The run whose agent pane is mounted on screen right now — the one run
  // notifications stay quiet about. Deliberately not `focusedRunId`: a run
  // stays focused while you're in the grid, on the Issues tab, or in another
  // app, and a turn ending out of sight is exactly the one worth a toast.
  // Set by FocusTerminal itself, so it tracks what is rendered, not what is
  // selected.
  onScreenRunId: string | null;
  setOnScreenRun: React.Dispatch<React.SetStateAction<string | null>>;
  refreshRuns: () => Promise<void>;
  tab: Tab;
  setTab: (t: Tab) => void;
  createAgent: (agentId: string, opts?: SpawnOpts) => Promise<void>;
  createTerminal: () => Promise<void>;
  // True while a workspace is being created (worktree + spawn — the slowest
  // first-session op). Drives the add-menu disable + placeholder tile.
  spawning: boolean;
  // Latest workspace-setup progress for the placeholder tile (null before the
  // first update or between spawns). Streamed from `create_run`.
  spawnProgress: CloneProgress | null;
  approveRunId: string | null;
  setApproveRun: (id: string | null) => void;
  // An extra agent tab to open once its run is focused (a PR review that had to
  // share an existing run's worktree). AgentFocus consumes and clears it, so a
  // later visit to the same run lands on the primary agent as usual.
  pendingSessionId: string | null;
  setPendingSession: (id: string | null) => void;
}

const Ctx = createContext<RunStore | null>(null);

// One preference for the whole app, not per project: the source-control panel
// is part of how someone works, not something about a particular repo.
const SOURCE_PANEL_KEY = "source-panel";

export function RunStoreProvider({ children }: { children: React.ReactNode }) {
  const [runs, setRuns] = useState<RunInfo[]>([]);
  const [projectRunLive, setProjectRunLive] = useState(false);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [view, setView] = useState<View>("grid");
  const [focusedRunId, setFocusedRun] = useState<string | null>(null);
  const [onScreenRunId, setOnScreenRun] = useState<string | null>(null);
  const [tab, setTabState] = useState<Tab>("agents");
  const [approveRunId, setApproveRun] = useState<string | null>(null);
  const [pendingSessionId, setPendingSession] = useState<string | null>(null);
  const [sourcePanelOpen, setSourcePanelOpenState] = useState<boolean>(() =>
    typeof localStorage === "undefined" ? false : loadFold(localStorage, SOURCE_PANEL_KEY, false),
  );
  // A count (not a flag) so overlapping creations can't clear each other.
  const [spawnCount, setSpawnCount] = useState(0);
  const [spawnProgress, setSpawnProgress] = useState<CloneProgress | null>(null);
  const projectRef = useRef<string | null>(null);
  projectRef.current = selectedProjectId;
  // Per-project memory of the last-viewed tab, so each project independently
  // restores where you left off. A ref (not state) because it only needs to be
  // read on project switch — the visible `tab` state drives rendering.
  const tabByProject = useRef<Record<string, Tab>>({});

  const setSourcePanelOpen = useCallback((open: boolean) => {
    setSourcePanelOpenState(open);
    try {
      if (typeof localStorage !== "undefined") saveFold(localStorage, SOURCE_PANEL_KEY, open);
    } catch {
      // ignore quota / security errors
    }
  }, []);

  const setTab = useCallback((t: Tab) => {
    setTabState(t);
    const pid = projectRef.current;
    if (pid) tabByProject.current[pid] = t;
  }, []);

  const refreshRuns = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) {
      setRuns([]);
      setProjectRunLive(false);
      return;
    }
    try {
      // The checkout's run scripts ride along on the runs tick rather than
      // polling on their own. Its failure is swallowed separately: a hiccup
      // reading the daemon should cost the dot, not the whole board.
      const [next, live] = await Promise.all([
        listRuns(pid),
        runScriptsLive(projectTarget(pid)).catch(() => false),
      ]);
      // Drop stale responses: if the selected project changed while awaiting,
      // a late reply from the old project must not overwrite the current runs.
      if (projectRef.current !== pid) return;
      // Ordered once, here, so a pinned run keeps its place in every surface
      // that reads the store: the grid, the focus rail, the sidebar tree.
      setRuns(pinnedFirst(next));
      setProjectRunLive(live);
    } catch {
      /* ignore transient errors */
    }
  }, []);

  const createAgent = useCallback(async (agentId: string, opts?: SpawnOpts) => {
    const pid = projectRef.current;
    if (!pid) return;
    const base = opts?.base ?? "HEAD";
    const mergeTarget = opts?.mergeTarget ?? null;
    // The add-menu always states its choice. Callers with no picker (the
    // menu-bar / shortcut "New Agent") take the Settings default, falling back
    // to an isolated worktree if that read fails.
    const worktree =
      opts?.worktree ?? (await getSettings().then((s) => s.defaultWorktree).catch(() => true));
    // Same for the model: a shortcut spawn repeats whatever this agent last ran
    // on, so "New Agent" doesn't quietly drop back to the default model after a
    // run was deliberately started on another one.
    const model = opts?.model !== undefined ? opts.model : await rememberedModel(agentId);
    setSpawnCount((c) => c + 1);
    setSpawnProgress(null);
    try {
      // Runs start promptless by design — the user types the real prompt into
      // the live agent terminal, and the first line is captured as the run's
      // prompt + title (see set_run_title).
      const run = await createRun(pid, "", agentId, model, base, mergeTarget, setSpawnProgress, worktree).catch((e) => {
        toastError(e, `Couldn't start ${agentId}`);
        return null;
      });
      if (!run) return;
      await refreshRuns();
      setFocusedRun(run.id);
      setView("focus");
    } finally {
      setSpawnCount((c) => c - 1);
      setSpawnProgress(null);
    }
  }, [refreshRuns]);

  const createTerminal = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) return;
    setSpawnCount((c) => c + 1);
    try {
      const run = await createTerminalApi(pid).catch((e) => {
        toastError(e, "Couldn't open terminal");
        return null;
      });
      if (!run) return;
      await refreshRuns();
      setFocusedRun(run.id);
      setView("focus");
    } finally {
      setSpawnCount((c) => c - 1);
    }
  }, [refreshRuns]);

  function setSelectedProject(id: string | null) {
    setSelectedProjectId(id);
    // Point the ref at the incoming project immediately, before React re-renders.
    // Callers routinely follow this with setTab (open an agent, an issue, a note),
    // and that call must record the tab against the project being opened — not
    // the one being left, whose remembered tab would otherwise be overwritten.
    projectRef.current = id;
    setView("grid");
    setFocusedRun(null);
    // The dot belongs to the project we just left; the next tick sets this
    // project's own.
    setProjectRunLive(false);
    // Restore this project's last-viewed tab (defaults to "agents" the first
    // time a project is opened). Viewing Files for one project and clicking
    // another lands you on that project's Files.
    if (id) setTabState(tabByProject.current[id] ?? "agents");
    // Clear any pending merge-approval so switching projects can't re-open the
    // MergeModal for a run from the old project. Same for a pending session tab:
    // its run belongs to the project we just left.
    setApproveRun(null);
    setPendingSession(null);
  }

  useEffect(() => {
    refreshRuns();
    const t = window.setInterval(refreshRuns, 1500);
    return () => window.clearInterval(t);
  }, [selectedProjectId, refreshRuns]);

  // Focus view works inside one run; the grid stands outside all of them.
  const selectedRunId = view === "focus" ? focusedRunId : null;

  return (
    <Ctx.Provider
      value={{ runs, projectRunLive, selectedProjectId, setSelectedProject, view, setView, focusedRunId, setFocusedRun, selectedRunId, sourcePanelOpen, setSourcePanelOpen, onScreenRunId, setOnScreenRun, refreshRuns, tab, setTab, createAgent, createTerminal, spawning: spawnCount > 0, spawnProgress, approveRunId, setApproveRun, pendingSessionId, setPendingSession }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function useRuns(): RunStore {
  const v = useContext(Ctx);
  if (!v) throw new Error("useRuns outside RunStoreProvider");
  return v;
}
