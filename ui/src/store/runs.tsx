import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { CloneProgress, RunInfo, createRun, createTerminal as createTerminalApi, getSettings, listRuns } from "../api";
import { toastError } from "../lib/toast";

type View = "grid" | "focus";
// What the add-menu hands to a spawn. `worktree: false` means "run in the
// project checkout on its current branch", which makes base/mergeTarget moot.
// Omitting it (the menu-bar shortcut, which has no picker) defers to the
// Settings default.
export type SpawnOpts = { base: string; mergeTarget: string; worktree?: boolean };
type Tab = "agents" | "source" | "files" | "issues" | "docs";

interface RunStore {
  runs: RunInfo[];
  selectedProjectId: string | null;
  setSelectedProject: (id: string | null) => void;
  view: View;
  setView: (v: View) => void;
  focusedRunId: string | null;
  setFocusedRun: (id: string | null) => void;
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

export function RunStoreProvider({ children }: { children: React.ReactNode }) {
  const [runs, setRuns] = useState<RunInfo[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [view, setView] = useState<View>("grid");
  const [focusedRunId, setFocusedRun] = useState<string | null>(null);
  const [tab, setTabState] = useState<Tab>("agents");
  const [approveRunId, setApproveRun] = useState<string | null>(null);
  const [pendingSessionId, setPendingSession] = useState<string | null>(null);
  // A count (not a flag) so overlapping creations can't clear each other.
  const [spawnCount, setSpawnCount] = useState(0);
  const [spawnProgress, setSpawnProgress] = useState<CloneProgress | null>(null);
  const projectRef = useRef<string | null>(null);
  projectRef.current = selectedProjectId;
  // Per-project memory of the last-viewed tab, so each project independently
  // restores where you left off. A ref (not state) because it only needs to be
  // read on project switch — the visible `tab` state drives rendering.
  const tabByProject = useRef<Record<string, Tab>>({});

  const setTab = useCallback((t: Tab) => {
    setTabState(t);
    const pid = projectRef.current;
    if (pid) tabByProject.current[pid] = t;
  }, []);

  const refreshRuns = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) {
      setRuns([]);
      return;
    }
    try {
      const next = await listRuns(pid);
      // Drop stale responses: if the selected project changed while awaiting,
      // a late reply from the old project must not overwrite the current runs.
      if (projectRef.current !== pid) return;
      setRuns(next);
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
    setSpawnCount((c) => c + 1);
    setSpawnProgress(null);
    try {
      // Runs start promptless by design — the user types the real prompt into
      // the live agent terminal, and the first line is captured as the run's
      // prompt + title (see set_run_title).
      const run = await createRun(pid, "", agentId, base, mergeTarget, setSpawnProgress, worktree).catch((e) => {
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
    setView("grid");
    setFocusedRun(null);
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

  return (
    <Ctx.Provider
      value={{ runs, selectedProjectId, setSelectedProject, view, setView, focusedRunId, setFocusedRun, refreshRuns, tab, setTab, createAgent, createTerminal, spawning: spawnCount > 0, spawnProgress, approveRunId, setApproveRun, pendingSessionId, setPendingSession }}
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
