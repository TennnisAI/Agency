import { RunActivity, RunInfo, SessionStatus } from "../api";

// Presentation of a run's live state, shared by every dot/label surface
// (tiles, sidebar tree, focus rail). States for a live agent:
//   working — pulsing green: the pane changed recently
//   waiting — amber: quiet after a user-driven turn (finished or needs input)
//   idle    — dim green: alive but no turn in flight (never prompted, or the
//             wait lapsed), shown without a timer so nothing counts forever
//   exited  — gray: finished / failed / session ended
// Terminals keep plain running/ended: a quiet shell isn't "waiting on you".
//
// A pin (`RunInfo.pinRank`) only changes board order — it does not change how a
// quiet run classifies or whether it needs attention.

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

/** One agent's live state: a run's own agent, or one of its extra tabs. */
export interface AgentLive {
  status: SessionStatus;
  activity: RunActivity | null;
}

/**
 * Every agent in a run's workspace: its own agent unless that tab was closed,
 * then each extra agent tab. A "New terminal" tab (the reserved "shell"
 * profile) is not an agent, and neither is a terminal run.
 *
 * The overview used to count runs, so a workspace with a working agent and a
 * waiting one read as whatever its first agent was doing, and the second never
 * reached the header at all (AGE-249). With the first tab closed, the run's own
 * status and activity describe the extra tab standing in for it (AGE-184), so
 * that tab is counted once, from its own row.
 */
export function runAgents(run: RunInfo): AgentLive[] {
  if (run.kind !== "agent") return [];
  const agents: AgentLive[] = run.primaryClosed ? [] : [{ status: run.status, activity: run.activity }];
  for (const s of run.sessions) {
    if (s.agent !== "shell") agents.push({ status: s.status, activity: s.activity });
  }
  return agents;
}

/**
 * An agent asking for the user right now: live, and gone quiet after a turn
 * the user drove.
 *
 * The backend's `waiting` is about the pane alone, so the running condition is
 * composed in here rather than there: it is the same condition the dots and
 * labels below already key off, and a dead session is never waiting on you
 * whatever its last sample said.
 */
export function agentWaiting(a: AgentLive): boolean {
  return a.status.state === "running" && a.activity?.state === "waiting";
}

/** A live agent actively producing output (no activity sample yet counts:
 * a freshly spawned agent is busy starting up). */
export function agentWorking(a: AgentLive): boolean {
  return a.status.state === "running" && (a.activity === null || a.activity.state === "working");
}

/** How many of the agents in `runs` pass `pred`, every tab counted. */
export function countAgents(runs: RunInfo[], pred: (a: AgentLive) => boolean = () => true): number {
  return runs.reduce((n, r) => n + runAgents(r).filter(pred).length, 0);
}

/**
 * A run with an agent asking for the user right now, in any of its tabs.
 * What the tiles badge and the "waiting" filter keeps.
 */
export function needsAttention(run: RunInfo): boolean {
  return runAgents(run).some(agentWaiting);
}

export function isPinned(run: RunInfo): boolean {
  return run.pinRank != null;
}

/**
 * Board order: pinned runs first, in the order the user put them, then
 * everything else in the order the backend sent (newest run first). A pin
 * holds its place regardless of what the run is doing, which is the whole
 * point of one — so this sorts on the pin alone and lets lifecycle ordering
 * happen inside each half.
 */
function byPin(a: RunInfo, b: RunInfo): number {
  const pa = a.pinRank;
  const pb = b.pinRank;
  if (pa == null && pb == null) return 0;
  if (pa == null) return 1;
  if (pb == null) return -1;
  return pa - pb;
}

/** `list` in board order, without mutating it. */
export function pinnedFirst(runs: RunInfo[]): RunInfo[] {
  return [...runs].sort(byPin);
}

/**
 * Each pinned run's place among the pins, by id. `runs` must already be in
 * board order, as the store keeps them (`pinnedFirst` runs on every refresh),
 * so this is one pass rather than a sort.
 */
export function pinIndex(runs: RunInfo[]): Map<string, number> {
  const index = new Map<string, number>();
  for (const r of runs) if (isPinned(r)) index.set(r.id, index.size);
  return index;
}

/**
 * Which side of `overId` the dragged run `id` would land on, for the drop
 * indicator: before it when dragged up the board, after it when dragged down.
 * The same rule the backend's `pin_order::plan_move` lands it by. Null over
 * the run's own place, or anything that is not a pin.
 */
export function pinDropSide(index: Map<string, number>, id: string, overId: string): "before" | "after" | null {
  const from = index.get(id);
  const to = index.get(overId);
  if (from == null || to == null || from === to) return null;
  return to < from ? "before" : "after";
}

/** A run with an agent producing output, in any of its tabs. */
export function isWorking(run: RunInfo): boolean {
  return runAgents(run).some(agentWorking);
}

/**
 * What the overview's header counts filter the board down to (AGE-247). Each
 * keeps the runs holding an agent its count counted, with the same predicate,
 * so clicking "3 waiting" shows every workspace those three are in: three
 * tiles, or fewer when two of them share a workspace.
 */
export type RunFilter = "all" | "working" | "waiting";

export function matchesRunFilter(run: RunInfo, filter: RunFilter): boolean {
  if (filter === "working") return isWorking(run);
  if (filter === "waiting") return needsAttention(run);
  return true;
}

/** A stored filter, read back through an allowlist: anything else is "all". */
export function parseRunFilter(raw: string | null): RunFilter {
  return raw === "working" || raw === "waiting" ? raw : "all";
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

/**
 * The agent's own status line for a card, or null when there is none to show.
 *
 * Shown only beside a live agent: the backend already drops it once the
 * session stops, and a terminal has no agent to write one. `stale` comes from
 * the backend, which has the busy streaks: a line the agent set before a
 * later stretch of work it has since finished. The card dims it rather than
 * letting it read as current. Not "set before the run went quiet", because the
 * set_status call is itself drawn in the pane and so always precedes quiet.
 */
export function agentNote(
  run: RunInfo,
  now: number = Date.now(),
): { text: string; title: string; stale: boolean } | null {
  const s = run.agentStatus;
  if (!s || run.kind !== "agent" || run.status.state !== "running") return null;
  return {
    text: s.text,
    title: `Written by the agent ${fmtDur(now - s.since)} ago${s.stale ? ", before its latest work" : ""}`,
    stale: s.stale,
  };
}

export function runStatus(
  run: Pick<RunInfo, "kind" | "status" | "activity">,
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
