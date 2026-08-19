import { describe, expect, it } from "vitest";
import { RunLike, checkoutTooltip, describeCheckout } from "./checkout";

const run = (over: Partial<RunLike> & { id: string }): RunLike => ({
  kind: "agent",
  title: null,
  prompt: "",
  branch: "",
  agent: "claude",
  loopConfig: null,
  raceId: null,
  worktree: true,
  ...over,
});

const agent = run({ id: "r1", title: "Fix login", branch: "agent/fix-login" });
const inCheckout = run({ id: "r2", title: "Tidy up", worktree: false });
const term = run({ id: "t1", kind: "terminal", title: "shell", worktree: false });

type Args = Parameters<typeof describeCheckout>[0];

/** The project checkout of "Agency", with the three runs above to pick from. */
const at = (over: Partial<Args> = {}): ReturnType<typeof describeCheckout> =>
  describeCheckout({
    root: { kind: "project", id: "p1" },
    projectId: "p1",
    projectName: "Agency",
    runs: [agent, inCheckout, term],
    selectedRunId: null,
    ...over,
  });

describe("describeCheckout", () => {
  it("names the project checkout when no run owns the root", () => {
    const c = at({});
    expect(c).toMatchObject({ taskId: "project:p1", name: "Agency", kind: "checkout", note: null });
  });

  it("names the agent and its worktree", () => {
    const c = at({ root: { kind: "run", id: "r1" }, selectedRunId: "r1" });
    expect(c.kind).toBe("worktree");
    expect(c.taskId).toBe("r1");
    expect(c.name).toBe("claude: Fix login");
    expect(c.note).toBeNull();
  });

  it("reads a run without a worktree as the project checkout", () => {
    const c = at({ root: { kind: "run", id: "r2" }, selectedRunId: "r2" });
    expect(c).toMatchObject({ taskId: "project:p1", name: "Agency", kind: "checkout" });
    // The selected run *is* what's on show, so there is nothing to disclaim.
    expect(c.note).toBeNull();
  });

  it("treats a terminal like the checkout it runs in", () => {
    const c = at({ root: { kind: "run", id: "t1" }, selectedRunId: "t1" });
    expect(c.kind).toBe("checkout");
    expect(c.note).toBeNull();
  });

  it("says so when the checkout is shown while an agent worktree is selected", () => {
    const c = at({ selectedRunId: "r1" });
    expect(c.kind).toBe("checkout");
    expect(c.note).toBe("not the selected agent's worktree");
    expect(c.noteDetail).toContain("claude: Fix login");
  });

  it("keeps an unknown run a worktree rather than claiming the checkout", () => {
    const c = at({ root: { kind: "run", id: "gone" } });
    expect(c).toMatchObject({ taskId: "gone", kind: "worktree", name: "agent worktree" });
  });
});

describe("checkoutTooltip", () => {
  it("names the place and the branch", () => {
    expect(checkoutTooltip(at({}), "main"))
      .toBe("You are viewing the Agency checkout on branch main. Opens Source Control for this checkout.");
    expect(checkoutTooltip(at({ root: { kind: "run", id: "r1" } }), "agent/fix-login"))
      .toBe("You are viewing claude: Fix login's own worktree on branch agent/fix-login. Opens Source Control for this worktree.");
  });

  it("drops the branch clause when there is no git", () => {
    expect(checkoutTooltip(at({}), null))
      .toBe("You are viewing the Agency checkout. Opens Source Control for this checkout.");
  });

  it("appends the reason the selected agent isn't on show", () => {
    expect(checkoutTooltip(at({ selectedRunId: "r1" }), "main"))
      .toBe("You are viewing the Agency checkout on branch main. claude: Fix login has a worktree of its own; browse it from the Files tab. Opens Source Control for this checkout.");
  });
});
