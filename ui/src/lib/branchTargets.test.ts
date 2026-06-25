import { describe, it, expect } from "vitest";
import { effectiveMergeTarget } from "./branchTargets";

describe("effectiveMergeTarget", () => {
  it("follows the base when there is no override", () => {
    expect(effectiveMergeTarget("main", null)).toBe("main");
    expect(effectiveMergeTarget("develop", null)).toBe("develop");
  });

  it("uses the override when set", () => {
    expect(effectiveMergeTarget("main", "release")).toBe("release");
  });
});
