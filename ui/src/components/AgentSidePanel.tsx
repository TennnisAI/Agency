import { useEffect, useRef, useState } from "react";
import { Project, RunInfo } from "../api";
import { useRuns } from "../store/runs";
import { runStatus } from "../lib/runstate";
import { loadFocusTab, saveFocusTab, PRIMARY_TAB } from "../lib/focusTab";
import { RunTab, runTabs, showsTabs, sidePanelTab } from "../lib/runTabs";
import { runListLabel } from "../agents";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { useRepoReadiness, isGitless } from "../hooks/useRepoReadiness";
import { useFirstPromptCapture } from "../hooks/useFirstPromptCapture";
import { useSpawnAgent } from "../hooks/useSpawnAgent";
import AgentAddMenu from "./AgentAddMenu";
import FocusTerminal from "./FocusTerminal";

/**
 * Agents and terminals in a side panel, so a note (or a file) can be edited
 * with a live agent next to it instead of behind a tab switch. Selection is the
 * store's focused run — the same agent you left in the Agents tab is the one
 * waiting here, and picking one here focuses it there too.
 *
 * A run is a worktree, and a worktree can hold several agents in tabs
 * (AGE-225). The picker lists those tabs, and the terminal attaches the one
 * picked (AGE-226): it used to attach the run's own session whatever the strip
 * held, so a second agent in the workspace could not be reached from here, and
 * once the first agent's tab had been closed (AGE-184) the panel sat on a
 * session that was gone for good.
 *
 * Only one copy of this ever mounts at a time (the host views are rendered
 * exclusively by tab), so its terminal never competes with the Agents tab's for
 * the same session.
 */
export default function AgentSidePanel({ project }: { project: Project }) {
  const {
    runs, focusedRunId, setFocusedRun, createTerminal, setTab, setView, requestAgentView, spawning,
    setPendingSession, shownTab, setShownTab,
  } = useRuns();
  const { readiness, refresh } = useRepoReadiness(project);
  const { spawn, error, dialogs } = useSpawnAgent(project, refresh);
  const gitless = isGitless(readiness);

  const focused = runs.find((r) => r.id === focusedRunId) ?? null;
  const captureFirstPrompt = useFirstPromptCapture(focused?.id, focused?.title);
  // The tab this panel shows for the focused run: the one the focus view (or a
  // pick here) last put on screen, else the one the run remembers, vetted by
  // sidePanelTab against the tabs the run still draws.
  const shownPanel = focused
    ? shownTab?.runId === focused.id
      ? shownTab.tab
      : typeof localStorage !== "undefined" ? loadFocusTab(localStorage, focused.id) : PRIMARY_TAB
    : null;
  const tab = focused ? sidePanelTab(focused, shownPanel) : null;
  // Picker menu, anchored in viewport coordinates so the panel's own overflow
  // can't clip it (same trick as AgentAddMenu).
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerCoords, setPickerCoords] = useState<{ top: number; left: number; width: number }>();
  const pickerRef = useRef<HTMLButtonElement>(null);
  useEffect(() => { setPickerOpen(false); }, [focusedRunId, project.id]);

  const togglePicker = () => {
    setPickerOpen((o) => {
      const next = !o;
      if (next && pickerRef.current) {
        const r = pickerRef.current.getBoundingClientRect();
        setPickerCoords({ top: r.bottom + 4, left: r.left, width: Math.max(r.width, 200) });
      }
      return next;
    });
  };

  // Those coords stop being true the moment the window changes size.
  useDismissOnResize(pickerOpen, () => setPickerOpen(false));

  // Straight onto that agent, here and in the Agents tab. Focus view too, not
  // just the id: that is what makes this run the one the app is working in, so
  // the file tree and source control follow the agent picked here. The tab
  // goes three ways: published as the one on screen, so this panel and the
  // sidebar tree show it now; saved as the run's last-used tab; and through
  // the pending-tab hand-off the rail uses (AGE-225), so the focus view lands
  // on it when the Agents tab next mounts. The hand-off is set on every pick,
  // a single-tab run's included, so one left over from an earlier pick cannot
  // outlive the run it was for.
  const pickTab = (run: RunInfo, t: RunTab) => {
    setFocusedRun(run.id);
    setView("focus");
    setShownTab({ runId: run.id, tab: t.panel });
    if (typeof localStorage !== "undefined") saveFocusTab(localStorage, run.id, t.panel);
    setPendingSession(t.session);
    setPickerOpen(false);
  };

  // "This agent, over there" — so it lands on the agent, not on the run panel
  // the Agents tab may have been left on for this run.
  const openInAgentsTab = () => {
    setTab("agents");
    setView("focus");
    if (focusedRunId) requestAgentView(focusedRunId);
  };

  const st = focused ? runStatus(focused) : null;

  // One row per run, or, for a workspace with more than one tab (or whose only
  // tab is not the agent it is named after), the run as a heading over a row
  // per tab, each opening straight onto that agent.
  const menuRows = (r: RunInfo) => {
    const tabs = runTabs(r);
    if (!showsTabs(r, tabs)) {
      const only = sidePanelTab(r, null);
      return (
        <button
          key={r.id}
          className={r.id === focusedRunId ? "on" : ""}
          disabled={!only}
          onClick={() => { if (only) pickTab(r, only); }}
        >
          <span className={`dot ${runStatus(r).cls}`} />
          <span className="agent-side-menu-name">{runListLabel(r)}</span>
        </button>
      );
    }
    return (
      <div key={r.id} className="agent-side-menu-run">
        <div className="agent-side-menu-run-name">
          <span className={`dot ${runStatus(r).cls}`} />
          <span className="agent-side-menu-name">{runListLabel(r)}</span>
        </div>
        {tabs.map((t) => (
          <button
            key={t.session}
            className={`agent-side-menu-tab${r.id === focusedRunId && t.panel === tab?.panel ? " on" : ""}`}
            title={`${t.label}: ${t.status}`}
            onClick={() => pickTab(r, t)}
          >
            <span className={`dot ${t.cls}`} />
            <span className="agent-side-menu-name">{t.label}</span>
          </button>
        ))}
      </div>
    );
  };

  return (
    <div className="agent-side">
      <div className="agent-side-head">
        <button
          ref={pickerRef}
          className="agent-side-pick"
          title={focused ? "Switch agent" : "Pick an agent"}
          onClick={togglePicker}
        >
          {st && <span className={`dot ${st.cls}`} />}
          <span className="agent-side-name">
            {focused ? runListLabel(focused) : runs.length ? "Pick an agent…" : "No agents"}
          </span>
          <span className="agent-side-chev" aria-hidden>▾</span>
        </button>
        <AgentAddMenu
          variant="icon"
          projectId={project.id}
          onSpawn={spawn}
          onTerminal={createTerminal}
          gitless={gitless}
        />
        <button
          className="icon-btn"
          title="Open this agent in the Agents tab"
          disabled={!focused}
          onClick={openInAgentsTab}
        >↗</button>
      </div>

      {pickerOpen && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setPickerOpen(false)} />
          <div className="agent-menu agent-side-menu" style={{ position: "fixed", ...pickerCoords }}>
            {runs.length === 0 ? (
              <div className="agent-side-menu-none">Nothing running yet.</div>
            ) : (
              runs.map(menuRows)
            )}
          </div>
        </>
      )}

      {error && <div className="git-error">{error}</div>}

      {focused && tab ? (
        <>
          {focused.kind === "agent" && (
            <div className="agent-side-sub">
              {/* The tab on screen, not the agent the run is named after: with
                  several agents in the workspace they differ. */}
              <span className="agent-side-agent" title={`${tab.label}: ${tab.status}`}>{tab.label}</span>
              {/* Empty in a project with no repository, where the agent works
                  in the folder and has no branch to name. */}
              {focused.branch && <code>{focused.branch}</code>}
            </div>
          )}
          <FocusTerminal
            key={tab.session}
            runId={tab.session}
            // Same wheel policy as the Agents tab: only a plain shell wants
            // xterm's wheel-to-arrow fallback, which an agent prompt would read
            // as history (see lib/termScroll).
            altScrollArrows={tab.agent === "shell"}
            // Only the run's own agent is the one it is named after; an extra
            // tab is not, and a terminal is not a run anyone named from a
            // prompt, so neither has anything to capture.
            onFirstPrompt={focused.kind === "agent" && tab.panel === PRIMARY_TAB ? captureFirstPrompt : undefined}
          />
        </>
      ) : (
        <div className="agent-side-empty">
          {spawning
            ? "starting…"
            : focused
              ? "No agent tabs left in this workspace."
              : runs.length
                ? "Pick an agent above to work with it here."
                : "No agents in this project yet. Start one with \"+\"."}
        </div>
      )}

      {dialogs}
    </div>
  );
}
