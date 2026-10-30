import { describe, it, expect, beforeEach } from "vitest";
import type { MergeState } from "../../api";
import {
  MergeFlow, clearMergeFlows, endMergeFlow, handoffNote, mergeCardPhase, mergeFlow,
  mergePollMs, patchMergeFlow, pickTarget, reconcileFlow,
} from "./mergeFlow";

const state = (over: Partial<MergeState> = {}): MergeState => ({
  merging: false, unresolved: [], merged: false, blockedBy: null, ...over,
});
const flow: MergeFlow = {
  deleteRemote: true, prSettled: false, pr: null, handoff: null, finishOnOpen: false,
};

describe("merge flow store", () => {
  beforeEach(clearMergeFlows);

  it("creates an entry on first patch, from the defaults", () => {
    patchMergeFlow("a", { deleteRemote: false });
    expect(mergeFlow("a")).toEqual({ ...flow, deleteRemote: false });
    expect(mergeFlow("b")).toBeNull();
  });

  it("keeps earlier answers through a later patch", () => {
    patchMergeFlow("a", { deleteRemote: false, prSettled: true });
    patchMergeFlow("a", { handoff: { session: "a", label: "Claude", spawned: false, queued: false } });
    expect(mergeFlow("a")?.deleteRemote).toBe(false);
    expect(mergeFlow("a")?.prSettled).toBe(true);
  });

  it("gives a new identity on change, which is how a stale poll is spotted", () => {
    patchMergeFlow("a", {});
    const asked = mergeFlow("a");
    expect(mergeFlow("a")).toBe(asked);
    patchMergeFlow("a", { finishOnOpen: true });
    expect(mergeFlow("a")).not.toBe(asked);
  });

  it("forgets a run once its merge ends", () => {
    patchMergeFlow("a", {});
    endMergeFlow("a");
    expect(mergeFlow("a")).toBeNull();
  });
});

describe("reconcileFlow", () => {
  it("starts on a merge found in progress", () => {
    expect(reconcileFlow(null, state({ merging: true }))).toBe("start");
    expect(reconcileFlow(flow, state({ merging: true }))).toBe("keep");
  });

  it("does not start for a branch that merely has nothing of its own", () => {
    expect(reconcileFlow(null, state({ merged: true }))).toBe("keep");
  });

  it("keeps a merge the agent committed itself, so it can still be finished", () => {
    expect(reconcileFlow(flow, state({ merged: true }))).toBe("keep");
  });

  it("ends when the merge was aborted elsewhere", () => {
    expect(reconcileFlow(flow, state())).toBe("end");
    expect(reconcileFlow(flow, state({ blockedBy: "agent/other" }))).toBe("end");
  });
});

describe("mergePollMs", () => {
  it("asks less often when no merge is being tracked", () => {
    expect(mergePollMs(false)).toBeGreaterThan(mergePollMs(true));
  });
});

describe("mergeCardPhase", () => {
  it("shows nothing without both an entry and an answer from git", () => {
    expect(mergeCardPhase(null, state({ merging: true }))).toBe("hidden");
    expect(mergeCardPhase(flow, null)).toBe("hidden");
    expect(mergeCardPhase(flow, state())).toBe("hidden");
    expect(mergeCardPhase(flow, state({ merged: true, blockedBy: "agent/other" }))).toBe("hidden");
  });

  it("reads conflicted while git still has unmerged paths", () => {
    expect(mergeCardPhase(flow, state({ merging: true, unresolved: ["a.rs"] }))).toBe("conflicted");
  });

  it("reads ready once nothing is unmerged, committed or not", () => {
    expect(mergeCardPhase(flow, state({ merging: true }))).toBe("ready");
    expect(mergeCardPhase(flow, state({ merged: true }))).toBe("ready");
  });
});

describe("handoffNote", () => {
  const h = { session: "a", label: "Claude", spawned: false, queued: false };

  it("says something different for each way the prompt can land", () => {
    const notes = [
      handoffNote(h),
      handoffNote({ ...h, queued: true }),
      handoffNote({ ...h, spawned: true }),
      handoffNote({ ...h, spawned: true, queued: true }),
    ];
    expect(new Set(notes).size).toBe(4);
    notes.forEach((n) => expect(n).toContain("Claude"));
  });

  it("does not claim a queued prompt has reached the agent", () => {
    expect(handoffNote({ ...h, queued: true })).not.toContain("Sent");
    expect(handoffNote({ ...h, spawned: true, queued: true })).not.toContain("on the conflict");
  });
});

describe("pickTarget", () => {
  const values = ["run", "run--2", "new agent"];

  it("prefers the choice made here, then who has it, then the remembered tab", () => {
    expect(pickTarget(values, "run--2", "run", "run")).toBe("run--2");
    expect(pickTarget(values, null, "run--2", "run")).toBe("run--2");
    expect(pickTarget(values, null, null, "run--2")).toBe("run--2");
  });

  it("skips a tab that has closed", () => {
    expect(pickTarget(values, "run--9", "run--8", "run--2")).toBe("run--2");
    expect(pickTarget(values, null, null, "run--9")).toBe("run");
  });

  it("has nothing to pick from an empty list", () => {
    expect(pickTarget([], null, null, "run")).toBeNull();
  });
});
