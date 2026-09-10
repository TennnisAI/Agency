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
      .catch(() => {});
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

  const running = Object.values(jobs).some((j) => j.state === "running");
  useEffect(() => {
    if (!watch && !running) return;
    const t = window.setInterval(refresh, POLL_MS);
    return () => window.clearInterval(t);
  }, [watch, running, refresh]);

  const start = useCallback(async (key: string, begin: () => Promise<void>) => {
    setJobs((j) => ({ ...j, [key]: { key, state: "running", exitCode: null, output: "" } }));
    try {
      await begin();
    } catch (e) {
      setJobs((j) => ({
        ...j,
        [key]: { key, state: "failed", exitCode: null, output: String(e) },
      }));
    }
  }, []);

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
