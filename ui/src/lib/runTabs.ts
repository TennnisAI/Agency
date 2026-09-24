import type { RunInfo, RunSessionInfo } from "../api";
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

// An extra tab's dot. A running agent tab reads exactly like the run's own
// agent: the backend samples each tab's activity (AGE-249), and the overview's
// counts are made from it, so a dot that said less would contradict them. A
// "New terminal" tab is a shell, which is never waiting on you, so it is "live":
// lit but still, like a terminal run's plain "running".
function sessionDot(s: RunSessionInfo, now: number): { cls: string; text: string } {
  const status = s.status;
  if (status.state === "running") {
    if (s.agent === "shell") return { cls: "live", text: "running" };
    return runStatus({ kind: "agent", status, activity: s.activity }, now);
  }
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
    const st = sessionDot(s, now);
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
 * The tabs waiting on you that a tile's own dot does not show. The "waiting"
 * filter keeps a workspace for any tab in it (AGE-249), so clicking "1 waiting"
 * could bring up a tile whose dot said "working" and nothing on it said which
 * tab was the one asking.
 *
 * `dot` is the class of the tile's own dot, `runStatus(run).cls`. It shows the
 * first tab, or once that was closed the lowest-numbered one still running
 * (AGE-184). It accounts for that tab only when it reads waiting itself;
 * otherwise every waiting tab is listed, the dot's own included.
 */
export function waitingElsewhere(run: Pick<RunInfo, "primaryClosed">, tabs: RunTab[], dot: string): RunTab[] {
  const waiting = tabs.filter((t) => t.cls === "awaiting");
  if (dot !== "awaiting") return waiting;
  const shown = run.primaryClosed ? tabs.find((t) => t.cls !== "exited") : tabs[0];
  return waiting.filter((t) => t !== shown);
}

/** "1 other tab waiting", for the foot of a tile, or null when there are none. */
export function waitingElsewhereLabel(tabs: RunTab[]): string | null {
  if (tabs.length === 0) return null;
  return `${tabs.length} other ${tabs.length === 1 ? "tab" : "tabs"} waiting`;
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
