// Per-run memory of the focus view's active tab. A worktree can host several
// agents (the extra session tabs), so leaving a run and coming back should land
// on the tab you were last using rather than always on the primary agent.
//
// Tab ids are the same tokens AgentFocus's `panel` state uses: "agent" (the
// primary terminal), "run" (the run panel), "log" (the server log of a
// web-served agent, whose own tab shows the GUI instead), or an extra session
// id.
export const PRIMARY_TAB = "agent";

// The pane a web-served agent's terminal moves to. Its GUI is the whole view on
// the agent's own tab, so the server log — which is all that pane ever shows —
// lives one tab over rather than taking half the window beside it.
export const LOG_TAB = "log";

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
//
// `hasLog` says whether this run still serves a browser GUI. It stops being one
// when its web session is closed (or a loop takes the run over), and the log
// tab goes with it — restoring onto a tab the strip no longer draws would leave
// an empty pane with no way back except another tab.
export function resolveFocusTab(
  remembered: string,
  sessions: { id: string; status: { state: string } }[],
  hasLog = false,
): string {
  if (remembered === PRIMARY_TAB || remembered === "run") return remembered;
  if (remembered === LOG_TAB) return hasLog ? remembered : PRIMARY_TAB;
  const live = sessions.some((s) => s.id === remembered && s.status.state !== "gone");
  return live ? remembered : PRIMARY_TAB;
}
