import { describe, it, expect } from "vitest";
import {
  largeFileSummary,
  ignoreRulesLine,
  ignoreRulesCaveat,
  repoSetupView,
} from "./repoSetupView";
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

const scan = (over: Partial<LargeFileScan>): LargeFileScan => ({
  files: [],
  count: 0,
  bytes: 0,
  truncated: false,
  ignorePaths: [],
  thresholdBytes: 100 * 1024 ** 2,
  ...over,
});

describe("largeFileSummary", () => {
  it("counts the files and their weight", () => {
    expect(largeFileSummary(scan({ count: 3, bytes: 48 * 1024 ** 3 }))).toBe(
      "3 files here are over 100.0 MB (48.0 GB in total)",
    );
  });
  it("says 'file' for one", () => {
    expect(largeFileSummary(scan({ count: 1, bytes: 200 * 1024 ** 2 }))).toBe(
      "1 file here is over 100.0 MB (200.0 MB)",
    );
  });
  it("a truncated scan reports a floor, not a total", () => {
    expect(largeFileSummary(scan({ count: 2000, bytes: 1024 ** 4, truncated: true }))).toMatch(
      /^At least 2000 files/,
    );
  });
  it("names the threshold the scan actually used", () => {
    // Follows the backend rather than restating it, so the two can't drift.
    expect(largeFileSummary(scan({ count: 1, bytes: 0, thresholdBytes: 2 * 1024 ** 3 }))).toContain(
      "over 2.0 GB",
    );
  });
});

describe("ignoreRulesLine", () => {
  const paths = (n: number) => Array.from({ length: n }, (_, i) => `dir${i}/`);

  it("lists every rule when there are few", () => {
    expect(ignoreRulesLine(scan({ ignorePaths: ["models/", "data/big.bin"] }))).toBe(
      "models/, data/big.bin",
    );
  });
  it("caps a long list so the modal can't outgrow its own buttons", () => {
    const line = ignoreRulesLine(scan({ ignorePaths: paths(200) }));
    expect(line).toBe(`${paths(6).join(", ")}, and 194 more`);
  });
  it("does not say 'and 0 more' at exactly the cap", () => {
    expect(ignoreRulesLine(scan({ ignorePaths: paths(6) }))).toBe(paths(6).join(", "));
  });
});

describe("ignoreRulesCaveat", () => {
  it("stays quiet when the scan finished", () => {
    expect(ignoreRulesCaveat(scan({ ignorePaths: ["models/"] }))).toBeNull();
  });
  it("warns that the rules may not cover everything when the scan gave up", () => {
    // The rules only reach what the walk saw, and committing is the choice here
    // that can't be quietly undone.
    expect(ignoreRulesCaveat(scan({ truncated: true }))).toMatch(/may hold large files/);
  });
});
