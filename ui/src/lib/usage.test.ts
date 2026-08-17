import { describe, it, expect } from "vitest";
import { formatTokens, formatCents, usageLabel, usageTitle } from "./usage";
import type { RunUsage } from "../api";

function usage(over: Partial<RunUsage> = {}): RunUsage {
  return {
    inputTokens: 0,
    outputTokens: 0,
    cacheWriteTokens: 0,
    cacheReadTokens: 0,
    totalTokens: 0,
    cents: null,
    costComplete: true,
    ...over,
  };
}

describe("formatTokens", () => {
  it("leaves small counts alone", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(999)).toBe("999");
  });

  it("switches to k and M at the thresholds", () => {
    expect(formatTokens(1_000)).toBe("1.0k");
    expect(formatTokens(1_200)).toBe("1.2k");
    expect(formatTokens(12_000)).toBe("12k");
    expect(formatTokens(1_000_000)).toBe("1.0M");
    expect(formatTokens(3_400_000)).toBe("3.4M");
    expect(formatTokens(34_000_000)).toBe("34M");
  });
});

describe("formatCents", () => {
  it("always shows two decimals", () => {
    expect(formatCents(0)).toBe("$0.00");
    expect(formatCents(5)).toBe("$0.05");
    expect(formatCents(100)).toBe("$1.00");
  });
});

describe("usageLabel", () => {
  it("says nothing when the agent cannot be accounted for", () => {
    // Distinct from zero spend: rendering "$0.00" here would be a claim we
    // cannot support.
    expect(usageLabel(null)).toBeNull();
  });

  it("says nothing when nothing has been spent yet", () => {
    expect(usageLabel(usage())).toBeNull();
  });

  it("shows tokens alone when the model has no published price", () => {
    expect(usageLabel(usage({ totalTokens: 1_500, cents: null }))).toBe("1.5k");
  });

  it("shows the cost when every record was priceable", () => {
    expect(usageLabel(usage({ totalTokens: 10, cents: 250, costComplete: true }))).toBe("$2.50");
  });

  it("marks a partly priced total as a floor", () => {
    expect(usageLabel(usage({ totalTokens: 10, cents: 250, costComplete: false }))).toBe(
      "over $2.50",
    );
  });
});

describe("usageTitle", () => {
  it("is absent when there is nothing to describe", () => {
    expect(usageTitle(null)).toBeUndefined();
    expect(usageTitle(usage())).toBeUndefined();
  });

  it("spells out the token split", () => {
    const t = usageTitle(
      usage({
        totalTokens: 4,
        inputTokens: 1_000,
        outputTokens: 2_000,
        cacheWriteTokens: 3_000,
        cacheReadTokens: 4_000,
        cents: 10,
      }),
    );
    expect(t).toContain("1.0k in");
    expect(t).toContain("2.0k out");
    expect(t).toContain("3.0k cache written");
    expect(t).toContain("4.0k cache read");
  });

  it("names the reason a cost is missing or incomplete", () => {
    expect(usageTitle(usage({ totalTokens: 1, cents: null }))).toContain("No published price");
    expect(usageTitle(usage({ totalTokens: 1, cents: 1, costComplete: false }))).toContain(
      "At least this much",
    );
    expect(usageTitle(usage({ totalTokens: 1, cents: 1, costComplete: true }))).toContain(
      "Estimated at list prices",
    );
  });

  it("uses no em dashes, per the copy rules", () => {
    const t = usageTitle(usage({ totalTokens: 1, cents: 1, costComplete: false }))!;
    expect(t).not.toContain("—");
  });
});
