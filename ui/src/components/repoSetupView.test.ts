import { describe, it, expect } from "vitest";
import { largeFileSummary, repoSetupView } from "./repoSetupView";
import type { LargeFileScan } from "../api";

describe("repoSetupView", () => {
  it("notARepo → init prompt", () => {
    const v = repoSetupView({ state: "notARepo", stageable: false, dirty: false }, "add");
    expect(v.kind).toBe("init");
    expect(v.title).toMatch(/Set up/i);
    expect(v.secondaryLabel).toBeNull();
  });
  it("noCommits → initial commit prompt", () => {
    const v = repoSetupView({ state: "noCommits", stageable: true, dirty: false }, "add");
    expect(v.kind).toBe("commit");
    expect(v.primaryLabel).toMatch(/initial commit/i);
  });
  it("ready+dirty in add context → 'Add anyway' secondary", () => {
    const v = repoSetupView({ state: "ready", stageable: false, dirty: true }, "add");
    expect(v.kind).toBe("dirty");
    expect(v.secondaryLabel).toBe("Add anyway");
  });
  it("ready+dirty in spawn context → 'Spawn anyway' secondary", () => {
    const v = repoSetupView({ state: "ready", stageable: false, dirty: true }, "spawn");
    expect(v.secondaryLabel).toBe("Spawn anyway");
  });
  it("ready+clean → kind ready (no-op)", () => {
    const v = repoSetupView({ state: "ready", stageable: false, dirty: false }, "add");
    expect(v.kind).toBe("ready");
  });
});

describe("largeFileSummary", () => {
  const scan = (over: Partial<LargeFileScan>): LargeFileScan => ({
    files: [],
    count: 0,
    bytes: 0,
    truncated: false,
    ignorePaths: [],
    ...over,
  });

  it("counts the files and their weight", () => {
    expect(largeFileSummary(scan({ count: 3, bytes: 48 * 1024 ** 3 }))).toBe(
      "3 files here are over 100 MB (48.0 GB in total)",
    );
  });
  it("says 'file' for one", () => {
    expect(largeFileSummary(scan({ count: 1, bytes: 200 * 1024 ** 2 }))).toBe(
      "1 file here is over 100 MB (200.0 MB)",
    );
  });
  it("a truncated scan reports a floor, not a total", () => {
    expect(largeFileSummary(scan({ count: 2000, bytes: 1024 ** 4, truncated: true }))).toMatch(
      /^At least 2000 files/,
    );
  });
});
