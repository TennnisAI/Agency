import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listIssues, listProjects, listRuns } from "../api";
import { CrossRefs, CrossSource, buildCrossRefs } from "../lib/links";

const POLL_MS = 5000;

/**
 * Everything the `CrossRefs` model exposes — the LinkProject / LinkIssue /
 * LinkRun views, which are all a consumer can reach through it. An unchanged
 * tick publishing a fresh object would churn every memo downstream (the
 * mentions link table rebuilds over every issue body; the markdown editors
 * reconfigure a CodeMirror compartment), five seconds at a time, so a poll
 * that finds nothing new keeps the object it already published.
 */
function signature(sources: CrossSource[]): string {
  return JSON.stringify(
    sources.map(({ project, issues, runs }) => [
      project.id, project.name, project.issue_key,
      issues.map((i) => [i.id, i.projectId, i.seq, i.title, i.body, i.status, i.links]),
      runs.map((r) => [r.id, r.projectId, r.issueId]),
    ]),
  );
}

/**
 * Cross-project reference data for typed wikilinks and the mentions panels
 * (one-stop Phase 7): every project's issues and runs, fan-out fetched (the
 * palette's "@" pattern) and polled while `active`. A failing project drops
 * out of that tick rather than failing the whole pass. `cross`/`sources` are
 * null until the first pass lands, and hold their identity across a tick that
 * fetched the same rows (see `signature`).
 */
export function useCrossRefs(active: boolean) {
  const [sources, setSources] = useState<CrossSource[] | null>(null);
  const token = useRef(0);
  const sigRef = useRef<string | null>(null);

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
      const fetched = next.filter((s): s is CrossSource => s !== null);
      const sig = signature(fetched);
      if (sig === sigRef.current) return;
      sigRef.current = sig;
      setSources(fetched);
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
