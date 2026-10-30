import { useEffect, useState } from "react";
import { RunSessionInfo, listRunSessions, sendMergeConflict, spawnMergeConflictAgent } from "../../api";
import { agentLabel } from "../../agents";
import { loadFocusTab, PRIMARY_TAB } from "../../lib/focusTab";
import { useRuns } from "../../store/runs";
import { MergeHandoff, patchMergeFlow, pickTarget, useMergeFlows } from "./mergeFlow";

// The picker's last option: not an agent that exists, but one to start. Every
// agent already in the worktree may be mid-turn on something else, and a
// prompt handed to one of those waits for it; a tab opened for the conflict
// takes it as its opening prompt and begins at once (AGE-199). The space in
// the value is what keeps it apart from every real session id, which is either
// a run id or `<run id>--<n>` and can hold neither a space nor this word.
const NEW_AGENT = "new agent";

// How an extra tab is named in the picker and in the sentence that follows:
// the agent, then the tab number the strip draws.
const tabLabel = (agent: string, id: string) => `${agentLabel(agent)} · ${id.split("--").pop()}`;

// "Fix with agent", shared by the merge window and the source control panel so
// the two offer the same agents and report the same hand-off.
//
// The hand-off also takes you to it: the prompt is typed into a live session, so
// the thing worth looking at next is that session, with the panel beside it
// saying where the merge stands. There used to be a "Close and watch" button
// here, a second click to ask for what the first one obviously meant (AGE-273).
export function useMergeHandoff(runId: string) {
  const { runs, setPendingSession, setSourcePanelOpen, setTab, setView } = useRuns();
  const handoff = useMergeFlows().get(runId)?.handoff ?? null;
  const me = runs.find((r) => r.id === runId);
  // The agent tabs sharing this run's worktree, so the conflict can be handed
  // to the one whose context has something to do with it (AGE-184). This used
  // to go to the run's own agent every time, which after a day's work is
  // routinely the one that has been doing something else entirely.
  const [tabs, setTabs] = useState<RunSessionInfo[]>([]);
  // Asked for and answered. The default target has to wait for this: the run
  // itself is in the store already, so without it the default would settle on
  // the run's own agent one render before the tabs that might hold the
  // remembered one arrived.
  const [tabsLoaded, setTabsLoaded] = useState(false);
  const [picked, setPicked] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState("");

  // Asked again after a hand-off: one that opened a new tab did so from the
  // window, and this may be the panel, which has been mounted the whole time.
  const handedTo = handoff?.session ?? null;
  useEffect(() => {
    let live = true;
    listRunSessions(runId)
      .then((s) => { if (live) setTabs(s); })
      .catch(() => {})
      .finally(() => { if (live) setTabsLoaded(true); });
    return () => { live = false; };
  }, [runId, handedTo]);

  // The same tabs the run's strip draws, in the same order. A shell tab is left
  // out: it is a login shell, not something that reads a prompt. A fresh agent
  // goes last, after the ones that exist: it is what to reach for when none of
  // them is free, not the first choice (AGE-199).
  const targets: { value: string; label: string }[] = [
    ...(me && !me.primaryClosed ? [{ value: me.id, label: agentLabel(me.agent) }] : []),
    ...tabs
      .filter((t) => t.status.state !== "gone" && t.agent !== "shell")
      .map((t) => ({ value: t.id, label: tabLabel(t.agent, t.id) })),
    { value: NEW_AGENT, label: "New agent" },
  ];
  let target: string | null = null;
  if (tabsLoaded) {
    const remembered = typeof localStorage !== "undefined"
      ? loadFocusTab(localStorage, runId)
      : PRIMARY_TAB;
    target = pickTarget(
      targets.map((t) => t.value),
      picked,
      handedTo,
      remembered === PRIMARY_TAB ? runId : remembered,
    );
  }
  const newAgent = target === NEW_AGENT;

  // Resolves true once the conflict is with an agent, so the window knows it can
  // close. The backend submits the prompt, so this really does start the agent
  // working rather than leaving a composed message sitting in its prompt box.
  //
  // "New agent" is the other shape of the same gesture: a tab opened for this
  // job, with the conflict as its opening prompt. Nothing can be queued behind
  // a session that did not exist a moment ago, so that one usually never waits,
  // except for a CLI that takes no prompt on the command line, where the backend
  // falls back to the queue and says so.
  async function send(): Promise<boolean> {
    setError("");
    setSending(true);
    try {
      let next: MergeHandoff;
      if (newAgent) {
        const { session, queued } = await spawnMergeConflictAgent(runId);
        // The same tab list the session has just joined, so the sentence names
        // the agent that got it without waiting on the reload above.
        setTabs((prev) => [...prev, session]);
        setPicked(null);
        next = {
          session: session.id,
          label: tabLabel(session.agent, session.id),
          spawned: true,
          queued,
        };
      } else {
        // False means the agent is not free (mid-turn, or with something
        // half-typed on its prompt line) and the prompt is held behind that;
        // saying "sent" then would have the user watching for work that has not
        // started yet.
        const went = await sendMergeConflict(runId, target ?? undefined);
        next = {
          session: target ?? runId,
          label: targets.find((t) => t.value === target)?.label ?? "this agent",
          spawned: false,
          queued: !went,
        };
      }
      patchMergeFlow(runId, { handoff: next });
      setPendingSession(next.session);
      setTab("agents");
      setView("focus");
      setSourcePanelOpen(true);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setSending(false);
    }
  }

  return {
    targets,
    target,
    setTarget: setPicked,
    handoff,
    // Already with the agent on the picker. Moving the picker makes it a new
    // hand-off rather than a repeat of the last one.
    sent: !!handoff && handoff.session === target,
    sendLabel: newAgent ? "Start a new agent" : "Fix with agent",
    sendingLabel: newAgent ? "Starting…" : "Sending…",
    send,
    sending,
    error,
  };
}
