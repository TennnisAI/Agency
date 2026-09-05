import { describe, expect, it } from "vitest";
import { agentViewTab, loadFocusTab, saveFocusTab, resolveFocusTab, PRIMARY_TAB, RUN_TAB, LOG_TAB } from "./focusTab";

function makeStorage(): Pick<Storage, "getItem" | "setItem"> {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { map.set(k, v); },
  };
}

const running = (id: string) => ({ id, status: { state: "running" } });

describe("focus tab memory", () => {
  it("defaults to the primary agent for a run never visited", () => {
    expect(loadFocusTab(makeStorage(), "run-1")).toBe(PRIMARY_TAB);
  });

  it("round-trips a remembered tab", () => {
    const storage = makeStorage();
    saveFocusTab(storage, "run-1", "run-1--2");
    expect(loadFocusTab(storage, "run-1")).toBe("run-1--2");
  });

  it("remembers each run separately", () => {
    const storage = makeStorage();
    saveFocusTab(storage, "run-1", "run-1--2");
    saveFocusTab(storage, "run-2", "run-2--3");
    expect(loadFocusTab(storage, "run-1")).toBe("run-1--2");
    expect(loadFocusTab(storage, "run-2")).toBe("run-2--3");
  });

  it("survives a storage that throws", () => {
    const broken = {
      getItem() { throw new Error("denied"); },
      setItem() { throw new Error("denied"); },
    };
    expect(() => saveFocusTab(broken, "run-1", "run-1--2")).not.toThrow();
    expect(loadFocusTab(broken, "run-1")).toBe(PRIMARY_TAB);
  });
});

describe("resolveFocusTab", () => {
  it("keeps a remembered session that is still running", () => {
    expect(resolveFocusTab("run-1--2", [running("run-1--2")])).toBe("run-1--2");
  });

  it("keeps a remembered session that exited, matching the tab strip", () => {
    const sessions = [{ id: "run-1--2", status: { state: "exited", code: 0 } }];
    expect(resolveFocusTab("run-1--2", sessions)).toBe("run-1--2");
  });

  it("falls back when the remembered session is gone", () => {
    const sessions = [{ id: "run-1--2", status: { state: "gone" } }];
    expect(resolveFocusTab("run-1--2", sessions)).toBe(PRIMARY_TAB);
  });

  it("falls back when the remembered session no longer exists", () => {
    expect(resolveFocusTab("run-1--2", [running("run-1--3")])).toBe(PRIMARY_TAB);
  });

  it("passes the built-in tabs through without a session", () => {
    expect(resolveFocusTab(PRIMARY_TAB, [])).toBe(PRIMARY_TAB);
    expect(resolveFocusTab("run", [])).toBe("run");
  });

  // AGE-184: with the run's own tab closed the strip does not draw it, so
  // falling back to it would strand an empty pane with no way out.
  it("falls back to the leftmost live tab when the primary is closed", () => {
    const sessions = [running("run-1--2"), running("run-1--3")];
    expect(resolveFocusTab(PRIMARY_TAB, sessions, false, true)).toBe("run-1--2");
    expect(resolveFocusTab("run-1--9", sessions, false, true)).toBe("run-1--2");
    expect(resolveFocusTab(LOG_TAB, sessions, false, true)).toBe("run-1--2");
    // A tab that is merely exited still renders, so it is still a landing spot.
    const exited = [{ id: "run-1--2", status: { state: "exited", code: 0 } }];
    expect(resolveFocusTab(PRIMARY_TAB, exited, false, true)).toBe("run-1--2");
  });

  it("keeps the primary as the last resort when nothing is left to draw", () => {
    // The strip draws it again the moment the run has an agent of its own,
    // and a tab id nothing renders would be worse than a dead pane.
    expect(resolveFocusTab(PRIMARY_TAB, [], false, true)).toBe(PRIMARY_TAB);
    const gone = [{ id: "run-1--2", status: { state: "gone" } }];
    expect(resolveFocusTab(PRIMARY_TAB, gone, false, true)).toBe(PRIMARY_TAB);
  });

  it("leaves the run tab alone whatever the primary is doing", () => {
    expect(resolveFocusTab(RUN_TAB, [running("run-1--2")], false, true)).toBe(RUN_TAB);
  });

  it("keeps the log tab only while the run still serves a GUI", () => {
    expect(resolveFocusTab(LOG_TAB, [], true)).toBe(LOG_TAB);
    // The web session was closed, or a loop took the run over: the strip stops
    // drawing the tab, so restoring onto it would strand an empty pane.
    expect(resolveFocusTab(LOG_TAB, [], false)).toBe(PRIMARY_TAB);
    expect(resolveFocusTab(LOG_TAB, [])).toBe(PRIMARY_TAB);
  });
});

describe("agentViewTab", () => {
  it("leaves the run tab for the tab it was opened from", () => {
    expect(agentViewTab(RUN_TAB, PRIMARY_TAB)).toBe(PRIMARY_TAB);
    expect(agentViewTab(RUN_TAB, "run-1--2")).toBe("run-1--2");
    expect(agentViewTab(RUN_TAB, LOG_TAB)).toBe(LOG_TAB);
  });

  it("falls back to the primary agent when the run tab is all it knows", () => {
    // A run restored straight onto Run remembers no tab before it.
    expect(agentViewTab(RUN_TAB, RUN_TAB)).toBe(PRIMARY_TAB);
  });

  it("leaves every other tab alone", () => {
    // An extra agent tab is the agent view too, so a name click stays on it
    // rather than dropping back to the primary agent.
    expect(agentViewTab("run-1--2", PRIMARY_TAB)).toBe("run-1--2");
    expect(agentViewTab(PRIMARY_TAB, "run-1--2")).toBe(PRIMARY_TAB);
    expect(agentViewTab(LOG_TAB, PRIMARY_TAB)).toBe(LOG_TAB);
  });
});
