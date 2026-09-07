import { describe, expect, it } from "vitest";
import { openingTab } from "./projectTab";

describe("openingTab", () => {
  it("opens the workspace on its docs vault", () => {
    expect(openingTab("workspace")).toBe("docs");
  });

  it("opens a repo project on the agents grid", () => {
    expect(openingTab(null)).toBe("agents");
    expect(openingTab(undefined)).toBe("agents");
  });

  // A kind added later must not silently claim the workspace's default.
  it("treats an unknown kind as a repo project", () => {
    expect(openingTab("scratch")).toBe("agents");
  });
});
