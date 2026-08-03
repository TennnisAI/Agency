import { describe, expect, it } from "vitest";
import { loadFocusTab, saveFocusTab, resolveFocusTab, PRIMARY_TAB } from "./focusTab";

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
});
