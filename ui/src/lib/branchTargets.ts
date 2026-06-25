/**
 * The merge target the UI should use: the user's explicit override if they have
 * diverged it from the base, otherwise the base branch (synced-by-default).
 */
export function effectiveMergeTarget(base: string, override: string | null): string {
  return override ?? base;
}
