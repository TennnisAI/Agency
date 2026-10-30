import { describe, expect, it } from "vitest";
import { RunActivity, RunInfo, RunSessionInfo } from "../api";
import { PRIMARY_TAB } from "./focusTab";
import { runStatus } from "./runstate";
import {
  restorableRun,
  runTabs,
  showsTabs,
  sidePanelTab,
  tabCountLabel,
  tabLabel,
  tabsTitle,
  waitingElsewhere,
  waitingElsewhereLabel,
  workspaceStatus,
} from "./runTabs";

const session = (
  id: string,
  agent: string,
  status: RunSessionInfo["status"] = { state: "running" },
  activity: RunSessionInfo["activity"] = null,
) => ({ id, runId: id.split("--")[0], agent, status, activity }) as RunSessionInfo;

const run = (over: Partial<RunInfo> = {}): RunInfo =>
  ({
    id: "r1",
    kind: "agent",
    agent: "claude",
    status: { state: "running" },
    activity: { state: "done", since: 0, reported: false },
    primaryClosed: false,
    sessions: [],
    ...over,
  }) as RunInfo;

describe("restorableRun", () => {
  const runs = [run({ id: "a" }), run({ id: "b" })];
  const primary = () => PRIMARY_TAB;

  it("goes back to the remembered run when nothing is focused", () => {
    expect(restorableRun("a", runs, null, primary)).toBe("a");
  });

  it("leaves a focused run alone, even one that differs from the memory", () => {
    expect(restorableRun("a", runs, "b", primary)).toBeNull();
  });

  it("has nothing to restore without a memory", () => {
    expect(restorableRun(null, runs, null, primary)).toBeNull();
  });

  it("waits while the list still holds the project just left", () => {
    expect(restorableRun("c", runs, null, primary)).toBeNull();
    expect(restorableRun("c", [...runs, run({ id: "c" })], null, primary)).toBe("c");
  });

  it("does not bring back a run removed while away", () => {
    expect(restorableRun("removed", runs, null, primary)).toBeNull();
  });

  it("does not respawn an agent whose session went while away", () => {
    const gone = run({ status: { state: "gone" } });
    expect(restorableRun("r1", [gone], null, primary)).toBeNull();
  });

  it("does not respawn a terminal whose session went while away", () => {
    const gone = run({ kind: "terminal", status: { state: "gone" } });
    expect(restorableRun("r1", [gone], null, primary)).toBeNull();
  });

  it("restores an agent that exited but still has its session", () => {
    // Only a gone session is respawned; an exited one just shows how it ended.
    const exited = run({ status: { state: "exited", code: 0 } });
    expect(restorableRun("r1", [exited], null, primary)).toBe("r1");
  });

  it("goes by the tab the panel would show, not the run's own agent", () => {
    const r = run({
      status: { state: "gone" },
      sessions: [session("r1--2", "codex"), session("r1--3", "codex", { state: "gone" })],
    });
    expect(restorableRun("r1", [r], null, () => "r1--2")).toBe("r1");
    expect(restorableRun("r1", [r], null, () => "r1--3")).toBeNull();
    expect(restorableRun("r1", [r], null, primary)).toBeNull();
  });

  it("restores a run with no tab left, which has nothing to respawn", () => {
    const empty = run({ status: { state: "gone" }, primaryClosed: true });
    expect(restorableRun("r1", [empty], null, primary)).toBe("r1");
  });
});

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

  // Each tab's activity is sampled on its own (AGE-249), and the overview
  // counts from it, so the tab's dot says the same thing the counts do.
  it("gives an extra agent tab its own activity", () => {
    const r = run({
      sessions: [
        session("r1--2", "codex", { state: "running" }, { state: "done", since: 0, reported: false }),
        session("r1--3", "codex", { state: "running" }, { state: "idle", since: 0, reported: false }),
        session("r1--4", "codex"),
      ],
    });
    const extra = runTabs(r, 0).slice(1);
    expect(extra.map((t) => t.cls)).toEqual(["awaiting", "idle", "running"]);
    expect(extra.map((t) => t.status)).toEqual(["waiting · 0s", "idle", "working"]);
  });

  it("gives a stopped tab, or a terminal tab, a still dot from its status alone", () => {
    const r = run({
      sessions: [
        session("r1--2", "shell", { state: "running" }, { state: "done", since: 0, reported: false }),
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

// Clicking "1 waiting" kept a tile whose dot said "working", and nothing on the
// tile said which of its tabs was the one asking (AGE-249).
describe("waitingElsewhere", () => {
  const waiting = { state: "running" } as const;
  const elsewhere = (r: RunInfo) => waitingElsewhere(runTabs(r, 0), workspaceStatus(r, 0).cls);

  it("names a waiting tab behind a working first agent", () => {
    const r = run({
      activity: { state: "working", since: 0, reported: false },
      sessions: [session("r1--2", "codex", waiting, { state: "done", since: 0, reported: false })],
    });
    expect(elsewhere(r).map((t) => t.label)).toEqual(["Codex · 2"]);
    expect(waitingElsewhereLabel(elsewhere(r))).toBe("1 other tab waiting");
  });

  it("leaves out the tab the dot already shows waiting", () => {
    const r = run({
      sessions: [
        session("r1--2", "codex", waiting, { state: "done", since: 0, reported: false }),
        session("r1--3", "codex", waiting, { state: "done", since: 0, reported: false }),
      ],
    });
    expect(elsewhere(r).map((t) => t.session)).toEqual(["r1--2", "r1--3"]);
    expect(waitingElsewhereLabel(elsewhere(r))).toBe("2 other tabs waiting");
  });

  it("is nothing when only the dot's own tab is waiting", () => {
    const r = run({ sessions: [session("r1--2", "codex", waiting, { state: "idle", since: 0, reported: false })] });
    expect(elsewhere(r)).toEqual([]);
    expect(waitingElsewhereLabel(elsewhere(r))).toBeNull();
  });

  // With the first tab closed its row is gone (AGE-184), and an exited tab
  // ahead of the waiting ones is not the one the dot shows.
  it("takes the first waiting tab as the dot's own when the first tab is closed", () => {
    const r = run({
      primaryClosed: true,
      sessions: [
        session("r1--2", "codex", { state: "exited", code: 0 }),
        session("r1--3", "codex", waiting, { state: "done", since: 0, reported: false }),
        session("r1--4", "codex", waiting, { state: "done", since: 0, reported: false }),
      ],
    });
    expect(elsewhere(r).map((t) => t.session)).toEqual(["r1--4"]);
  });

  it("lists every waiting tab when the dot reads working", () => {
    const r = run({
      primaryClosed: true,
      sessions: [
        session("r1--2", "codex", waiting, { state: "done", since: 0, reported: false }),
        session("r1--3", "codex", waiting, { state: "working", since: 0, reported: false }),
        session("r1--4", "codex", waiting, { state: "done", since: 0, reported: false }),
      ],
    });
    expect(elsewhere(r).map((t) => t.session)).toEqual(["r1--2", "r1--4"]);
  });

  it("names a blocked tab as blocked, not waiting", () => {
    const blocked = { state: "blocked", since: 0, reported: true } as const;
    const done = { state: "done", since: 0, reported: false } as const;
    const working = { state: "working", since: 0, reported: false } as const;
    const one = run({
      activity: blocked,
      sessions: [session("r1--2", "claude", waiting, blocked), session("r1--3", "codex", waiting, working)],
    });
    expect(waitingElsewhereLabel(elsewhere(one))).toBe("1 other tab blocked");
    const both = run({
      activity: blocked,
      sessions: [session("r1--2", "claude", waiting, blocked), session("r1--3", "codex", waiting, done)],
    });
    expect(waitingElsewhereLabel(elsewhere(both))).toBe("2 other tabs: 1 blocked, 1 waiting");
  });
});

// A worktree whose first agent had finished read "waiting" on its tile, rail
// row and tree row while a second agent in it was still working.
describe("workspaceStatus", () => {
  const live = (n: number, state: RunActivity["state"] | null, agent = "codex") =>
    session(`r1--${n}`, agent, { state: "running" }, state && { state, since: 0, reported: false });

  it("reads working when any agent is working, whatever the first one says", () => {
    const st = workspaceStatus(run({ sessions: [live(2, "idle"), live(3, "working")] }), 0);
    expect(st.cls).toBe("running");
    expect(st.text).toBe("working · 0s");
  });

  it("counts a freshly spawned tab with no activity yet as working", () => {
    expect(workspaceStatus(run({ sessions: [live(2, null)] }), 0).cls).toBe("running");
  });

  it("reads blocked over working: that agent cannot go on without you", () => {
    const r = run({ activity: { state: "working", since: 0, reported: false }, sessions: [live(2, "blocked")] });
    expect(workspaceStatus(r, 0).cls).toBe("blocked");
  });

  it("reads waiting only when no agent is working or blocked", () => {
    const r = run({ activity: { state: "idle", since: 0, reported: false }, sessions: [live(2, "done")] });
    expect(workspaceStatus(r, 0).cls).toBe("awaiting");
  });

  it("takes the earliest tab on a tie, for its timer", () => {
    const r = run({ activity: { state: "done", since: 5_000, reported: true }, sessions: [live(2, "done")] });
    expect(workspaceStatus(r, 60_000).text).toBe("done · 55s");
  });

  it("ignores a closed first agent, and a terminal tab", () => {
    const r = run({
      primaryClosed: true,
      activity: { state: "working", since: 0, reported: false },
      sessions: [live(2, "done"), live(3, "working", "shell")],
    });
    expect(workspaceStatus(r, 0).cls).toBe("awaiting");
  });

  it("reads a live extra agent over a first agent that exited", () => {
    const r = run({ status: { state: "exited", code: 0 }, sessions: [live(2, "working")] });
    expect(workspaceStatus(r, 0).cls).toBe("running");
  });

  it("keeps a terminal run's own status", () => {
    const r = run({ kind: "terminal", activity: null });
    expect(workspaceStatus(r, 0)).toEqual(runStatus(r, 0));
  });

  // A tile named "Claude Code" explained a quiet Codex tab beside it as "this
  // agent doesn't report which".
  it("names the tab its title is about once the row spells tabs out", () => {
    const r = run({ activity: { state: "idle", since: 0, reported: false }, sessions: [live(2, "done")] });
    expect(workspaceStatus(r, 0).title).toBe(`Codex · 2\n${runStatus(run(), 0).title}`);
    const working = run({ activity: { state: "working", since: 0, reported: false }, sessions: [live(2, "idle")] });
    expect(workspaceStatus(working, 0).title).toBe("Claude Code");
  });

  it("leaves a lone agent's title as it is", () => {
    expect(workspaceStatus(run(), 0).title).toBe(runStatus(run(), 0).title);
  });

  // The run's own status stands for its lowest-numbered live session once the
  // first tab is closed (AGE-184), and with only terminals left that is a shell.
  it("reads a workspace holding only terminals as live, not working", () => {
    const r = run({ primaryClosed: true, activity: null, sessions: [live(2, null, "shell")] });
    expect(workspaceStatus(r, 0)).toMatchObject({ cls: "live", text: "running" });
    const ended = run({ primaryClosed: true, sessions: [session("r1--2", "shell", { state: "exited", code: 0 })] });
    expect(workspaceStatus(ended, 0)).toEqual({ cls: "exited", text: "session ended" });
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

// The Docs and Files side panel attached the run's own session whatever tab
// the workspace was on, so an extra tab was out of its reach and a closed first
// tab (AGE-184) left it on a dead pane (AGE-226).
describe("sidePanelTab", () => {
  const r = run({ sessions: [session("r1--2", "codex"), session("r1--3", "shell")] });

  it("follows the tab the focus view showed or remembers", () => {
    expect(sidePanelTab(r, "r1--3", 0)).toMatchObject({ session: "r1--3", agent: "shell" });
    expect(sidePanelTab(r, PRIMARY_TAB, 0)).toMatchObject({ session: "r1", agent: "claude" });
  });

  it("falls back to the first drawn tab when that tab is not an agent or is gone", () => {
    expect(sidePanelTab(r, "run", 0)?.session).toBe("r1");
    expect(sidePanelTab(r, "r1--9", 0)?.session).toBe("r1");
    expect(sidePanelTab(r, null, 0)?.session).toBe("r1");
  });

  it("never attaches a closed primary, and lands on the tab standing in for it", () => {
    const closed = run({ primaryClosed: true, sessions: [session("r1--2", "codex")] });
    expect(sidePanelTab(closed, PRIMARY_TAB, 0)?.session).toBe("r1--2");
    expect(sidePanelTab(closed, null, 0)?.session).toBe("r1--2");
    expect(sidePanelTab(run({ primaryClosed: true }), null, 0)).toBeNull();
  });

  it("attaches a terminal's one session as its own tab", () => {
    const t = run({ id: "t1", kind: "terminal", agent: "shell", title: "build" });
    expect(sidePanelTab(t, "whatever", 0)).toMatchObject({ session: "t1", agent: "shell", panel: PRIMARY_TAB });
  });
});
