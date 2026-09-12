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
  model: null,
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
    ...over,
  });

describe("describeCheckout", () => {
  it("names the project checkout when no run owns the root", () => {
    expect(at({})).toEqual({ taskId: "project:p1", name: "Agency", kind: "checkout" });
  });

  it("names the agent and its worktree", () => {
    expect(at({ root: { kind: "run", id: "r1" } }))
      .toEqual({ taskId: "r1", name: "claude: Fix login", kind: "worktree" });
  });

  it("reads a run without a worktree as the project checkout", () => {
    expect(at({ root: { kind: "run", id: "r2" } }))
      .toEqual({ taskId: "project:p1", name: "Agency", kind: "checkout" });
  });

  it("treats a terminal like the checkout it runs in", () => {
    expect(at({ root: { kind: "run", id: "t1" } }).kind).toBe("checkout");
  });

  it("keeps an unknown run a worktree rather than claiming the checkout", () => {
    const c = at({ root: { kind: "run", id: "gone" } });
    expect(c).toEqual({ taskId: "gone", kind: "worktree", name: "agent worktree" });
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
});
