import { RunInfo, RunStanding } from "../api";

// Presentation of a run's live state, shared by every dot/label surface
// (tiles, sidebar tree, focus rail). States for a live agent:
//   working — pulsing green: the pane changed recently
//   waiting — amber: quiet after a user-driven turn (finished or needs input)
//   idle    — dim green: alive but no turn in flight (never prompted, or the
//             wait lapsed), shown without a timer so nothing counts forever
//   exited  — gray: finished / failed / session ended
// Terminals keep plain running/ended: a quiet shell isn't "waiting on you".
//
// Over the top of that sits what the user has said about the run — settled,
// active, snoozed, pinned (`RunAttention`). The state stays what the pane
// says; the standing decides whether it is worth surfacing, and takes the
// amber down when the answer is no.

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

/**
 * A run that is asking for the user right now: waiting, and nothing the user
 * has said about it says otherwise. This, not `isWaiting`, is what the counts
 * and the attention badges read — settling a run is meant to take it off the
 * list, not to relabel it.
 */
export function needsAttention(run: RunInfo): boolean {
  return isWaiting(run) && run.attention.needsAttention;
}

export function isPinned(run: RunInfo): boolean {
  return run.attention.pinRank != null;
}

/**
 * Board order: pinned runs first, in the order they were pinned, then
 * everything else in the order the backend sent (newest run first). A pin
 * holds its place regardless of what the run is doing, which is the whole
 * point of one — so this sorts on the pin alone and lets lifecycle ordering
 * happen inside each half.
 */
function byPin(a: RunInfo, b: RunInfo): number {
  const pa = a.attention.pinRank;
  const pb = b.attention.pinRank;
  if (pa == null && pb == null) return 0;
  if (pa == null) return 1;
  if (pb == null) return -1;
  return pa - pb;
}

/** `list` in board order, without mutating it. */
export function pinnedFirst(runs: RunInfo[]): RunInfo[] {
  return [...runs].sort(byPin);
}

/** Wake times a snooze can be set to, as epoch ms from `now`. */
export const SNOOZE_PRESETS: { label: string; ms: number }[] = [
  { label: "15 minutes", ms: 15 * 60 * 1000 },
  { label: "1 hour", ms: 60 * 60 * 1000 },
  { label: "4 hours", ms: 4 * 60 * 60 * 1000 },
];

/** Clock time a snooze wakes at, for a menu label and a tooltip. */
export function fmtWake(untilMs: number): string {
  return new Date(untilMs).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/**
 * How a standing reads where it replaces the status line. Snoozing shows the
 * time left rather than the wake time: the question a glance is asking is "how
 * long until this is back", and the exact clock time is in the tooltip. An
 * active run keeps its own waiting line, so that label is only for a surface
 * that names the standing outright.
 */
export function standingLabel(standing: RunStanding, now: number = Date.now()): string {
  if (standing.kind === "settled") return "settled";
  if (standing.kind === "active") return "active";
  return `snoozed · ${fmtDur(Math.max(0, standing.untilMs - now))}`;
}

/** The hover text that goes with `standingLabel`. */
export function standingTitle(standing: RunStanding): string {
  if (standing.kind === "settled") {
    return "You have dealt with this run. It raises its hand again when the agent comes back.";
  }
  if (standing.kind === "active") {
    return "You marked this run as still needing you, so it keeps its place on the list.";
  }
  return `Snoozed until ${fmtWake(standing.untilMs)}. New output wakes it early.`;
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
      // The user has answered this one already: no amber, no climbing timer,
      // and the line says what they said rather than what the pane did.
      const standing = run.attention.standing;
      if (standing && standing.kind !== "active") {
        return { cls: "idle", text: standingLabel(standing, now), title: standingTitle(standing) };
      }
      return {
        cls: "awaiting",
        text: `waiting · ${fmtDur(now - a.since)}`,
        title: standing
          ? standingTitle(standing)
          : "The agent finished a turn or needs input. Waiting on you.",
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
