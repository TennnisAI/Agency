import { RunInfo } from "../api";

// Presentation of a run's live state, shared by every dot/label surface
// (tiles, sidebar tree, focus rail). States for a live agent:
//   working — pulsing green: the pane changed recently
//   waiting — amber: quiet after a user-driven turn (finished or needs input)
//   idle    — dim green: alive but no turn in flight (never prompted, or the
//             wait lapsed), shown without a timer so nothing counts forever
//   exited  — gray: finished / failed / session ended
// Terminals keep plain running/ended: a quiet shell isn't "waiting on you".

/** Milliseconds elapsed rendered at the coarsest honest granularity. */
export function fmtDur(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m`;
  return `${Math.floor(h / 24)}d`;
}

/** A running agent that went quiet after a user-driven turn. */
export function isWaiting(run: RunInfo): boolean {
  return run.kind === "agent" && run.status.state === "running" && run.activity?.state === "waiting";
}

/** A running agent actively producing output (no activity sample yet counts:
 * a freshly spawned agent is busy starting up). */
export function isWorking(run: RunInfo): boolean {
  return (
    run.kind === "agent" &&
    run.status.state === "running" &&
    (run.activity === null || run.activity.state === "working")
  );
}

/**
 * An agent working in a project folder that has no git repository. It has no
 * branch to name and no diff to count, so the tiles show neither. A worktree
 * run always has a branch, and a run in a real checkout reports the live one,
 * so an empty branch on a worktree-less run means exactly this.
 */
export function inGitlessFolder(run: Pick<RunInfo, "worktree" | "branch">): boolean {
  return !run.worktree && !run.branch;
}

export function runStatus(
  run: RunInfo,
  now: number = Date.now(),
): { cls: string; text: string; title?: string } {
  if (run.status.state === "running") {
    if (run.kind === "terminal") return { cls: "running", text: "running" };
    const a = run.activity;
    if (a?.state === "waiting") {
      return {
        cls: "awaiting",
        text: `waiting · ${fmtDur(now - a.since)}`,
        title: "The agent finished a turn or needs input. Waiting on you.",
      };
    }
    if (a?.state === "idle") {
      return { cls: "idle", text: "idle", title: "No turn in flight. Send the agent a message." };
    }
    return { cls: "running", text: a ? `working · ${fmtDur(now - a.since)}` : "working" };
  }
  if (run.status.state === "exited") {
    if (run.status.code === 0) return { cls: "exited", text: "finished" };
    return { cls: "exited", text: "failed", title: `exited (${run.status.code})` };
  }
  return { cls: "exited", text: "session ended" };
}
