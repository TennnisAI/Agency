import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { InstallJob, ToolStatus, installAgent, installTool, listInstalls, toolStatus } from "../api";

/** Poll cadence while something is installing, or while the caller asks to watch PATH. */
const POLL_MS = 3000;

/**
 * The tool rows (git, npm, gh) and every background install job, kept live.
 *
 * One hook for the onboarding step, the tool dialog an error raises and the
 * onboarding agent tiles: they all need the same three things. What is on
 * PATH now, which jobs are running, and a way to start one. Jobs finish on
 * the backend's `install-finished` event; `watch` re-reads the tool statuses
 * on a timer as well, so a tool installed in some other terminal is noticed
 * without a restart.
 */
export function useToolInstalls(watch = false) {
  const [tools, setTools] = useState<ToolStatus[] | null>(null);
  const [jobs, setJobs] = useState<Record<string, InstallJob>>({});
  const live = useRef(true);

  const refresh = useCallback(() => {
    toolStatus()
      .then((t) => {
        if (live.current) setTools(t);
      })
      .catch(() => {
        // A rejected status check must not leave `tools` null for good: the
        // onboarding step list waits on it and rendered nothing at all until
        // it arrived. An empty list reads as "nothing missing", which is the
        // safe default; the next poll fills it in if the check recovers.
        if (live.current) setTools((t) => t ?? []);
      });
  }, []);

  useEffect(() => {
    live.current = true;
    refresh();
    listInstalls()
      .then((list) => {
        if (live.current) setJobs(Object.fromEntries(list.map((j) => [j.key, j])));
      })
      .catch(() => {});
    const unlisten = listen<InstallJob>("install-finished", (e) => {
      if (!live.current) return;
      setJobs((j) => ({ ...j, [e.payload.key]: e.payload }));
      refresh();
    });
    return () => {
      live.current = false;
      unlisten.then((un) => un()).catch(() => {});
    };
  }, [refresh]);

  const running = Object.values(jobs).some((j) => j.state === "running" || j.state === "queued");
  useEffect(() => {
    if (!watch && !running) return;
    const t = window.setInterval(refresh, POLL_MS);
    return () => window.clearInterval(t);
  }, [watch, running, refresh]);

  // Sync the local job map with the backend's, which knows about queueing.
  const reconcile = useCallback(() => {
    listInstalls()
      .then((list) => {
        if (live.current) setJobs((j) => ({ ...j, ...Object.fromEntries(list.map((x) => [x.key, x])) }));
      })
      .catch(() => {});
  }, []);

  const start = useCallback(async (key: string, begin: () => Promise<void>) => {
    // Optimistic "running" so the tile reacts to the click at once; the
    // reconcile after `begin` replaces it with the backend's word, which is
    // "queued" when another job holds the lane (git then gh both showed
    // "Installing…" while the second was waiting on the apt lock).
    setJobs((j) => ({ ...j, [key]: { key, state: "running", exitCode: null, output: "" } }));
    try {
      await begin();
    } catch (e) {
      // "already installing" is the backend refusing a duplicate of a job that
      // is still queued or running. That job is fine; do not paint it failed.
      if (/already installing/.test(String(e))) {
        reconcile();
        return;
      }
      setJobs((j) => ({
        ...j,
        [key]: { key, state: "failed", exitCode: null, output: String(e) },
      }));
      return;
    }
    reconcile();
  }, [reconcile]);

  const tool = useCallback((id: string) => start(`tool:${id}`, () => installTool(id)), [start]);
  const agent = useCallback(
    (id: string, command: string) => start(`agent:${id}`, () => installAgent(id, command)),
    [start],
  );

  return { tools, jobs, refresh, installTool: tool, installAgent: agent };
}

export function jobFor(jobs: Record<string, InstallJob>, key: string): InstallJob | null {
  return jobs[key] ?? null;
}

/** True while a job is still to run or running: the states a second start would be refused in. */
export function jobPending(job: InstallJob | null | undefined): boolean {
  return job?.state === "running" || job?.state === "queued";
}
