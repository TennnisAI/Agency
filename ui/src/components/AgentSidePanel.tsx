import { useEffect, useRef, useState } from "react";
import { Project } from "../api";
import { useRuns } from "../store/runs";
import { runStatus } from "../lib/runstate";
import { agentLabel, runListLabel } from "../agents";
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
 * Only one copy of this ever mounts at a time (the host views are rendered
 * exclusively by tab), so its terminal never competes with the Agents tab's for
 * the same session.
 */
export default function AgentSidePanel({ project }: { project: Project }) {
  const { runs, focusedRunId, setFocusedRun, createTerminal, setTab, setView, requestAgentView, spawning } = useRuns();
  const { readiness, refresh } = useRepoReadiness(project);
  const { spawn, error, dialogs } = useSpawnAgent(project, refresh);
  const gitless = isGitless(readiness);

  const focused = runs.find((r) => r.id === focusedRunId) ?? null;
  const captureFirstPrompt = useFirstPromptCapture(focused?.id, focused?.title);
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

  // "This agent, over there" — so it lands on the agent, not on the run panel
  // the Agents tab may have been left on for this run.
  const openInAgentsTab = () => {
    setTab("agents");
    setView("focus");
    if (focusedRunId) requestAgentView(focusedRunId);
  };

  const st = focused ? runStatus(focused) : null;

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
              runs.map((r) => (
                <button
                  key={r.id}
                  className={r.id === focusedRunId ? "on" : ""}
                  // Focus view too, not just the id: that is what makes this
                  // run the one the app is working in, so the file tree and
                  // source control follow the agent picked here.
                  onClick={() => { setFocusedRun(r.id); setView("focus"); setPickerOpen(false); }}
                >
                  <span className={`dot ${runStatus(r).cls}`} />
                  <span className="agent-side-menu-name">{runListLabel(r)}</span>
                </button>
              ))
            )}
          </div>
        </>
      )}

      {error && <div className="git-error">{error}</div>}

      {focused ? (
        <>
          {focused.kind === "agent" && (
            <div className="agent-side-sub">
              <span className="agent-side-agent">{agentLabel(focused.agent)}</span>
              {/* Empty in a project with no repository, where the agent works
                  in the folder and has no branch to name. */}
              {focused.branch && <code>{focused.branch}</code>}
            </div>
          )}
          <FocusTerminal
            key={focused.id}
            runId={focused.id}
            // Same wheel policy as the Agents tab: only a plain shell wants
            // xterm's wheel-to-arrow fallback, which an agent prompt would read
            // as history (see lib/termScroll).
            altScrollArrows={focused.agent === "shell"}
            // A terminal is not a run anyone named from a prompt, so it has
            // nothing to capture.
            onFirstPrompt={focused.kind === "agent" ? captureFirstPrompt : undefined}
          />
        </>
      ) : (
        <div className="agent-side-empty">
          {spawning
            ? "starting…"
            : runs.length
              ? "Pick an agent above to work with it here."
              : "No agents in this project yet. Start one with \"+\"."}
        </div>
      )}

      {dialogs}
    </div>
  );
}
