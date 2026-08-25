import { describe, expect, it } from "vitest";
import { RunInfo, RunStanding } from "../api";
import {
  fmtWake,
  inGitlessFolder,
  isPinned,
  needsAttention,
  pinnedFirst,
  runStatus,
  standingLabel,
} from "./runstate";

/** A live agent, quiet after a turn the user drove, with nothing said about it. */
function waitingRun(over: Partial<RunInfo> = {}): RunInfo {
  return {
    kind: "agent",
    status: { state: "running" },
    activity: { state: "waiting", since: 1_000 },
    attention: { standing: null, pinRank: null, needsAttention: true },
    ...over,
  } as RunInfo;
}

/** The same run with the user's word on it, as the backend would report it. */
function said(standing: RunStanding, pinRank: number | null = null): RunInfo {
  const suppressed = standing.kind !== "active";
  return waitingRun({ attention: { standing, pinRank, needsAttention: !suppressed } });
}

describe("inGitlessFolder", () => {
  it("is true only for a worktree-less run with no branch", () => {
    expect(inGitlessFolder({ worktree: false, branch: "" })).toBe(true);
  });

  it("is false for a run in a real checkout, which reports the live branch", () => {
    expect(inGitlessFolder({ worktree: false, branch: "main" })).toBe(false);
  });

  it("is false for a worktree run, which always has a branch of its own", () => {
    expect(inGitlessFolder({ worktree: true, branch: "agent/foo" })).toBe(false);
  });
});

describe("needsAttention", () => {
  it("is true for a waiting agent nobody has classified", () => {
    expect(needsAttention(waitingRun())).toBe(true);
  });

  it("is false once the run is settled or snoozed", () => {
    expect(needsAttention(said({ kind: "settled" }))).toBe(false);
    expect(needsAttention(said({ kind: "snoozed", untilMs: 9e12 }))).toBe(false);
  });

  it("stays true for a run the user marked active", () => {
    expect(needsAttention(said({ kind: "active" }))).toBe(true);
  });

  it("is false for a terminal, which is never waiting on you", () => {
    expect(needsAttention(waitingRun({ kind: "terminal" }))).toBe(false);
  });
});

describe("runStatus with a standing", () => {
  it("drops the amber and the climbing timer for a settled run", () => {
    const st = runStatus(said({ kind: "settled" }), 3_600_000);
    expect(st.cls).toBe("idle");
    expect(st.text).toBe("settled");
  });

  it("counts down to the wake time for a snoozed run", () => {
    const st = runStatus(said({ kind: "snoozed", untilMs: 10 * 60_000 }), 60_000);
    expect(st.cls).toBe("idle");
    expect(st.text).toBe("snoozed · 9m");
  });

  it("keeps the waiting badge for a run the user marked active", () => {
    const st = runStatus(said({ kind: "active" }), 3_600_000);
    expect(st.cls).toBe("awaiting");
    expect(st.text).toMatch(/^waiting · /);
  });

  it("leaves a working run alone whatever the user said about it", () => {
    const working = waitingRun({
      activity: { state: "working", since: 0 },
      attention: { standing: { kind: "settled" }, pinRank: null, needsAttention: false },
    });
    expect(runStatus(working, 5_000).cls).toBe("running");
  });
});

describe("standingLabel", () => {
  it("never counts a snooze below zero", () => {
    expect(standingLabel({ kind: "snoozed", untilMs: 1_000 }, 9_000)).toBe("snoozed · 0s");
  });
});

describe("fmtWake", () => {
  it("renders a clock time, not a date", () => {
    expect(fmtWake(new Date(2026, 7, 25, 14, 5).getTime())).toMatch(/\b2:05|\b14:05/);
  });
});

describe("pinnedFirst", () => {
  const run = (id: string, pinRank: number | null): RunInfo =>
    ({ ...waitingRun(), id, attention: { standing: null, pinRank, needsAttention: true } }) as RunInfo;

  it("puts pins first in pin order and leaves the rest as they came", () => {
    const list = [run("a", null), run("b", 2), run("c", null), run("d", 1)];
    expect(pinnedFirst(list).map((r) => r.id)).toEqual(["d", "b", "a", "c"]);
  });

  it("does not mutate the list it was given", () => {
    const list = [run("a", null), run("b", 1)];
    pinnedFirst(list);
    expect(list.map((r) => r.id)).toEqual(["a", "b"]);
  });

  it("reads a pin off the run whatever it is doing", () => {
    expect(isPinned(run("a", 0.5))).toBe(true);
    expect(isPinned(run("a", null))).toBe(false);
  });
});
