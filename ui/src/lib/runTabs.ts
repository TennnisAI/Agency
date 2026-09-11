import type { RunInfo, SessionStatus } from "../api";
import { agentLabel } from "../agents";
import { PRIMARY_TAB } from "./focusTab";
import { runStatus } from "./runstate";

// Every agent tab a run's worktree holds, for the surfaces that list the run by
// one name: the focus rail, the sidebar tree, the tiles (AGE-225). A worktree
// can host several agents side by side in the focus view's tab strip, but
// everywhere outside that strip a run was one row named after its first agent.
// Three agents in one worktree read as one, and once that first agent's tab was
// closed (AGE-184) the row named an agent that was not there at all.

export interface RunTab {
  /** The focus view's tab token: PRIMARY_TAB, or the extra session's id. */
  panel: string;
  /**
   * The daemon session behind the tab: the run's own id for its first agent,
   * `<runId>--<n>` for an extra one. The form an "open this tab" request takes
   * (the store's pendingSessionId).
   */
  session: string;
  agent: string;
  label: string;
  /** Status dot class, from the same palette as the run's own dot. */
  cls: string;
  /** The dot in words, for a title. */
  status: string;
}

/**
 * A tab's name, the one the strip itself draws: the agent for the run's own
 * tab, the agent and its session number for an extra one. "shell" is the
 * reserved profile behind a "New terminal" tab, not an agent.
 */
export function tabLabel(agent: string, session: string, primary: boolean): string {
  if (primary) return agentLabel(agent);
  const n = session.split("--").pop();
  return agent === "shell" ? `≳ terminal · ${n}` : `${agentLabel(agent)} · ${n}`;
}

// An extra tab's dot. Activity (working, waiting, idle) is sampled for a run's
// lead session only, so an extra agent that is up is "live": lit but still.
// That says it is running and nothing about whether it is busy, which is all
// anyone has measured. The pulse would claim output nobody has seen.
function sessionDot(status: SessionStatus): { cls: string; text: string } {
  if (status.state === "running") return { cls: "live", text: "running" };
  if (status.state === "exited") {
    return { cls: "exited", text: status.code === 0 ? "finished" : "failed" };
  }
  return { cls: "exited", text: "not running" };
}

/**
 * The run's tabs in strip order: its own agent unless that tab has been closed,
 * then every extra tab it has a row for, running or not. The same list
 * AgentFocus draws, so a tab listed here is one the strip can open.
 */
export function runTabs(run: RunInfo, now: number = Date.now()): RunTab[] {
  if (run.kind === "terminal") return [];
  const tabs: RunTab[] = [];
  if (!run.primaryClosed) {
    const st = runStatus(run, now);
    tabs.push({
      panel: PRIMARY_TAB,
      session: run.id,
      agent: run.agent,
      label: tabLabel(run.agent, run.id, true),
      cls: st.cls,
      status: st.text,
    });
  }
  for (const s of run.sessions) {
    const st = sessionDot(s.status);
    tabs.push({
      panel: s.id,
      session: s.id,
      agent: s.agent,
      label: tabLabel(s.agent, s.id, false),
      cls: st.cls,
      status: st.text,
    });
  }
  return tabs;
}

/**
 * Whether a run's one-name row needs its tabs spelled out: there is more than
 * one, or the only one is not the agent the row is named after, because that
 * agent's tab was closed.
 */
export function showsTabs(run: Pick<RunInfo, "primaryClosed">, tabs: RunTab[]): boolean {
  return tabs.length > 1 || (run.primaryClosed && tabs.length > 0);
}

/** "3 tabs", for a surface with room for the word. */
export function tabCountLabel(n: number): string {
  return `${n} ${n === 1 ? "tab" : "tabs"}`;
}

/** What the count stands for, named, for its title. */
export function tabsTitle(tabs: RunTab[]): string {
  return `${tabCountLabel(tabs.length)} in this workspace: ${tabs.map((t) => t.label).join(", ")}`;
}

/**
 * The tab the agents side panel on Docs and Files attaches for a run
 * (AGE-226). `panel` is the tab the focus view last showed or remembers for
 * the run; a tab the strip no longer draws (closed while you were on Docs, or
 * the Run tab, which is not an agent) gives way to the first it does. The
 * panel used to attach the run's own session whatever the strip held, so an
 * extra tab could not be reached from it at all, and once the first agent's
 * tab was closed (AGE-184) it showed a dead pane for a workspace whose other
 * agents were still running.
 *
 * A terminal has no strip, so its one session stands as its own tab. Null only
 * for an agent run with no tab left, which the backend refuses to produce.
 */
export function sidePanelTab(run: RunInfo, panel: string | null, now: number = Date.now()): RunTab | null {
  if (run.kind === "terminal") {
    const st = runStatus(run, now);
    return {
      panel: PRIMARY_TAB,
      session: run.id,
      agent: run.agent,
      label: tabLabel(run.agent, run.id, true),
      cls: st.cls,
      status: st.text,
    };
  }
  const tabs = runTabs(run, now);
  return tabs.find((t) => t.panel === panel) ?? tabs[0] ?? null;
}
