import { describe, expect, it } from "vitest";
import { RunInfo, RunSessionInfo } from "../api";
import { PRIMARY_TAB } from "./focusTab";
import { runTabs, showsTabs, tabCountLabel, tabLabel, tabsTitle } from "./runTabs";

const session = (id: string, agent: string, status: RunSessionInfo["status"] = { state: "running" }) =>
  ({ id, runId: id.split("--")[0], agent, status }) as RunSessionInfo;

const run = (over: Partial<RunInfo> = {}): RunInfo =>
  ({
    id: "r1",
    kind: "agent",
    agent: "claude",
    status: { state: "running" },
    activity: { state: "waiting", since: 0 },
    primaryClosed: false,
    sessions: [],
    ...over,
  }) as RunInfo;

describe("runTabs", () => {
  it("is the run's own agent alone when it has no extra tabs", () => {
    const tabs = runTabs(run(), 0);
    expect(tabs).toHaveLength(1);
    expect(tabs[0]).toMatchObject({ panel: PRIMARY_TAB, session: "r1", label: "Claude Code" });
    expect(showsTabs(run(), tabs)).toBe(false);
  });

  it("lists every tab in strip order, the run's own agent first", () => {
    const r = run({ sessions: [session("r1--2", "codex"), session("r1--3", "shell")] });
    const tabs = runTabs(r, 0);
    expect(tabs.map((t) => t.label)).toEqual(["Claude Code", "Codex · 2", "≳ terminal · 3"]);
    // The primary opens by the run's id, an extra tab by its own.
    expect(tabs.map((t) => t.session)).toEqual(["r1", "r1--2", "r1--3"]);
    expect(tabs.map((t) => t.panel)).toEqual([PRIMARY_TAB, "r1--2", "r1--3"]);
    expect(showsTabs(r, tabs)).toBe(true);
  });

  // AGE-184 closed the first agent's tab without renaming the run, so its row
  // went on naming an agent that was gone. One tab left is still worth listing
  // when it is not the one the row is named after.
  it("drops a closed primary, and still lists a lone tab that stands in for it", () => {
    const r = run({ primaryClosed: true, sessions: [session("r1--2", "codex")] });
    const tabs = runTabs(r, 0);
    expect(tabs.map((t) => t.label)).toEqual(["Codex · 2"]);
    expect(showsTabs(r, tabs)).toBe(true);
  });

  it("has nothing to list for a terminal, which has no strip", () => {
    const r = run({ kind: "terminal", sessions: [] });
    expect(runTabs(r, 0)).toEqual([]);
    expect(showsTabs(r, [])).toBe(false);
  });

  it("gives the primary the run's own dot, activity included", () => {
    expect(runTabs(run(), 0)[0].cls).toBe("awaiting");
  });

  // Activity is sampled for the lead session only, so an extra tab that is up
  // must not pulse as though someone had watched it working.
  it("gives an extra tab a still dot from its session status alone", () => {
    const r = run({
      sessions: [
        session("r1--2", "codex"),
        session("r1--3", "codex", { state: "exited", code: 0 }),
        session("r1--4", "codex", { state: "exited", code: 1 }),
        session("r1--5", "codex", { state: "gone" }),
      ],
    });
    const extra = runTabs(r, 0).slice(1);
    expect(extra.map((t) => t.cls)).toEqual(["live", "exited", "exited", "exited"]);
    expect(extra.map((t) => t.status)).toEqual(["running", "finished", "failed", "not running"]);
  });
});

describe("tab labels", () => {
  it("matches what the strip draws", () => {
    expect(tabLabel("claude", "r1", true)).toBe("Claude Code");
    expect(tabLabel("claude", "r1--2", false)).toBe("Claude Code · 2");
    expect(tabLabel("shell", "r1--3", false)).toBe("≳ terminal · 3");
    // A custom profile keeps the name the user gave it.
    expect(tabLabel("mine", "r1--4", false)).toBe("mine · 4");
  });

  it("counts and names the tabs for a title", () => {
    expect(tabCountLabel(1)).toBe("1 tab");
    expect(tabCountLabel(3)).toBe("3 tabs");
    const tabs = runTabs(run({ sessions: [session("r1--2", "codex")] }), 0);
    expect(tabsTitle(tabs)).toBe("2 tabs in this workspace: Claude Code, Codex · 2");
  });
});
