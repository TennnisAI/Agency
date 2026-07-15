import { useCallback, useEffect, useRef, useState } from "react";
import { Issue, listIssues } from "../api";

// The selected project's issues, polled at the same cadence as runs while
// `active` (the Issues tab is showing). Mutations call `refresh` explicitly.
export function useIssues(projectId: string | null, active: boolean) {
  const [issues, setIssues] = useState<Issue[]>([]);
  const [loaded, setLoaded] = useState(false);
  const projectRef = useRef(projectId);
  projectRef.current = projectId;

  const refresh = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) {
      setIssues([]);
      return;
    }
    try {
      const next = await listIssues(pid);
      // Drop stale replies after a project switch (same rule as refreshRuns).
      if (projectRef.current !== pid) return;
      setIssues(next);
      setLoaded(true);
    } catch {
      /* transient IPC errors: keep the last good list */
    }
  }, []);

  useEffect(() => {
    setIssues([]);
    setLoaded(false);
    if (!active) return;
    refresh();
    const t = window.setInterval(refresh, 1500);
    return () => window.clearInterval(t);
  }, [projectId, active, refresh]);

  return { issues, loaded, refresh };
}
