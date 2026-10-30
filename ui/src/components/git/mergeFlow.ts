// A conflicted merge outlives the window that started it. "Fix with agent"
// closes that window (AGE-273), and the source control panel carries the merge
// from there, so what the window knew can't live in its component state: which
// agent got the conflict, and the two answers the end of the merge depends on.
// Keyed by run, in the same shape as ./ops. In memory only, so a restart loses
// the answers and falls back to the safe half of each; git still says the merge
// is there, and the panel picks it back up from that.

import { useSyncExternalStore } from "react";
import type { MergeState, PrInfo } from "../../api";

// Who has the conflict. `label` is the name as it read when it was sent, since a
// tab can be closed afterwards and the sentence still has to name it.
export interface MergeHandoff {
  session: string;
  label: string;
  // A tab opened for this conflict, rather than one that was already there.
  spawned: boolean;
  // The prompt is waiting for the agent to be free, not with the agent yet.
  queued: boolean;
}

export interface MergeFlow {
  // The merge window's "delete the remote branch" tick. It reset to ticked on
  // every reopen, so a box unticked before the conflict was ignored at the end.
  deleteRemote: boolean;
  // Whether the PR probe had answered by the time the merge started, and what it
  // said. The section that runs the probe only exists on the review step, so a
  // window reopened mid-merge never learns: it would keep the remote branch and
  // warn about a PR that may not exist, on every merge that came through here.
  prSettled: boolean;
  pr: PrInfo | null;
  handoff: MergeHandoff | null;
  // The panel's Finish merge opens the window to land the merge there, because
  // the cleanup choices that follow are the window's.
  finishOnOpen: boolean;
}

const FRESH: MergeFlow = {
  deleteRemote: true,
  prSettled: false,
  pr: null,
  handoff: null,
  finishOnOpen: false,
};

// Replaced rather than mutated, so the map itself is the snapshot of the store.
let flows: ReadonlyMap<string, MergeFlow> = new Map();
const listeners = new Set<() => void>();

function publish(next: Map<string, MergeFlow>) {
  flows = next;
  listeners.forEach((notify) => notify());
}

export function mergeFlow(runId: string): MergeFlow | null {
  return flows.get(runId) ?? null;
}

// Creates the entry if the run has none: a merge the panel found by asking git
// starts from the same defaults a reopened window always had.
export function patchMergeFlow(runId: string, patch: Partial<MergeFlow>): void {
  publish(new Map(flows).set(runId, { ...(flows.get(runId) ?? FRESH), ...patch }));
}

export function endMergeFlow(runId: string): void {
  if (!flows.has(runId)) return;
  const next = new Map(flows);
  next.delete(runId);
  publish(next);
}

function subscribe(notify: () => void): () => void {
  listeners.add(notify);
  return () => { listeners.delete(notify); };
}

export function useMergeFlows(): ReadonlyMap<string, MergeFlow> {
  return useSyncExternalStore(subscribe, () => flows);
}

/** Test hook. */
export function clearMergeFlows(): void {
  flows = new Map();
}

// What git's answer means for the entry. A merge in progress always has one. The
// entry ends once there is neither a merge nor a landed branch, which is someone
// aborting it from a terminal. `merged` alone starts nothing: it is also true of
// any branch with no commits of its own, so it only keeps a merge this store
// already knew about, the one an agent committed by itself.
export function reconcileFlow(
  flow: MergeFlow | null,
  state: MergeState,
): "start" | "end" | "keep" {
  if (state.merging) return flow ? "keep" : "start";
  return flow && !state.merged ? "end" : "keep";
}

export type MergeCardPhase = "hidden" | "conflicted" | "ready";

export function mergeCardPhase(flow: MergeFlow | null, state: MergeState | null): MergeCardPhase {
  // The checkout has moved on to another run's merge, which a Finish here would
  // only be refused over.
  if (!flow || !state || state.blockedBy) return "hidden";
  if (state.merging) return state.unresolved.length > 0 ? "conflicted" : "ready";
  return state.merged ? "ready" : "hidden";
}

// Where the conflict is now, in one sentence. Each of these used to end with
// instructions for closing the window, which closes itself now.
export function handoffNote(h: MergeHandoff): string {
  if (h.spawned) {
    return h.queued
      ? `Started ${h.label}. The conflict goes in once it reaches its prompt.`
      : `Started ${h.label} on the conflict.`;
  }
  return h.queued
    ? `Queued for ${h.label}. The conflict goes in once that agent is free.`
    : `Sent to ${h.label}.`;
}

// Which agent the picker sits on: the one chosen here, then the one that already
// has the conflict, then the tab this run was last open on, which is the app's
// best record of "the agent I was just working with". A tab can close while any
// of those is remembered, so each has to still be on offer.
export function pickTarget(
  values: string[],
  picked: string | null,
  handedTo: string | null,
  remembered: string,
): string | null {
  for (const v of [picked, handedTo, remembered]) {
    if (v !== null && values.includes(v)) return v;
  }
  return values[0] ?? null;
}
