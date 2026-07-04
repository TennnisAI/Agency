import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { RunInfo, createRun, createTerminal as createTerminalApi, listRuns } from "../api";
import { toastError } from "../lib/toast";

type View = "grid" | "focus";
type Tab = "agents" | "source" | "files";

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
  createAgent: (agentId: string, opts?: { base: string; mergeTarget: string }) => Promise<void>;
  createTerminal: () => Promise<void>;
  approveRunId: string | null;
  setApproveRun: (id: string | null) => void;
}

const Ctx = createContext<RunStore | null>(null);

export function RunStoreProvider({ children }: { children: React.ReactNode }) {
  const [runs, setRuns] = useState<RunInfo[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [view, setView] = useState<View>("grid");
  const [focusedRunId, setFocusedRun] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("agents");
  const [approveRunId, setApproveRun] = useState<string | null>(null);
  const projectRef = useRef<string | null>(null);
  projectRef.current = selectedProjectId;

  const refreshRuns = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) {
      setRuns([]);
      return;
    }
    try {
      setRuns(await listRuns(pid));
    } catch {
      /* ignore transient errors */
    }
  }, []);

  const createAgent = useCallback(async (agentId: string, opts?: { base: string; mergeTarget: string }) => {
    const pid = projectRef.current;
    if (!pid) return;
    const base = opts?.base ?? "HEAD";
    const mergeTarget = opts?.mergeTarget ?? null;
    // Runs start promptless by design — the user types the real prompt into
    // the live agent terminal, and the first line is captured as the run's
    // prompt + title (see set_run_title).
    const run = await createRun(pid, "", agentId, base, mergeTarget).catch((e) => {
      toastError(e, `Couldn't start ${agentId}`);
      return null;
    });
    if (!run) return;
    await refreshRuns();
    setFocusedRun(run.id);
    setView("focus");
  }, [refreshRuns]);

  const createTerminal = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) return;
    const run = await createTerminalApi(pid).catch((e) => {
      toastError(e, "Couldn't open terminal");
      return null;
    });
    if (!run) return;
    await refreshRuns();
    setFocusedRun(run.id);
    setView("focus");
  }, [refreshRuns]);

  function setSelectedProject(id: string | null) {
    setSelectedProjectId(id);
    setView("grid");
    setFocusedRun(null);
    setTab("agents");
  }

  useEffect(() => {
    refreshRuns();
    const t = window.setInterval(refreshRuns, 1500);
    return () => window.clearInterval(t);
  }, [selectedProjectId, refreshRuns]);

  return (
    <Ctx.Provider
      value={{ runs, selectedProjectId, setSelectedProject, view, setView, focusedRunId, setFocusedRun, refreshRuns, tab, setTab, createAgent, createTerminal, approveRunId, setApproveRun }}
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
