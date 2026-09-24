import { describe, expect, it } from "vitest";
import { checkoutStatus, checkoutStatusTooltip } from "./checkoutStatus";

const info = (over: Partial<Parameters<typeof checkoutStatus>[0]> = {}) => ({
  branch: "main",
  upstream: "origin/main",
  ahead: 0,
  behind: 0,
  hasRemote: true,
  ...over,
});

describe("checkoutStatus", () => {
  it("carries both counts for a tracked branch", () => {
    expect(checkoutStatus(info({ ahead: 2, behind: 3 }))).toEqual({
      kind: "tracked", branch: "main", upstream: "origin/main", ahead: 2, behind: 3,
    });
  });

  it("drops the fork-point count when the branch tracks nothing", () => {
    // branch_info counts from the fork point (or all history) without an
    // upstream; showing that as "to push" would misstate what origin has.
    expect(checkoutStatus(info({ upstream: null, ahead: 40 }))).toEqual({
      kind: "unpublished", branch: "main",
    });
  });

  it("says local when there is no origin at all", () => {
    expect(checkoutStatus(info({ upstream: null, hasRemote: false, ahead: 12 }))).toEqual({
      kind: "local", branch: "main",
    });
  });

  it("names a detached HEAD rather than a branch called HEAD", () => {
    expect(checkoutStatus(info({ branch: "HEAD", upstream: null })).branch).toBe("detached HEAD");
  });
});

describe("checkoutStatusTooltip", () => {
  const tip = (over: Parameters<typeof info>[0]) => checkoutStatusTooltip(checkoutStatus(info(over)));

  it("says in sync when there is nothing either way", () => {
    expect(tip({})).toBe(
      "Your checkout is on main. In sync with origin/main, as of the last fetch. "
        + "Opens Source Control for your checkout.",
    );
  });

  it("counts each direction, singular and plural", () => {
    expect(tip({ ahead: 1, behind: 4 })).toContain("1 commit to push, 4 commits to pull against origin/main");
    expect(tip({ behind: 1 })).toContain("1 commit to pull against origin/main");
    expect(tip({ ahead: 2 })).not.toContain("pull");
  });

  it("explains an unpublished branch and a remoteless checkout", () => {
    expect(tip({ upstream: null })).toContain("never been pushed to origin");
    expect(tip({ upstream: null, hasRemote: false })).toContain("no remote");
  });

  it("uses no em dashes", () => {
    for (const over of [{}, { ahead: 1 }, { upstream: null }, { upstream: null, hasRemote: false }]) {
      expect(tip(over)).not.toContain("—");
    }
  });
});
