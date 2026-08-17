import type { RunUsage } from "../api";

// Formatting for a run's token and cost figures.
//
// The rule that drives every branch here: never show a number we cannot stand
// behind. A run on an agent whose transcript we cannot read has no usage at
// all, and a model we have no price for yields tokens without a cost. Both
// render as an absence, not as zero.

/** Compact token count: 1200 -> "1.2k", 3400000 -> "3.4M". */
export function formatTokens(n: number): string {
  if (n < 1_000) return String(n);
  if (n < 1_000_000) {
    const k = n / 1_000;
    return `${k < 10 ? k.toFixed(1) : Math.round(k)}k`;
  }
  const m = n / 1_000_000;
  return `${m < 10 ? m.toFixed(1) : Math.round(m)}M`;
}

/** Whole cents as currency. 5 -> "$0.05", 254072 -> "$2,540.72". */
export function formatCents(cents: number): string {
  return `$${(cents / 100).toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

/**
 * The short label for a run tile, or null when there is nothing true to say.
 *
 * Null covers two different absences that look the same to a reader and must
 * both stay silent: an agent we cannot account for at all, and one that simply
 * has not spent anything yet. Rendering "$0.00" for the first would be a claim
 * we cannot support.
 */
export function usageLabel(usage: RunUsage | null): string | null {
  if (!usage || usage.totalTokens === 0) return null;
  if (usage.cents === null) return formatTokens(usage.totalTokens);
  // A partly priced total is a floor, so it leads with "over" rather than
  // presenting itself as the finished number.
  const cost = formatCents(usage.cents);
  return usage.costComplete ? cost : `over ${cost}`;
}

/** The hover text behind {@link usageLabel}, spelling the split out. */
export function usageTitle(usage: RunUsage | null): string | undefined {
  if (!usage || usage.totalTokens === 0) return undefined;
  const parts = [
    `${formatTokens(usage.inputTokens)} in`,
    `${formatTokens(usage.outputTokens)} out`,
    `${formatTokens(usage.cacheWriteTokens)} cache written`,
    `${formatTokens(usage.cacheReadTokens)} cache read`,
  ];
  const head =
    usage.cents === null
      ? "No published price for this model, so tokens only"
      : usage.costComplete
        ? "Estimated at list prices"
        : "At least this much; some records used a model with no published price";
  return `${head}. ${parts.join(", ")}.`;
}
