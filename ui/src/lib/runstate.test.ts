import { describe, expect, it } from "vitest";
import { RunInfo } from "../api";
import {
  agentNote,
  fmtDur,
  inGitlessFolder,
  isPinned,
  needsAttention,
  pinDropSide,
  pinnedFirst,
  planPinDrop,
  runStatus,
} from "./runstate";

/** A live agent, quiet after a turn the user drove. */
const waitingRun = (over: Partial<RunInfo> = {}): RunInfo =>
  ({
    kind: "agent",
    status: { state: "running" },
    activity: { state: "waiting", since: 0 },
    pinRank: null,
    ...over,
  }) as RunInfo;

describe("inGitlessFolder", () => {
  it("is true only for a worktree-less run with no branch", () => {
    const run = (worktree: boolean, branch: string) => ({ worktree, branch }) as RunInfo;
    expect(inGitlessFolder(run(false, ""))).toBe(true);
    expect(inGitlessFolder(run(false, "main"))).toBe(false);
    expect(inGitlessFolder(run(true, ""))).toBe(false);
  });

  it("is false for a run whose branch is only whitespace", () => {
    expect(inGitlessFolder({ worktree: false, branch: "  " } as RunInfo)).toBe(false);
  });
});

describe("needsAttention", () => {
  it("is true for a live agent quiet after a user-driven turn", () => {
    expect(needsAttention(waitingRun())).toBe(true);
  });

  it("is false for a terminal, which is never waiting on you", () => {
    expect(needsAttention(waitingRun({ kind: "terminal" }))).toBe(false);
  });

  it("is false once the session is gone, whatever the last sample said", () => {
    expect(needsAttention(waitingRun({ status: { state: "exited", code: 0 } as never }))).toBe(
      false,
    );
  });

  it("is false while the agent is working, and before the first sample", () => {
    expect(needsAttention(waitingRun({ activity: { state: "working", since: 0 } }))).toBe(false);
    expect(needsAttention(waitingRun({ activity: null }))).toBe(false);
  });

  it("stays true when the run is pinned: a pin is placement, not suppression", () => {
    expect(needsAttention(waitingRun({ pinRank: 1 }))).toBe(true);
  });
});

describe("runStatus", () => {
  it("shows the climbing timer for a waiting run", () => {
    const st = runStatus(waitingRun(), 3_600_000);
    expect(st.cls).toBe("awaiting");
    expect(st.text).toBe("waiting · 1h 0m");
  });

  it("keeps the waiting line when the run is pinned", () => {
    const st = runStatus(waitingRun({ pinRank: 1 }), 60_000);
    expect(st.cls).toBe("awaiting");
    expect(st.text).toBe("waiting · 1m");
  });

  it("drops the timer for an idle run, so nothing counts forever", () => {
    const st = runStatus(waitingRun({ activity: { state: "idle", since: 0 } }), 3_600_000);
    expect(st.cls).toBe("idle");
    expect(st.text).not.toMatch(/·/);
  });
});

describe("isPinned", () => {
  it("reads the pin rank, including the zero rank", () => {
    expect(isPinned(waitingRun())).toBe(false);
    expect(isPinned(waitingRun({ pinRank: 2 }))).toBe(true);
    expect(isPinned(waitingRun({ pinRank: 0 }))).toBe(true);
  });
});

describe("fmtDur", () => {
  it("steps down to the coarsest honest granularity", () => {
    expect(fmtDur(9_000)).toBe("9s");
    expect(fmtDur(59_999)).toBe("59s");
    expect(fmtDur(60_000)).toBe("1m");
    expect(fmtDur(59 * 60_000)).toBe("59m");
    expect(fmtDur(3_600_000)).toBe("1h 0m");
    expect(fmtDur(90 * 60_000)).toBe("1h 30m");
    expect(fmtDur(48 * 3_600_000)).toBe("2d");
  });

  it("never counts below zero, so a clock skew reads as 0s not -1s", () => {
    expect(fmtDur(-1)).toBe("0s");
    expect(fmtDur(-90_000)).toBe("0s");
  });
});

describe("agentNote", () => {
  const working = (over: Partial<RunInfo> = {}): RunInfo =>
    waitingRun({
      activity: { state: "working", since: 1_000 },
      agentStatus: { text: "running the migration tests", since: 5_000, stale: false },
      ...over,
    });

  it("shows the agent's line beside a live agent, with its age", () => {
    expect(agentNote(working(), 245_000)).toEqual({
      text: "running the migration tests",
      title: "Written by the agent 4m ago",
      stale: false,
    });
  });

  it("is null when the agent has not set one", () => {
    expect(agentNote(working({ agentStatus: null }))).toBeNull();
  });

  it("is null for a terminal or a stopped session, whatever was last set", () => {
    expect(agentNote(working({ kind: "terminal" }))).toBeNull();
    expect(agentNote(working({ status: { state: "exited", code: 0 } as never }))).toBeNull();
  });

  it("dims a line the backend marks stale, and says why", () => {
    const run = working({
      activity: { state: "waiting", since: 9_000 },
      agentStatus: { text: "running the migration tests", since: 5_000, stale: true },
    });
    const note = agentNote(run, 65_000);
    expect(note?.stale).toBe(true);
    expect(note?.title).toBe("Written by the agent 1m ago, before its latest work");
  });

  // A line set as the agent's last act predates the pane going quiet, because
  // the call is drawn in the pane. Only the backend flag decides.
  it("keeps a line current after the run goes quiet unless the backend says stale", () => {
    const run = working({ activity: { state: "waiting", since: 9_000 } });
    expect(agentNote(run)?.stale).toBe(false);
  });
});

describe("pinnedFirst", () => {
  const run = (id: string, pinRank: number | null): RunInfo =>
    ({ ...waitingRun(), id, pinRank }) as RunInfo;

  it("puts pinned runs first, in pin order, leaving the rest alone", () => {
    const list = [run("a", null), run("b", 2), run("c", 1), run("d", null)];
    expect(pinnedFirst(list).map((r) => r.id)).toEqual(["c", "b", "a", "d"]);
  });

  it("does not mutate the list it was given", () => {
    const list = [run("a", null), run("b", 1)];
    pinnedFirst(list);
    expect(list.map((r) => r.id)).toEqual(["a", "b"]);
  });
});

describe("planPinDrop", () => {
  const run = (id: string, pinRank: number | null): RunInfo =>
    ({ ...waitingRun(), id, pinRank }) as RunInfo;
  // Pinned c, b, e in board order; a and d unpinned, listed in between.
  const board = [run("a", null), run("b", 2), run("c", 1), run("d", null), run("e", 3)];

  it("lands a pin between its new neighbours with one write", () => {
    expect(planPinDrop(board, "e", "b")).toEqual([{ id: "e", rank: 1.5 }]);
  });

  it("goes ahead of the first pin and behind the last", () => {
    expect(planPinDrop(board, "b", "c")).toEqual([{ id: "b", rank: 0 }]);
    expect(planPinDrop(board, "c", "e")).toEqual([{ id: "c", rank: 4 }]);
  });

  it("writes nothing for a drop on its own place", () => {
    expect(planPinDrop(board, "b", "b")).toEqual([]);
  });

  // The board refreshes while the pointer is down: a run unpinned from the
  // sidebar mid-drag must not be written back, whichever end of the drag it is.
  it("writes nothing when either end is no longer pinned", () => {
    expect(planPinDrop(board, "a", "b")).toEqual([]);
    expect(planPinDrop(board, "b", "d")).toEqual([]);
    expect(planPinDrop(board, "gone", "b")).toEqual([]);
  });

  it("renumbers every pin once the gap between two has collapsed", () => {
    const tight = [run("x", 1), run("y", 1 + 1e-7), run("z", 2)];
    expect(planPinDrop(tight, "z", "y")).toEqual([
      { id: "x", rank: 1 },
      { id: "z", rank: 2 },
      { id: "y", rank: 3 },
    ]);
  });
});

describe("pinDropSide", () => {
  const run = (id: string, pinRank: number | null): RunInfo =>
    ({ ...waitingRun(), id, pinRank }) as RunInfo;
  const board = [run("a", 1), run("b", 2), run("c", 3), run("d", null)];

  it("marks the side the dragged pin would land on", () => {
    expect(pinDropSide(board, "c", "a")).toBe("before");
    expect(pinDropSide(board, "a", "b")).toBe("after");
  });

  it("marks nothing over its own place or an unpinned run", () => {
    expect(pinDropSide(board, "b", "b")).toBeNull();
    expect(pinDropSide(board, "b", "d")).toBeNull();
  });
});
