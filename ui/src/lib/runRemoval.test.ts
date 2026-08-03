import { describe, expect, it } from "vitest";
import { removalCopy, removalLabel, removalsFor, RemovalRun } from "./runRemoval";

const agent: RemovalRun = { kind: "agent", agent: "claude", branch: "agent/foo", worktree: true };
const inCheckout: RemovalRun = { ...agent, worktree: false };
const terminal: RemovalRun = { kind: "terminal", agent: "shell", branch: "main", worktree: false };

describe("removalCopy", () => {
  it("names the branch the archive commits to", () => {
    const c = removalCopy(agent, "archive");
    expect(c.confirmLabel).toBe("Archive");
    expect(c.danger).toBe(false);
    expect(c.body).toContain("agent/foo");
  });

  it("promises nothing is committed when the run has no worktree", () => {
    expect(removalCopy(inCheckout, "archive").body).toContain("Nothing in your checkout");
    expect(removalCopy(inCheckout, "discard").body).toContain("left exactly as they are");
  });

  it("warns that discarding a worktree run cannot be undone", () => {
    const c = removalCopy(agent, "discard");
    expect(c.danger).toBe(true);
    expect(c.body).toContain("cannot be undone");
  });

  it("talks about the shell, not an agent, for a terminal", () => {
    const c = removalCopy(terminal, "discard");
    expect(c.title).toBe("Close terminal?");
    expect(c.confirmLabel).toBe("Close");
    expect(c.failTitle).toBe("Close failed");
  });
});

describe("removalsFor", () => {
  it("offers archive for agents only", () => {
    expect(removalsFor(agent)).toEqual(["archive", "discard"]);
    expect(removalsFor(terminal)).toEqual(["discard"]);
  });
});

describe("removalLabel", () => {
  it("matches the run's vocabulary", () => {
    expect(removalLabel(agent, "archive")).toBe("Archive agent");
    expect(removalLabel(agent, "discard")).toBe("Discard agent");
    expect(removalLabel(terminal, "discard")).toBe("Close terminal");
  });
});
