import React, { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useRuns, SpawnOpts } from "../store/runs";
import {
  listProfiles, AgentProfile,
  listRunSessions, startRunSession, closeRunSession, reopenRunAgent, RunSessionInfo,
  RunInfo, SessionStatus, stopLoop, listIssues, listProjects, ensureRunActive,
} from "../api";
import { Removal, removalLabel, removalsFor } from "../lib/runRemoval";
import { issueLabel } from "../lib/issues";
import { requestNavigate } from "../lib/navigate";
import { toastError } from "../lib/toast";
import { inGitlessFolder, runStatus } from "../lib/runstate";
import { runListLabel, agentLabel } from "../agents";
import FocusTerminal, { shellStream } from "./FocusTerminal";
import RunPanel from "./RunPanel";
import ConfirmDialog from "./ConfirmDialog";
import Resizer from "./Resizer";
import ArchivedSection from "./ArchivedSection";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { useRunMenu } from "../hooks/useRunMenu";
import { useFirstPromptCapture } from "../hooks/useFirstPromptCapture";
import { usePaneWidth, loadFold, saveFold } from "../hooks/usePaneWidth";
import { agentViewTab, loadFocusTab, saveFocusTab, resolveFocusTab, PRIMARY_TAB, RUN_TAB, LOG_TAB } from "../lib/focusTab";
import AgentAddMenu from "./AgentAddMenu";
import QueuedMarker from "./QueuedMarker";
import OverflowMenu from "./OverflowMenu";
import { PinMark, pinItems } from "./AttentionMarker";
import { TrashIcon, InboxIcon, TerminalIcon, PencilIcon, CheckIcon, BranchIcon } from "./icons";

const SHELL_MIN = 120;
const SHELL_MAX = 640;
const SHELL_FOLD_KEY = "focus-shell-open";

// The pane for an agent whose interactive surface is a browser app served from
// its workspace (RunInfo.guiPort, e.g. dsh). It is the whole tab, not a half of
// one: the terminal beside it only ever holds the server's boot line, and half
// a window spent on `web: http://127.0.0.1:5218` is half a window the user
// works in. The log moves to its own tab (LOG_TAB).
//
// Kept mounted while that log tab is showing, hidden rather than unmounted: an
// iframe keeps its document through `display: none`, so a look at the log costs
// nothing. Their sessions live on the server keyed to the workspace directory,
// so a reload (the Run tab, which replaces the whole body) reopens the same
// conversation — what it loses is the page's own state: scroll, a half-typed
// message, whatever panel was open.
function GuiPane({
  run,
  sessionId,
  status,
  hidden,
}: {
  run: RunInfo;
  sessionId: string;
  status: SessionStatus;
  hidden: boolean;
}) {
  // Remounting the iframe is the reload: same trick as the Run tab's preview.
  const [reloadKey, setReloadKey] = useState(0);
  const url = `http://127.0.0.1:${run.guiPort}/`;
  // Mounting a terminal is what revives a session the app was quit on, and this
  // tab has no terminal: without this, reopening a web agent after a restart
  // would sit on "the agent isn't running" with the only way to start it being
  // to go and look at its Log. Once per session, like the terminal's own call —
  // a server that failed to come up must not be respawned on every poll.
  const revived = useRef<string | null>(null);
  useEffect(() => {
    if (status.state !== "gone" || revived.current === sessionId) return;
    revived.current = sessionId;
    ensureRunActive(sessionId).catch((e) => toastError(e, "Couldn't start the agent"));
  }, [sessionId, status.state]);
  return (
    <div className={`focus-gui ${hidden ? "is-hidden" : ""}`}>
      <div className="focus-gui-head">
        <code className="run-url">{url}</code>
        <span className="spacer" />
        <button
          className="tile-act"
          title="Reload the GUI"
          onClick={() => setReloadKey((k) => k + 1)}
        >⟳ Reload</button>
        <button
          className="tile-act"
          title={`Open ${url} in your browser`}
          onClick={() => openUrl(url).catch((e) => toastError(e, "Couldn't open the GUI"))}
        >
          Open ↗
        </button>
      </div>
      {run.guiLive ? (
        // Sandboxed for one reason: a plain frame may navigate the top-level
        // context on a click, and in a window with no browser chrome that is a
        // one-way trip out of Agency with no way back. Everything the app
        // actually needs is granted — its own origin (so its `/api` calls and
        // storage work), scripts, forms, dialogs, popups and file downloads —
        // so the only thing withheld is the navigation.
        <iframe
          key={reloadKey}
          className="focus-gui-frame"
          sandbox="allow-scripts allow-same-origin allow-forms allow-modals allow-popups allow-popups-to-escape-sandbox allow-downloads"
          src={url}
          title={`${agentLabel(run.agent)} GUI`}
        />
      ) : (
        <div className="focus-gui-wait">
          {status.state === "running"
            ? `Starting the ${agentLabel(run.agent)} GUI. It opens here once the server answers; the Log tab shows what it is doing.`
            : status.state === "exited"
            ? `The server stopped (exit ${status.code}). The Log tab has what it printed on the way out.`
            : "Starting the agent. Its GUI opens here once the server answers."}
        </div>
      )}
    </div>
  );
}

// The server log of a web-served agent, sat next to the tab whose view its GUI
// took over. Not a session of its own: it is the same pane that agent's tab
// used to be, moved out of the way of the thing the user actually works in.
function LogTab({ panel, onSelect }: { panel: string; onSelect: (t: string) => void }) {
  return (
    <button
      className={`session-tab ${panel === LOG_TAB ? "on" : ""}`}
      title="Log: what this agent's server is printing"
      onClick={() => onSelect(LOG_TAB)}
    >≡ log</button>
  );
}

function badgeClass(a: string) {
  return ["claude", "pi", "hermes"].includes(a) ? `badge ${a}` : "badge";
}

// Live progress of a looping run: attempt counter, phase, and a stop control
// while active; the outcome once terminal. State arrives with the run via the
// store's poll, so this renders fresh data without its own polling.
function LoopStrip({ run, onChanged }: { run: RunInfo; onChanged: () => void }) {
  const cfg = run.loopConfig;
  const st = run.loopState;
  if (!cfg || !st) return null;
  const active = st.status === "awaitingAgent" || st.status === "checking";
  // The stall names its reason (AGE-110): "stalled" alone doesn't say whether
  // to raise a cap, fix a flag, or rewrite the prompt. Null reasons (stalls
  // recorded before reasons existed, and attempt-spawn failures) keep the old
  // generic text.
  const stalledText =
    st.stallReason === "wallClock" ? "stalled · time cap hit"
    : st.stallReason === "budget" ? "stalled · token cap hit"
    : st.stallReason === "crashLoop" ? "stalled · agent failed repeatedly"
    : st.stallReason === "attemptCap" ? `stalled · attempt cap after attempt ${st.attempt}`
    : `stalled · after attempt ${st.attempt}`;
  const text =
    st.status === "awaitingAgent" ? `attempt ${st.attempt}/${cfg.maxAttempts} · running`
    : st.status === "checking" ? `attempt ${st.attempt}/${cfg.maxAttempts} · checking`
    : st.status === "complete" ? (cfg.checkCommand
        ? `complete · checks passed on attempt ${st.attempt}`
        : `complete · ${st.attempt} attempts`)
    : st.status === "stalled" ? stalledText
    : "stopped";
  return (
    <div className={`loop-strip ${active ? "active" : st.status}`}>
      <span className="loop-glyph">⟳</span>
      <span className="loop-text">{text}</span>
      {cfg.checkCommand && <code className="loop-check-cmd">{cfg.checkCommand}</code>}
      <span className="spacer" />
      {active && (
        <button
          className="tile-act"
          title="Stop the loop. The worktree and its commits stay."
          onClick={async () => {
            try {
              await stopLoop(run.id);
            } catch (e) {
              toastError(e, "Couldn't stop loop");
            }
            onChanged();
          }}
        >■ Stop loop</button>
      )}
    </div>
  );
}

// One entry in the agents rail. Hovering it (or tabbing to its control) reveals
// a close button, and the menu behind that is where a run ends without first
// having to open it: archived, so its branch survives and the Archived section
// can restore it, or deleted outright. A terminal only closes. Right-clicking
// the row opens the run's full menu, the same one its tile carries.
function RailRow({
  run,
  on,
  onSelect,
  onRename,
  onRemove,
  onContextMenu,
  menuOpen,
}: {
  run: RunInfo;
  on: boolean;
  onSelect: () => void;
  onRename: () => void;
  onRemove: (action: Removal) => void;
  onContextMenu: (e: React.MouseEvent) => void;
  /** This row's menu is up; its overlay has taken the pointer away. */
  menuOpen: boolean;
}) {
  const isTerminal = run.kind === "terminal";
  return (
    <div className={`rail-row-wrap${menuOpen ? " ctx" : ""}`} onContextMenu={onContextMenu}>
      <button
        className={`rail-row ${on ? "on" : ""}`}
        onClick={onSelect}
        onDoubleClick={onRename}
        title="Double-click to rename"
      >
        <PinMark run={run} />
        <span className={`dot ${runStatus(run).cls}`} />
        <span className="rail-name">{runListLabel(run)}</span>
        {run.runScriptsLive && (
          <span className="run-dot" title="A run script is running in this workspace" />
        )}
      </button>
      {/* Sibling of the row rather than a child of it: a button inside a button
          is invalid markup, and the row keeps its own click target intact. */}
      <OverflowMenu
        buttonClass="hover-close rail-row-close"
        icon={<span aria-hidden>✕</span>}
        title={isTerminal ? "Close terminal" : "Archive or delete this agent"}
        items={removalsFor(run).map((action) => ({
          label: removalLabel(run, action),
          icon: action === "archive" ? <InboxIcon /> : <TrashIcon />,
          danger: action === "delete",
          onSelect: () => onRemove(action),
        }))}
      />
    </div>
  );
}

// `onSpawn` lets the host view wrap agent creation with its pre-flight checks
// (missing-CLI install offer, repo readiness); without it the rail's add menu
// falls back to the raw store spawn.
export default function AgentFocus({
  onSpawn,
  gitless = false,
}: {
  onSpawn?: (agentId: string, opts?: SpawnOpts) => void;
  // The project folder has no git repository, so the rail's add menu hides
  // everything that needs a branch (see AgentAddMenu).
  gitless?: boolean;
}) {
  const { runs, focusedRunId, setFocusedRun, refreshRuns, createAgent, createTerminal, selectedProjectId, pendingSessionId, setPendingSession, agentViewRunId, requestAgentView, setApproveRun } = useRuns();
  // The rail's right-click menu, and the dialogs its entries raise: renaming a
  // run or its branch, archiving, deleting. The header menu below drives the
  // same actions rather than keeping a second copy of them.
  const { openRunMenu, menuRunId, rename, renameBranch, remove, runMenu } = useRunMenu();
  // "agent" (primary terminal), "run" (RunPanel), or an extra-session id —
  // extra agent tabs sharing this run's worktree.
  const [panel, setPanel] = useState<string>(PRIMARY_TAB);
  // Mirror of `panel` for effects that need the value at the moment an await
  // settles, not the one captured when they started. Written the moment a tab
  // is decided as well as on render, so an effect that runs in the same commit
  // as the one that changed it reads the new tab rather than the old one.
  const panelRef = useRef(panel);
  panelRef.current = panel;
  // Every deliberate tab change goes through here so the run remembers where
  // you left it (AGE-31): with several agents in one worktree, coming back to
  // it should reopen the agent you were using, not always the primary one.
  const selectPanel = useCallback((next: string) => {
    setPanel(next);
    panelRef.current = next;
    if (focusedRunId && typeof localStorage !== "undefined") saveFocusTab(localStorage, focusedRunId, next);
  }, [focusedRunId]);
  // The tab Run was opened from, so backing out of the run setup card returns
  // where you came from instead of dumping you on the primary agent.
  const beforeRun = useRef<string>(PRIMARY_TAB);
  const [sessions, setSessions] = useState<RunSessionInfo[]>([]);
  // Read by the tab-restore effect, which keys off the run id alone: the strip
  // does not draw a closed primary tab, so restoring onto it would strand an
  // empty pane. Declared here (like guiSessionRef below) so the effect does not
  // have to re-run every time the flag changes.
  const primaryClosedRef = useRef(false);
  // The tabs the strip is drawing, for effects that need them at the moment
  // they run rather than the value captured when they were declared.
  const drawnTabsRef = useRef<string[]>([]);
  const [confirmCloseTab, setConfirmCloseTab] = useState<string | null>(null);
  // "+" tab menu: agent profiles to open as an extra tab. Anchored in viewport
  // coordinates like AgentAddMenu so ancestor overflow can't clip it.
  const [addOpen, setAddOpen] = useState(false);
  const [addCoords, setAddCoords] = useState<{ top: number; left: number }>();
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const addBtnRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    setApproveRun(null);
    setAddOpen(false);
    setSessions([]);
    // Reopen this run on its last-used tab. The strip isn't loaded yet, so the
    // remembered id is taken on trust here and vetted against the sessions
    // below; an unknown run remembers nothing and lands on the primary agent.
    const remembered = focusedRunId && typeof localStorage !== "undefined"
      ? loadFocusTab(localStorage, focusedRunId)
      : PRIMARY_TAB;
    setPanel(remembered);
    panelRef.current = remembered;
    beforeRun.current = remembered === RUN_TAB ? PRIMARY_TAB : remembered;
    if (!focusedRunId) return;
    // `live` drops a response that lands after the run changed (a slow fetch for
    // the previous run must not repopulate this one's tab strip). The interval
    // re-polls so a tab opened or closed elsewhere — a PR review that had to
    // share this worktree, say — turns up without a focus change.
    let live = true;
    // Only the first response vets the restored tab. Later polls leave the
    // visible tab alone: a session that dies while you're watching it keeps its
    // pane until you close the tab.
    let vet = true;
    const load = () =>
      listRunSessions(focusedRunId)
        .then((s) => {
          if (!live) return;
          setSessions(s);
          if (!vet) return;
          vet = false;
          // Only correct the tab we restored — anything the user (or a pending
          // session hand-off) has since picked stands.
          if (panelRef.current !== remembered) return;
          const resolved = resolveFocusTab(
            remembered,
            s,
            guiSessionRef.current != null,
            primaryClosedRef.current,
          );
          if (resolved === remembered) return;
          selectPanel(resolved);
          if (beforeRun.current === remembered) beforeRun.current = resolved;
        })
        .catch(() => {});
    load();
    const iv = setInterval(load, 4000);
    return () => { live = false; clearInterval(iv); };
  }, [focusedRunId, selectPanel, setApproveRun]);

  // A run opened for a specific extra tab (an agent PR review that had to share
  // this worktree) lands on that tab instead of the one it remembers. Declared
  // after the restore above so it wins on the render that focuses the run, and
  // cleared once applied — from here on it is simply this run's last-used tab.
  useEffect(() => {
    if (!pendingSessionId || !focusedRunId) return;
    if (!pendingSessionId.startsWith(`${focusedRunId}--`)) return;
    selectPanel(pendingSessionId);
    setPendingSession(null);
  }, [pendingSessionId, focusedRunId, setPendingSession, selectPanel]);

  // "Show me this agent", clicked by name in the rail, the sidebar tree or the
  // palette. Only the Run tab moves (agentViewTab), and only for the run the
  // request names — a request for a run that isn't focused yet waits for the
  // focus change, which lands in this same commit and has already restored its
  // remembered tab into panelRef above.
  useEffect(() => {
    if (!agentViewRunId || agentViewRunId !== focusedRunId) return;
    requestAgentView(null);
    const next = agentViewTab(panelRef.current, beforeRun.current);
    // The tab it was opened from may be the primary agent's, which the strip
    // no longer draws if that tab has been closed (AGE-184).
    const drawn = drawnTabsRef.current;
    const shown = next === PRIMARY_TAB && !drawn.includes(PRIMARY_TAB)
      ? drawn[0] ?? PRIMARY_TAB
      : next;
    if (shown !== panelRef.current) selectPanel(shown);
  }, [agentViewRunId, focusedRunId, requestAgentView, selectPanel]);

  const toggleAddMenu = () => {
    setAddOpen((o) => {
      const next = !o;
      if (next) {
        listProfiles()
          .then(setProfiles)
          .catch(() => {});
        if (addBtnRef.current) {
          const r = addBtnRef.current.getBoundingClientRect();
          setAddCoords({ top: r.bottom + 4, left: r.left });
        }
      }
      return next;
    });
  };

  // Anchored to where the + was when it was clicked; a resize moves it.
  useDismissOnResize(addOpen, () => setAddOpen(false));

  const spawnTab = async (agent: string) => {
    setAddOpen(false);
    if (!focusedRunId) return;
    try {
      const s = await startRunSession(focusedRunId, agent);
      setSessions((prev) => [...prev, s]);
      selectPanel(s.id);
    } catch (e) {
      toastError(e, "Couldn't open agent tab");
    }
  };

  const reopenPrimary = async () => {
    setAddOpen(false);
    if (!focusedRunId) return;
    try {
      await reopenRunAgent(focusedRunId);
      await refreshRuns();
      selectPanel(PRIMARY_TAB);
    } catch (e) {
      toastError(e, "Couldn't reopen the agent");
    }
  };

  // The tab strip scrolls horizontally (VS Code-style): vertical wheel input
  // pans it, and the active tab is kept in view when tabs change.
  const tabsScrollRef = useRef<HTMLDivElement>(null);
  const onTabsWheel = (e: React.WheelEvent<HTMLDivElement>) => {
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) e.currentTarget.scrollLeft += e.deltaY;
  };
  useEffect(() => {
    tabsScrollRef.current
      ?.querySelector(".session-tab.on")
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [panel, sessions.length]);
  const [railOpen, setRailOpen] = useState(true);
  const rail = usePaneWidth("rail", 312, 220, 520);
  // Companion terminal (bottom panel) — height shared across runs, but the
  // open-state is per-run (per-worktree): keyed by the focused run id so
  // toggling one agent's terminal doesn't flip every other agent's.
  const shellPane = usePaneWidth("focus-shell-h", 240, SHELL_MIN, SHELL_MAX);
  const shellFoldKey = focusedRunId ? `${SHELL_FOLD_KEY}:${focusedRunId}` : null;
  const [shellOpen, setShellOpen] = useState<boolean>(false);
  useEffect(() => {
    setShellOpen(
      shellFoldKey && typeof localStorage !== "undefined"
        ? loadFold(localStorage, shellFoldKey, false)
        : false,
    );
  }, [shellFoldKey]);
  const toggleShell = () =>
    setShellOpen((o) => {
      const next = !o;
      try {
        if (shellFoldKey && typeof localStorage !== "undefined")
          saveFold(localStorage, shellFoldKey, next);
      } catch { /* ignore quota / security errors */ }
      return next;
    });
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;
  const captureFirstPrompt = useFirstPromptCapture(focused?.id, focused?.title);

  // The one session in this workspace serving a browser GUI, if any. It is the
  // primary agent's session most of the time, but a web agent can also be
  // opened as an extra tab, and then the tab owns the port — so the backend
  // names the session rather than the UI assuming the run's own.
  const guiSession = focused?.guiSessionId ?? null;
  // Which session's terminal the visible tab shows. Null means the GUI owns the
  // view: on the web agent's own tab the browser app *is* the pane, and its
  // terminal is one tab over, on LOG_TAB.
  const termId = (() => {
    if (!focused) return null;
    if (panel === LOG_TAB) return guiSession;
    const session = panel === PRIMARY_TAB ? focused.id : panel;
    return session === guiSession ? null : session;
  })();
  const guiVisible = guiSession != null && termId === null;
  // The GUI session's own status, not the run's: the web agent may be an extra
  // tab, in which case the run's primary session says nothing about it. A tab
  // the sessions poll has not caught up with yet reads as gone, which is the
  // safe answer — `ensure_run_active` is a no-op on a session already up.
  const guiStatus: SessionStatus =
    guiSession == null ? { state: "gone" }
    : guiSession === focused?.id ? focused.status
    : sessions.find((s) => s.id === guiSession)?.status ?? { state: "gone" };
  // Read by the tab-restore effect, which must not re-run every time the GUI
  // comes and goes: a remembered Log tab is only valid while one is served.
  const guiSessionRef = useRef<string | null>(guiSession);
  guiSessionRef.current = guiSession;
  const primaryClosed = focused?.primaryClosed ?? false;
  primaryClosedRef.current = primaryClosed;
  // Tabs the strip is drawing, in the order it draws them: the run's own agent
  // unless that tab has been closed, then every extra session that has not
  // gone. What "one left" means for the close controls below (AGE-184) — the
  // backend refuses a close that would empty this list, and a ✕ that only ever
  // produces that refusal is a ✕ that should not be there.
  const drawnTabs = [
    ...(primaryClosed ? [] : [PRIMARY_TAB]),
    // Every tab the run has a row for, running or not. The strip used to drop
    // a tab whose session had gone, which quitting the app makes true of all
    // of them (the quit kills every session and shuts the daemon down): three
    // agents in a worktree came back from a restart as one. A tab is the row,
    // not the process — same as the run's own tab above, which was never
    // dropped for being dead — and selecting one revives it.
    ...sessions.map((s) => s.id),
  ];
  drawnTabsRef.current = drawnTabs;
  const canCloseTabs = drawnTabs.length > 1;
  // A tab id the strip actually draws. Anything the run remembers — the tab it
  // was opened on, the one the Run panel backs out to — can name the primary
  // agent, which is not drawn once its tab is closed (AGE-184), so every such
  // hand-back goes through here rather than stranding an empty pane.
  const visibleTab = (t: string) =>
    t === RUN_TAB || t === LOG_TAB || drawnTabs.includes(t) ? t : drawnTabs[0] ?? PRIMARY_TAB;
  // Is the visible pane an agent, or a plain shell? Session tabs carry both:
  // "New terminal" spawns the reserved "shell" profile. Only a shell wants
  // xterm's wheel-to-arrow fallback; in an agent those arrows walk the prompt
  // history (AGE-15).
  const panelIsShell =
    termId != null && sessions.some((s) => s.id === termId && s.agent === "shell");

  // Issue chip (one-stop Phase 7): a run dispatched from an issue links back
  // to it in the header. One-shot lookup on focus change — the label needs
  // the project's issue key and the issue's seq, neither of which rides on
  // RunInfo.
  const [issueChip, setIssueChip] = useState<{ label: string; title: string; issueId: string; projectId: string } | null>(null);
  const chipIssueId = focused?.kind === "agent" ? focused.issueId : null;
  const chipProjectId = focused?.projectId ?? null;
  useEffect(() => {
    setIssueChip(null);
    if (!chipIssueId || !chipProjectId) return;
    let live = true;
    (async () => {
      const project = (await listProjects()).find((p) => p.id === chipProjectId);
      if (!project) return;
      const issue = (await listIssues(project.id)).find((i) => i.id === chipIssueId);
      if (live && issue) {
        setIssueChip({
          label: issueLabel(project, issue),
          title: issue.title,
          issueId: issue.id,
          projectId: project.id,
        });
      }
    })().catch(() => {});
    return () => { live = false; };
  }, [chipIssueId, chipProjectId]);

  return (
    <div className="focus">
      {railOpen ? (
        <>
          <div className="rail" style={{ width: rail.width, minWidth: rail.width }}>
            <div className="rail-head">
              <AgentAddMenu variant="header" projectId={selectedProjectId ?? undefined} onSpawn={onSpawn ?? createAgent} onTerminal={createTerminal} gitless={gitless} />
              <span className="spacer" />
              <button className="icon-btn" onClick={() => setRailOpen(false)}>«</button>
            </div>
            {runs.map((r) => (
              <RailRow
                key={r.id}
                run={r}
                on={r.id === focusedRunId}
                onSelect={() => { setFocusedRun(r.id); requestAgentView(r.id); }}
                onRename={() => rename(r)}
                onRemove={(action) => remove(r, action)}
                onContextMenu={(e) => openRunMenu(e, r)}
                menuOpen={menuRunId === r.id}
              />
            ))}
            <ArchivedSection />
          </div>
          <Resizer size={rail.width} min={220} max={520} onChange={rail.setWidth} side="left" />
        </>
      ) : (
        <div className="rail-stub">
          <button className="icon-btn" onClick={() => setRailOpen(true)}>»</button>
          <span className="rail-spine">AGENTS · {runs.length}</span>
        </div>
      )}

      <div className="focus-main">
        {focused ? (
          focused.kind === "terminal" ? (
            <>
              <div className="focus-head">
                <span className="badge">terminal</span>
                <span className="focus-name" title={focused.title || "terminal"}>{focused.title || "terminal"}</span>
                <span className="spacer" />
                <OverflowMenu
                  items={[
                    { label: "Rename terminal", icon: <PencilIcon />, onSelect: () => rename(focused) },
                    { label: "Close terminal", icon: <TrashIcon />, danger: true, separator: true, onSelect: () => remove(focused, "delete") },
                  ]}
                />
              </div>
              <FocusTerminal key={focused.id} runId={focused.id} />
            </>
          ) : (
            <>
              {/* One row, always: identity on the left, the one primary action
                  on the right, everything else behind "…". The middle group is
                  the only thing allowed to shrink, and it truncates rather than
                  wrapping the header onto a second line. */}
              <div className="focus-head">
                <span className={badgeClass(focused.agent)}>{focused.agent}</span>
                {focused.title && <span className="focus-name" title={focused.title}>{focused.title}</span>}
                <div className="focus-head-meta">
                  {inGitlessFolder(focused) ? (
                    <span className="badge" title="Works in the project folder, which is not a git repository">
                      in folder
                    </span>
                  ) : (
                    <>
                      <span className="branch-chip" title={`Branch: ${focused.branch}`}>
                        <BranchIcon />
                        <span className="branch-chip-name">{focused.branch}</span>
                      </span>
                      {!focused.worktree && (
                        <span className="badge" title="Works in the project checkout, not an isolated worktree">
                          in checkout
                        </span>
                      )}
                    </>
                  )}
                  {issueChip && (
                    <button
                      className="issue-chip"
                      title={issueChip.title}
                      onClick={() => requestNavigate({ kind: "issue", projectId: issueChip.projectId, issueId: issueChip.issueId })}
                    >
                      <span aria-hidden>▧</span> {issueChip.label}
                    </button>
                  )}
                  <QueuedMarker run={focused} />
                </div>
                <span className="spacer" />
                {panel !== RUN_TAB && (
                  <button
                    className={`head-icon-btn ${shellOpen ? "on" : ""}`}
                    title={focused.worktree ? "Toggle terminal in this worktree" : "Toggle terminal in the project checkout"}
                    onClick={toggleShell}
                  ><TerminalIcon /></button>
                )}
                <OverflowMenu
                  items={[
                    { label: "Rename agent", icon: <PencilIcon />, onSelect: () => rename(focused) },
                    // Pin / unpin, the same control the tile carries.
                    ...pinItems(focused, refreshRuns),
                    // A merge writes the branch name into the base branch's
                    // history for good, so this is offered while the branch is
                    // still local. A run working in the project's own checkout
                    // is on the user's branch, not one to rename from here.
                    ...(focused.worktree
                      ? [{ label: "Rename branch…", icon: <BranchIcon />, onSelect: () => renameBranch(focused) }]
                      : []),
                    { label: "Archive agent", icon: <InboxIcon />, separator: true, onSelect: () => remove(focused, "archive") },
                    { label: "Delete agent", icon: <TrashIcon />, danger: true, onSelect: () => remove(focused, "delete") },
                  ]}
                />
                {/* Nothing to approve without a branch of its own: the work is
                    already on the checkout's branch, reviewed in Source Control.
                    Routed through the store rather than a local modal so this
                    button and ⌘↵ open the one Approve window AgentsView owns
                    (AGE-59): the one wired to deep-link a created PR into Source
                    Control instead of out to github.com. */}
                {focused.worktree && (
                  <button className="btn-approve" title="Approve & merge this agent's branch" onClick={() => setApproveRun(focused.id)}>
                    <CheckIcon />
                    <span>Approve</span>
                    <kbd className="btn-approve-kbd">⌘↵</kbd>
                  </button>
                )}
              </div>
              <LoopStrip run={focused} onChanged={refreshRuns} />
              <div className="session-tabs">
                <div className="session-tabs-scroll" ref={tabsScrollRef} onWheel={onTabsWheel}>
                  {/* The run's own agent. Closable like any other tab once it
                      is not the only one left (AGE-184): after a day's work
                      the first agent's context is often the least relevant in
                      the worktree, and it used to be the one tab you were
                      stuck with. The + menu's Reopen brings it back. */}
                  {!primaryClosed && (
                    <button
                      className={`session-tab ${panel === PRIMARY_TAB ? "on" : ""}`}
                      title={`${agentLabel(focused.agent)}: the agent this workspace was created for`}
                      onClick={() => selectPanel(PRIMARY_TAB)}
                    >
                      {agentLabel(focused.agent)}
                      {canCloseTabs && (
                        <span
                          className="tab-close"
                          title="Close this agent tab"
                          onClick={(e) => { e.stopPropagation(); setConfirmCloseTab(focused.id); }}
                        >✕</span>
                      )}
                    </button>
                  )}
                  {guiSession === focused.id && <LogTab panel={panel} onSelect={selectPanel} />}
                  {sessions.map((s) => (
                    <React.Fragment key={s.id}>
                    <button
                      className={`session-tab ${panel === s.id ? "on" : ""}`}
                      title={s.agent === "shell"
                        ? "Terminal: extra shell in this workspace"
                        : `${agentLabel(s.agent)}: extra agent in this workspace`}
                      onClick={() => selectPanel(s.id)}
                    >
                      {s.agent === "shell" ? "≳ terminal" : agentLabel(s.agent)} · {s.id.split("--").pop()}
                      {canCloseTabs && (
                        <span
                          className="tab-close"
                          title="Close this agent tab"
                          onClick={(e) => { e.stopPropagation(); setConfirmCloseTab(s.id); }}
                        >✕</span>
                      )}
                    </button>
                    {guiSession === s.id && <LogTab panel={panel} onSelect={selectPanel} />}
                    </React.Fragment>
                  ))}
                </div>
                <button
                  ref={addBtnRef}
                  className="session-tab-add"
                  title="New agent tab in this workspace"
                  onClick={toggleAddMenu}
                >+</button>
                <span className="spacer" />
                <button
                  className={`session-tab run-tab ${panel === RUN_TAB ? "on" : ""}`}
                  title={focused.runScriptsLive
                    ? "Run: a script is running in this workspace"
                    : "Run this project's scripts in this agent's workspace"}
                  onClick={() => {
                    if (panel !== RUN_TAB) beforeRun.current = panel;
                    selectPanel(RUN_TAB);
                  }}
                >
                  Run
                  {focused.runScriptsLive && <span className="run-dot" aria-hidden />}
                </button>
              </div>
              {addOpen && (
                <>
                  <div className="agent-menu-backdrop" onClick={() => setAddOpen(false)} />
                  <div className="agent-menu" style={{ position: "fixed", ...addCoords }}>
                    {/* The way back from closing the run's own tab (AGE-184).
                        First, and separated, because it is not a new agent:
                        it reopens the one this workspace was created for, on
                        the conversation it already had. */}
                    {primaryClosed && (
                      <>
                        <button onClick={reopenPrimary}>
                          ⟳ Reopen {agentLabel(focused.agent)}
                        </button>
                        <div className="agent-menu-sep" />
                      </>
                    )}
                    {profiles.map((p) => (
                      <button key={p.name} onClick={() => spawnTab(p.name)}>{agentLabel(p.name)}</button>
                    ))}
                    <div className="agent-menu-sep" />
                    {/* "shell" is a reserved session name, not an agent profile —
                        the backend runs the login shell in the worktree for it. */}
                    <button onClick={() => spawnTab("shell")}>≳ New terminal</button>
                  </div>
                </>
              )}
              {panel !== RUN_TAB ? (
                <div className="focus-body">
                  {/* Exactly one of these fills the pane. A web-served agent's
                      tab is its GUI, whole; every other tab (its own Log
                      included) is a terminal. The GUI stays mounted while the
                      log is up so a glance at the server output does not throw
                      away the session in the browser app. */}
                  <div className="focus-body-main">
                    {guiSession && (
                      <GuiPane
                        run={focused}
                        sessionId={guiSession}
                        status={guiStatus}
                        hidden={!guiVisible}
                      />
                    )}
                    {termId && (
                      <FocusTerminal key={termId}
                        runId={termId}
                        altScrollArrows={panelIsShell}
                        // Only the primary tab is the agent the run is named
                        // after; an extra tab or a companion shell is not.
                        onFirstPrompt={panel === PRIMARY_TAB ? captureFirstPrompt : undefined} />
                    )}
                  </div>
                  {shellOpen && (
                    <>
                      <Resizer orientation="horizontal" side="right"
                        size={shellPane.width} min={SHELL_MIN} max={SHELL_MAX} onChange={shellPane.setWidth} />
                      <div className="focus-shell" style={{ height: shellPane.width, flexShrink: 0 }}>
                        <div className="focus-shell-head">
                          <span className="focus-shell-title">≳ terminal · <code>{focused.branch}</code></span>
                          <span className="spacer" />
                          <button className="icon-btn" title="Hide terminal" onClick={toggleShell}>✕</button>
                        </div>
                        <FocusTerminal key={`shell-${focused.id}`} runId={focused.id} stream={shellStream} />
                      </div>
                    </>
                  )}
                </div>
              ) : (
                <RunPanel
                  key={`run-${focused.id}`}
                  target={focused.id}
                  where="this agent's workspace"
                  onClose={() => selectPanel(visibleTab(beforeRun.current))}
                />
              )}
              {confirmCloseTab && (
                <ConfirmDialog
                  title="Close agent tab?"
                  body={confirmCloseTab === focused.id
                    // The run's own agent. Its conversation is not deleted (it
                    // is the agent's own file, and the archive still rescues
                    // it), so the promise here is only that the tab goes and
                    // the session stops.
                    ? `Stop ${agentLabel(focused.agent)}, the agent this workspace was created for. The workspace, its branch and the other tabs are untouched, and the + menu can reopen it on the same conversation.`
                    : "Stop this extra agent session. The workspace, its branch and the other tabs are untouched."}
                  confirmLabel="Close"
                  danger
                  onConfirm={async () => {
                    const sid = confirmCloseTab;
                    setConfirmCloseTab(null);
                    try {
                      await closeRunSession(sid);
                      const left = drawnTabs.filter((t) =>
                        t !== (sid === focused.id ? PRIMARY_TAB : sid));
                      const next = left[0] ?? PRIMARY_TAB;
                      if (sid !== focused.id) setSessions((prev) => prev.filter((s) => s.id !== sid));
                      else await refreshRuns();
                      if (panel === sid || (sid === focused.id && panel === PRIMARY_TAB)) selectPanel(next);
                      if (beforeRun.current === sid || (sid === focused.id && beforeRun.current === PRIMARY_TAB)) {
                        beforeRun.current = next;
                      }
                    } catch (e) {
                      toastError(e, "Close failed");
                    }
                  }}
                  onCancel={() => setConfirmCloseTab(null)}
                />
              )}
            </>
          )
        ) : (
          <div className="board empty">Select an agent from the rail.</div>
        )}
      </div>

      {runMenu}
    </div>
  );
}
