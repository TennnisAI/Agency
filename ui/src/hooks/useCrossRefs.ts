import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listIssues, listProjects, listRuns } from "../api";
import { CrossRefs, CrossSource, buildCrossRefs } from "../lib/links";

const POLL_MS = 5000;

/**
 * Cross-project reference data for typed wikilinks and the mentions panels
 * (one-stop Phase 7): every project's issues and runs, fan-out fetched (the
 * palette's "@" pattern) and polled while `active`. A failing project drops
 * out of that tick rather than failing the whole pass. `cross`/`sources` are
 * null until the first pass lands.
 */
export function useCrossRefs(active: boolean) {
  const [sources, setSources] = useState<CrossSource[] | null>(null);
  const token = useRef(0);

  const refresh = useCallback(async () => {
    const t = ++token.current;
    try {
      const projects = await listProjects();
      const next = await Promise.all(
        projects.map((project) =>
          Promise.all([listIssues(project.id), listRuns(project.id)])
            .then(([issues, runs]): CrossSource => ({ project, issues, runs }))
            .catch(() => null),
        ),
      );
      if (token.current !== t) return;
      setSources(next.filter((s): s is CrossSource => s !== null));
    } catch {
      /* transient IPC errors: keep the last good data */
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    void refresh();
    const t = window.setInterval(refresh, POLL_MS);
    return () => window.clearInterval(t);
  }, [active, refresh]);

  const cross: CrossRefs | null = useMemo(
    () => (sources ? buildCrossRefs(sources) : null),
    [sources],
  );

  return { cross, sources, refresh };
}
