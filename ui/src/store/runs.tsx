import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { RunInfo, listRuns } from "../api";

type View = "grid" | "focus";

interface RunStore {
  runs: RunInfo[];
  selectedProjectId: string | null;
  setSelectedProject: (id: string | null) => void;
  view: View;
  setView: (v: View) => void;
  focusedRunId: string | null;
  setFocusedRun: (id: string | null) => void;
  refreshRuns: () => Promise<void>;
}

const Ctx = createContext<RunStore | null>(null);

export function RunStoreProvider({ children }: { children: React.ReactNode }) {
  const [runs, setRuns] = useState<RunInfo[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [view, setView] = useState<View>("grid");
  const [focusedRunId, setFocusedRun] = useState<string | null>(null);
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

  function setSelectedProject(id: string | null) {
    setSelectedProjectId(id);
    setView("grid");
    setFocusedRun(null);
  }

  useEffect(() => {
    refreshRuns();
    const t = window.setInterval(refreshRuns, 1500);
    return () => window.clearInterval(t);
  }, [selectedProjectId, refreshRuns]);

  return (
    <Ctx.Provider
      value={{ runs, selectedProjectId, setSelectedProject, view, setView, focusedRunId, setFocusedRun, refreshRuns }}
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
