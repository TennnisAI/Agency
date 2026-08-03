// Per-run memory of the focus view's active tab. A worktree can host several
// agents (the extra session tabs), so leaving a run and coming back should land
// on the tab you were last using rather than always on the primary agent.
//
// Tab ids are the same tokens AgentFocus's `panel` state uses: "agent" (the
// primary terminal), "run" (the run panel), or an extra session id.
export const PRIMARY_TAB = "agent";

const key = (runId: string) => `focus-tab:${runId}`;

export function loadFocusTab(
  storage: Pick<Storage, "getItem">,
  runId: string,
): string {
  try {
    return storage.getItem(key(runId)) || PRIMARY_TAB;
  } catch {
    return PRIMARY_TAB;
  }
}

export function saveFocusTab(
  storage: Pick<Storage, "setItem">,
  runId: string,
  tab: string,
): void {
  try {
    storage.setItem(key(runId), tab);
  } catch {
    /* ignore quota / security errors */
  }
}

// Vets a remembered tab against the sessions the run actually has now. A tab
// whose session was closed while we were away (or died, and so no longer
// renders) would otherwise restore as a dead pane, so it falls back to the
// primary agent. Sessions that merely exited keep their tab, matching the tab
// strip, which only drops "gone" ones.
export function resolveFocusTab(
  remembered: string,
  sessions: { id: string; status: { state: string } }[],
): string {
  if (remembered === PRIMARY_TAB || remembered === "run") return remembered;
  const live = sessions.some((s) => s.id === remembered && s.status.state !== "gone");
  return live ? remembered : PRIMARY_TAB;
}
