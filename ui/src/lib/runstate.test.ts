import { describe, expect, it } from "vitest";
import { RunActivity, RunInfo, RunSessionInfo } from "../api";
import {
  agentNote,
  agentWaiting,
  agentWorking,
  countAgents,
  fmtDur,
  inGitlessFolder,
  isPinned,
  isWorking,
  matchesRunFilter,
  needsAttention,
  parseRunFilter,
  pinDropSide,
  pinIndex,
  pinnedFirst,
  projectAgentsLabel,
  runAgents,
  runStatus,
} from "./runstate";

/** A live agent, quiet after a turn the user drove. */
const waitingRun = (over: Partial<RunInfo> = {}): RunInfo =>
  ({
    kind: "agent",
    status: { state: "running" },
    activity: { state: "waiting", since: 0 },
    pinRank: null,
    primaryClosed: false,
    sessions: [],
    ...over,
  }) as RunInfo;

/** An extra tab in run r1's workspace, up and in the given state. */
const tab = (n: number, state: RunActivity["state"] | null, agent = "codex"): RunSessionInfo => ({
  id: `r1--${n}`,
  runId: "r1",
  agent,
  status: { state: "running" },
  activity: state && { state, since: 0 },
});

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

describe("pinIndex", () => {
  const run = (id: string, pinRank: number | null): RunInfo =>
    ({ ...waitingRun(), id, pinRank }) as RunInfo;

  it("numbers the pins in board order and skips the rest", () => {
    const board = pinnedFirst([run("a", null), run("b", 2), run("c", 1), run("d", null)]);
    expect([...pinIndex(board)]).toEqual([["c", 0], ["b", 1]]);
  });
});

describe("pinDropSide", () => {
  const run = (id: string, pinRank: number | null): RunInfo =>
    ({ ...waitingRun(), id, pinRank }) as RunInfo;
  const index = pinIndex([run("a", 1), run("b", 2), run("c", 3), run("d", null)]);

  it("marks the side the dragged pin would land on", () => {
    expect(pinDropSide(index, "c", "a")).toBe("before");
    expect(pinDropSide(index, "a", "b")).toBe("after");
  });

  it("marks nothing over its own place or an unpinned run", () => {
    expect(pinDropSide(index, "b", "b")).toBeNull();
    expect(pinDropSide(index, "b", "d")).toBeNull();
  });
});

// The overview counted runs, so a workspace read as whatever its first agent
// was doing and every other tab in it went uncounted (AGE-249).
describe("runAgents", () => {
  it("counts every agent tab in a workspace, not just the first", () => {
    const r = waitingRun({ sessions: [tab(2, "working"), tab(3, "waiting"), tab(4, "idle")] });
    expect(runAgents(r)).toHaveLength(4);
    expect(countAgents([r], agentWorking)).toBe(1);
    expect(countAgents([r], agentWaiting)).toBe(2);
  });

  it("finds a waiting agent behind a working first one", () => {
    const r = waitingRun({ activity: { state: "working", since: 0 }, sessions: [tab(2, "waiting")] });
    expect(needsAttention(r)).toBe(true);
    expect(isWorking(r)).toBe(true);
  });

  it("counts a freshly opened tab, not yet sampled, as working", () => {
    expect(countAgents([waitingRun({ sessions: [tab(2, null)] })], agentWorking)).toBe(1);
  });

  it("does not count a terminal tab as an agent", () => {
    const r = waitingRun({ sessions: [tab(2, "waiting", "shell")] });
    expect(runAgents(r)).toHaveLength(1);
  });

  it("does not count a tab whose agent has stopped, whatever its last sample", () => {
    const stopped = { ...tab(2, "waiting"), status: { state: "exited", code: 0 } } as RunSessionInfo;
    const r = waitingRun({ activity: { state: "idle", since: 0 }, sessions: [stopped] });
    expect(countAgents([r])).toBe(2);
    expect(needsAttention(r)).toBe(false);
  });

  // With the first tab closed, the run's own status and activity describe the
  // tab standing in for it (AGE-184). Counting both would count that tab twice.
  it("counts a closed first tab's stand-in once", () => {
    const r = waitingRun({ primaryClosed: true, sessions: [tab(2, "waiting")] });
    expect(runAgents(r)).toHaveLength(1);
    expect(countAgents([r], agentWaiting)).toBe(1);
  });

  it("has no agents in a terminal run", () => {
    expect(runAgents(waitingRun({ kind: "terminal" }))).toEqual([]);
  });

  it("sums across runs", () => {
    const runs = [waitingRun(), waitingRun({ sessions: [tab(2, "waiting")] })];
    expect(countAgents(runs)).toBe(3);
    expect(countAgents(runs, agentWaiting)).toBe(3);
  });
});

describe("projectAgentsLabel", () => {
  it("counts every agent tab", () => {
    expect(projectAgentsLabel([waitingRun({ sessions: [tab(2, "working")] })])).toBe("2 agents");
    expect(projectAgentsLabel([waitingRun(), waitingRun({ kind: "terminal" })])).toBe("1 agent");
  });

  it("names a project of terminal runs by its terminals", () => {
    const t = waitingRun({ kind: "terminal" });
    expect(projectAgentsLabel([t])).toBe("1 terminal");
    expect(projectAgentsLabel([t, t])).toBe("2 terminals");
  });

  // An agent run whose first tab was closed, with only terminal tabs left, is
  // still an agent run: it used to be counted among the terminals.
  it("does not call an agent run with only terminal tabs left a terminal", () => {
    const r = waitingRun({ primaryClosed: true, sessions: [tab(2, null, "shell")] });
    expect(projectAgentsLabel([r])).toBe("no agents");
  });

  it("says so when there is nothing at all", () => {
    expect(projectAgentsLabel([])).toBe("no agents");
  });
});

describe("matchesRunFilter", () => {
  const working = waitingRun({ activity: { state: "working", since: 0 } } as Partial<RunInfo>);
  const waiting = waitingRun();
  const exited = waitingRun({ status: { state: "exited", code: 0 } } as Partial<RunInfo>);
  const terminal = waitingRun({ kind: "terminal" } as Partial<RunInfo>);

  it("keeps every run under all", () => {
    for (const r of [working, waiting, exited, terminal]) expect(matchesRunFilter(r, "all")).toBe(true);
  });

  it("keeps only the runs each header count counts", () => {
    expect([working, waiting, exited, terminal].filter((r) => matchesRunFilter(r, "working"))).toEqual([working]);
    expect([working, waiting, exited, terminal].filter((r) => matchesRunFilter(r, "waiting"))).toEqual([waiting]);
  });

  it("keeps a workspace for an agent in any of its tabs", () => {
    const mixed = waitingRun({ activity: { state: "idle", since: 0 }, sessions: [tab(2, "working")] });
    expect(matchesRunFilter(mixed, "working")).toBe(true);
    expect(matchesRunFilter(mixed, "waiting")).toBe(false);
  });
});

describe("parseRunFilter", () => {
  it("reads back a stored filter and falls back to all for anything else", () => {
    expect(parseRunFilter("working")).toBe("working");
    expect(parseRunFilter("waiting")).toBe("waiting");
    expect(parseRunFilter(null)).toBe("all");
    expect(parseRunFilter("exited")).toBe("all");
  });
});
