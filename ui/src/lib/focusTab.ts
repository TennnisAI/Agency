// Per-run memory of the focus view's active tab. A worktree can host several
// agents (the extra session tabs), so leaving a run and coming back should land
// on the tab you were last using rather than always on the primary agent.
//
// Tab ids are the same tokens AgentFocus's `panel` state uses: "agent" (the
// primary terminal), "run" (the run panel), "log" (the server log of a
// web-served agent, whose own tab shows the GUI instead), or an extra session
// id.
export const PRIMARY_TAB = "agent";

// The run panel's tab. It shares the strip with the agent tabs, and is the only
// tab that is not an agent, which is why clicking an agent's name anywhere else
// in the app has to leave it (see agentViewTab).
export const RUN_TAB = "run";

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

// The tab to land on when the remembered one cannot be drawn. Normally the
// primary agent; once that tab has been closed (AGE-184) the run is carried by
// its extra tabs, so it is the leftmost of those that is still alive. With
// none, the primary stands: the strip draws it again the moment the run has an
// agent of its own, and a tab id nothing renders would be worse.
function fallbackTab(
  sessions: { id: string; status: { state: string } }[],
  primaryClosed: boolean,
): string {
  if (!primaryClosed) return PRIMARY_TAB;
  return sessions.find((s) => s.status.state !== "gone")?.id ?? PRIMARY_TAB;
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
//
// `primaryClosed` says the run's own agent tab has been closed, so the strip
// does not draw it either and it cannot be the fallback.
export function resolveFocusTab(
  remembered: string,
  sessions: { id: string; status: { state: string } }[],
  hasLog = false,
  primaryClosed = false,
): string {
  const fallback = fallbackTab(sessions, primaryClosed);
  if (remembered === PRIMARY_TAB) return primaryClosed ? fallback : remembered;
  if (remembered === RUN_TAB) return remembered;
  if (remembered === LOG_TAB) return hasLog ? remembered : fallback;
  const live = sessions.some((s) => s.id === remembered && s.status.state !== "gone");
  return live ? remembered : fallback;
}

// The tab an "open this agent" click should land on: the focus rail's rows, the
// sidebar tree's rows under a project, and everything else that opens a run by
// name. Sitting on the Run tab, the only way back to the agent was the strip's
// own agent tab, and clicking the agent's name in a pane did nothing at all
// (the run was already focused, so nothing re-ran) — the one gesture that reads
// like "show me this agent" was the one that didn't. So a name click leaves the
// Run tab, for the tab it was opened from. Every other tab stands: an extra
// agent tab is still the agent view, and that is the memory AGE-31 added.
export function agentViewTab(current: string, before: string): string {
  if (current !== RUN_TAB) return current;
  return before === RUN_TAB ? PRIMARY_TAB : before;
}
