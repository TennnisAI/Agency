import { describe, expect, it } from "vitest";
import { PopoutEntry, PopoutTarget, findPopout, isPopoutLabel, popoutForSession, popoutTitle, sameTarget } from "./popout";

const run = (runId: string): PopoutTarget => ({ kind: "run", projectId: "p1", runId });
const file = (path: string, root: "project" | "run" = "project"): PopoutTarget =>
  ({ kind: "file", projectId: "p1", root: { kind: root, id: root === "run" ? "r1" : "p1" }, path });
const note = (path: string): PopoutTarget =>
  ({ kind: "note", projectId: "p1", root: { kind: "project", id: "p1" }, path });

describe("popout targets", () => {
  it("match on what they show, not on how they were reached", () => {
    expect(sameTarget(run("r1"), { kind: "run", projectId: "other", runId: "r1" })).toBe(true);
    expect(sameTarget(run("r1"), run("r2"))).toBe(false);
    expect(sameTarget(file("a.ts"), file("a.ts"))).toBe(true);
    // The same path in a worktree is a different file from the checkout's.
    expect(sameTarget(file("a.ts"), file("a.ts", "run"))).toBe(false);
    // A note and a file never match, even at one path.
    expect(sameTarget(file("a.md"), note("a.md"))).toBe(false);
  });

  it("find the window an item is in", () => {
    const list: PopoutEntry[] = [
      { label: "popout-1", target: run("r1") },
      { label: "popout-2", target: note("Daily/2026-10-04.md") },
    ];
    expect(findPopout(list, run("r1"))?.label).toBe("popout-1");
    expect(findPopout(list, note("Daily/2026-10-04.md"))?.label).toBe("popout-2");
    expect(findPopout(list, run("r9"))).toBeNull();
  });

  it("count a run's extra tabs as out with the run", () => {
    const list: PopoutEntry[] = [{ label: "popout-1", target: run("r1") }];
    expect(popoutForSession(list, "r1")?.label).toBe("popout-1");
    expect(popoutForSession(list, "r1--2")?.label).toBe("popout-1");
    expect(popoutForSession(list, "r10--2")).toBeNull();
  });

  it("know their own windows by label", () => {
    expect(isPopoutLabel("popout-00ff")).toBe(true);
    expect(isPopoutLabel("main")).toBe(false);
    expect(isPopoutLabel(null)).toBe(false);
  });

  it("title a window after what it shows", () => {
    expect(popoutTitle(file("src/lib/a.ts"))).toBe("a.ts");
    expect(popoutTitle(note("Projects/Launch plan.md"))).toBe("Launch plan");
    expect(popoutTitle(run("r1"), "claude · fix login")).toBe("claude · fix login");
    expect(popoutTitle(run("r1"), "  ")).toBe("Agent");
  });
});
