import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { RunStoreProvider, useRuns } from "./store/runs";
import TitleBar from "./components/TitleBar";
import StatusBar from "./components/StatusBar";
import ProjectTree from "./components/ProjectTree";
import AgentsView from "./components/AgentsView";
import Settings from "./components/Settings";
import AgentOnboarding from "./components/AgentOnboarding";
import CommandPalette from "./components/CommandPalette";
import ConfirmDialog from "./components/ConfirmDialog";
import Resizer from "./components/Resizer";
import Toasts from "./components/Toasts";
import { useShortcuts } from "./hooks/useShortcuts";
import { usePaneWidth } from "./hooks/usePaneWidth";
import { FileRoot, Project, QueueNotice, RepoReadiness, RunInfo, agentOnboardingNeeded, checkForUpdate, confirmQuit, createDir, createFile, createRun, ensureWorkspaceGuide, getUpdateCheckEnabled, getWorkspace, gitLogGraph, inspectRepo, listArchivedRuns, listIssues, listProjects, readFile, rememberedModel, setMenuContext, setUiState, writeFile } from "./api";
import { pickDefaultAgent } from "./lib/defaultAgent";
import { PENDING_ISSUE_KEY, PENDING_QUICKADD_KEY, isClosed, issueLabel } from "./lib/issues";
import { NAVIGATE_EVENT, NavTarget } from "./lib/navigate";
import { SectionId } from "./lib/settingsSections";
import { fileRootKey, requestOpenFile } from "./lib/openFile";
import { requestFind, requestFindStep } from "./lib/findBus";
import { DAILY_TEMPLATE_PATH, JOURNAL_DIR, dailyNotePath, defaultDailyContent, renderDailyTemplate } from "./lib/dailyNote";
import { WEEKLY_DIR, buildWeeklyNote, isoWeekStamp, isoWeekStart, weeklyNotePath } from "./lib/weeklyNote";
import { toastError, toastInfo } from "./lib/toast";
import { workspaceHidden } from "./lib/workspacePref";
import { Removal } from "./lib/runRemoval";
import PreviewKeeper from "./components/PreviewKeeper";
import RepoSetupDialog from "./components/RepoSetupDialog";
import RunRemoveDialog from "./components/RunRemoveDialog";
import { PROJECTS_CHANGED_EVENT } from "./lib/projectEvents";

const REPO_URL = "https://github.com/TennnisAI/Agency";

function Shell() {
  const { selectedProjectId, setSelectedProject, createAgent, createTerminal, setTab, focusedRunId, selectedRunId, onScreenRunId, setApproveRun, setFocusedRun, setView, requestAgentView, runs } = useRuns();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [project, setProject] = useState<Project | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showSettings, setShowSettings] = useState(false);
  // Which Settings section to land on, for the callers that mean a particular
  // one: the empty backlog's own "set up sharing" button, and anything like it.
  // null leaves Settings on whichever section it was last left at.
  const [settingsSection, setSettingsSection] = useState<SectionId | null>(null);
  // A newer release exists on GitHub. Dots the Settings button; the actual
  // download link lives in Settings ▸ Diagnostics.
  const [updateAvailable, setUpdateAvailable] = useState(false);
  // Menu-driven archive/discard of the focused agent, awaiting the same
  // confirm dialog the tile, the rail and the focus header use.
  // null = no confirmation showing.
  const [agentAction, setAgentAction] = useState<{ action: Removal; run: RunInfo } | null>(null);
  // Sessions the quit would stop (from the backend's quit-requested event);
  // null = no quit confirmation showing.
  const [quitPrompt, setQuitPrompt] = useState<number | null>(null);
  const sidebar = usePaneWidth("sidebar", 266, 200, 460);

  // Picks the agent a menu/shortcut "New Agent" spawns — the shared pick
  // order in lib/defaultAgent (Settings default → project's last-used → claude).
  async function newTaskDefaultAgent() {
    if (!selectedProjectId) return;
    const agent = await pickDefaultAgent(selectedProjectId, project?.default_agent);
    // This path normally takes the Settings default for isolation, but a folder
    // with no repository has nothing to cut a worktree from, so the agent works
    // in it. The in-view add menu forces the same thing; this is its copy for
    // the menu bar and the keyboard shortcut.
    const r = project ? await inspectRepo(project.repo_path).catch(() => null) : null;
    // A folder that has gone from disk is not a folder to work in: without this
    // the "no repository, so work in the folder itself" arm below started an
    // agent against a path that is not there (AGE-203).
    if (r?.state === "missing") {
      toastInfo(`${project?.name ?? "This project"} can't start an agent: its folder is missing.`);
      return;
    }
    if (r?.state === "notARepo") {
      createAgent(agent, { base: "HEAD", mergeTarget: "", worktree: false });
      return;
    }
    createAgent(agent);
  }

  // ⌘⇧D / File ▸ Today's Note / palette: open today's journal note in the
  // workspace, creating the note (from templates/daily.md when present) — and,
  // on very first use, the workspace itself via ProjectTree's create dialog,
  // which resumes this flow through the agency:workspace-ready event.
  async function openDailyNote() {
    // Hidden means "I don't use this" — respect it rather than resurrecting
    // the workspace from a stray shortcut press.
    if (workspaceHidden()) {
      toastInfo("The workspace is hidden. Turn it back on in Settings ▸ Workspace.");
      return;
    }
    const ws = await getWorkspace().catch(() => null);
    if (!ws) {
      window.dispatchEvent(new CustomEvent("agency:create-workspace", { detail: { intent: "daily-note" } }));
      return;
    }
    const root: FileRoot = { kind: "project", id: ws.id };
    const now = new Date();
    const path = dailyNotePath(now);
    try {
      const existing = await readFile(root, path).catch(() => null);
      if (!existing) {
        let content = defaultDailyContent(now);
        const tpl = await readFile(root, DAILY_TEMPLATE_PATH).catch(() => null);
        if (tpl && !tpl.binary && !tpl.tooLarge && tpl.text.trim()) {
          content = renderDailyTemplate(tpl.text, now);
        }
        await createDir(root, JOURNAL_DIR).catch(() => { /* already exists */ });
        await createFile(root, path);
        await writeFile(root, path, content);
      }
    } catch (e) {
      toastError(e, "Couldn't open today's note");
      return;
    }
    // Stamp the last-open note so a freshly mounted DocsView restores straight
    // to it; the event covers the already-mounted case.
    try { localStorage.setItem(`docs:last:${ws.id}`, path); } catch { /* storage unavailable */ }
    selectProject(ws);
    setTab("docs");
    window.dispatchEvent(new CustomEvent("agency:open-note", { detail: { projectId: ws.id, path } }));
  }
  const dailyRef = useRef(openDailyNote);
  dailyRef.current = openDailyNote;

  // Palette "Workspace Guide": bring back (or just open) the seeded
  // Welcome.md that demos links, properties, tasks, and the journal.
  async function openWorkspaceGuide() {
    if (workspaceHidden()) {
      toastInfo("The workspace is hidden. Turn it back on in Settings ▸ Workspace.");
      return;
    }
    const ws = await getWorkspace().catch(() => null);
    if (!ws) {
      window.dispatchEvent(new CustomEvent("agency:create-workspace", { detail: { intent: "workspace-guide" } }));
      return;
    }
    try {
      const path = await ensureWorkspaceGuide(ws.id);
      try { localStorage.setItem(`docs:last:${ws.id}`, path); } catch { /* storage unavailable */ }
      selectProject(ws);
      setTab("docs");
      window.dispatchEvent(new CustomEvent("agency:open-note", { detail: { projectId: ws.id, path } }));
    } catch (e) {
      toastError(e, "Couldn't open the guide");
    }
  }
  const guideRef = useRef(openWorkspaceGuide);
  guideRef.current = openWorkspaceGuide;

  // Narration offer after a fresh weekly note (confirm), and the repo-setup
  // step when the workspace has uncommitted work (the note itself, usually).
  const [weeklyNarrate, setWeeklyNarrate] = useState<{ ws: Project; path: string } | null>(null);
  const [weeklySetup, setWeeklySetup] = useState<{ ws: Project; path: string; readiness: RepoReadiness } | null>(null);

  // File ▸ Generate Weekly Note / palette: assemble journal/weekly/2026-W31.md
  // from local data — merges (git log per project), issues closed (index
  // status + updated), agent runs archived — then open it in the workspace.
  // An existing note for this week is opened untouched: narration and hand
  // edits live there, regeneration would clobber them.
  async function generateWeeklyNote() {
    if (workspaceHidden()) {
      toastInfo("The workspace is hidden. Turn it back on in Settings ▸ Workspace.");
      return;
    }
    const ws = await getWorkspace().catch(() => null);
    if (!ws) {
      window.dispatchEvent(new CustomEvent("agency:create-workspace", { detail: { intent: "weekly-note" } }));
      return;
    }
    const root: FileRoot = { kind: "project", id: ws.id };
    const now = new Date();
    const path = weeklyNotePath(now);
    const existing = await readFile(root, path).catch(() => null);
    if (existing) {
      toastInfo("This week's note already exists. Opening it.");
    } else {
      try {
        const weekStartSec = Math.floor(isoWeekStart(now).getTime() / 1000);
        const projects = await listProjects();
        const merges: { project: string; subjects: string[] }[] = [];
        const issuesClosed: { label: string; title: string }[] = [];
        const runsArchived: { id: string; title: string; project: string }[] = [];
        // Per-project fetches in parallel; a failing project contributes
        // nothing rather than failing the note.
        await Promise.all(projects.map(async (p) => {
          const [log, issues, archived] = await Promise.all([
            gitLogGraph(`project:${p.id}`, 300).catch(() => []),
            listIssues(p.id).catch(() => []),
            listArchivedRuns(p.id).catch(() => []),
          ]);
          merges.push({
            project: p.name,
            subjects: log
              .filter((c) => c.parents.length > 1 && c.date >= weekStartSec)
              .map((c) => c.subject),
          });
          for (const i of issues) {
            if (isClosed(i.status) && i.updatedAt >= weekStartSec) {
              issuesClosed.push({ label: issueLabel(p, i), title: i.title });
            }
          }
          for (const r of archived) {
            if ((r.archivedAt ?? 0) >= weekStartSec) {
              runsArchived.push({ id: r.id, title: r.title ?? r.prompt.slice(0, 60), project: p.name });
            }
          }
        }));
        merges.sort((a, b) => a.project.localeCompare(b.project));
        await createDir(root, JOURNAL_DIR).catch(() => { /* already exists */ });
        await createDir(root, WEEKLY_DIR).catch(() => { /* already exists */ });
        await createFile(root, path);
        await writeFile(root, path, buildWeeklyNote(isoWeekStamp(now), { merges, issuesClosed, runsArchived }));
      } catch (e) {
        toastError(e, "Couldn't generate the weekly note");
        return;
      }
    }
    try { localStorage.setItem(`docs:last:${ws.id}`, path); } catch { /* storage unavailable */ }
    selectProject(ws);
    setTab("docs");
    window.dispatchEvent(new CustomEvent("agency:open-note", { detail: { projectId: ws.id, path } }));
    // An agent can be dispatched on the note either way: on a branch when the
    // workspace uses git, in the folder itself when it doesn't.
    if (!existing) setWeeklyNarrate({ ws, path });
  }
  const weeklyRef = useRef(generateWeeklyNote);
  weeklyRef.current = generateWeeklyNote;

  async function dispatchWeeklyNarration(ws: Project, path: string, worktree = true) {
    try {
      const agent = await pickDefaultAgent(ws.id, ws.default_agent);
      const prompt = `Narrate the weekly review note \`${path}\`. Read it, then write a short narrative summary of the week into its Notes section, drawing on the listed merges, closed issues, and archived runs. Keep the existing sections and wikilinks intact.`;
      // No picker on this path, so it repeats whatever that agent last ran on.
      const run = await createRun(
        ws.id, prompt, agent, await rememberedModel(agent), "HEAD", null, undefined, worktree,
      );
      openRun(ws, run.id);
    } catch (e) {
      toastError(e, "Couldn't start agent");
    }
  }

  // Confirmed narration: same readiness gate as issue dispatch — a dirty
  // workspace (the fresh note is uncommitted) goes through RepoSetupDialog
  // so the note is committed and visible in the agent's worktree.
  async function confirmWeeklyNarration() {
    if (!weeklyNarrate) return;
    const { ws, path } = weeklyNarrate;
    setWeeklyNarrate(null);
    const r = await inspectRepo(ws.repo_path).catch(() => null);
    if (!r) {
      toastInfo("Couldn't read the workspace folder.");
      return;
    }
    // The workspace folder itself has gone (AGE-203): the note this would
    // narrate was written into a folder that is no longer there.
    if (r.state === "missing") {
      toastInfo("The workspace folder is missing. Open the workspace to reconnect it.");
      return;
    }
    // With no repository the agent edits the note where it lies, so there is no
    // worktree for the uncommitted note to be missing from and nothing to set up.
    if (r.state === "notARepo") return dispatchWeeklyNarration(ws, path, false);
    if (r.state === "ready" && !r.dirty) await dispatchWeeklyNarration(ws, path);
    else setWeeklySetup({ ws, path, readiness: r });
  }

  // Settings, optionally at a named section. Every caller that just wants the
  // screen passes nothing and keeps the section it was last left at; a caller
  // that is answering "where do I turn this on" names the section, so the
  // answer is on screen rather than thirteen sections away.
  function openSettings(section?: SectionId) {
    setSettingsSection(section ?? null);
    setShowSettings(true);
  }

  useShortcuts({
    onNewTask: () => { newTaskDefaultAgent(); },
    onSource: () => setTab("source"),
    onDailyNote: () => { openDailyNote(); },
    onApprove: () => {
      // Approve/merge is an agent-only workflow; terminals have no branch to merge.
      const focused = runs.find((r) => r.id === focusedRunId);
      if (focused?.kind === "agent") setApproveRun(focused.id);
    },
    onPalette: () => setPaletteOpen(true),
    onSettings: () => openSettings(),
  });

  // ⌘F / ⌥⌘F normally arrive as Edit-menu actions — macOS gives the menu bar
  // the key before the webview ever sees it. This is the fallback for the paths
  // that don't go through the menu, and it listens in the capture phase on
  // purpose: CodeMirror binds ⌘F too, and stopping the event here is what keeps
  // its own search panel from opening behind the app's find bar.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.shiftKey || e.key.toLowerCase() !== "f") return;
      if (!requestFind(e.altKey ? "replace" : "find")) return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  // One passive release check per launch, when the user hasn't opted out. It
  // only lights the dot on Settings — Agency never downloads or installs
  // anything on its own, so this can't disturb running agents. Failures are
  // silent by design: being offline is not something to interrupt anyone about.
  useEffect(() => {
    let cancelled = false;
    getUpdateCheckEnabled()
      .then((enabled) => (enabled ? checkForUpdate() : null))
      .then((res) => {
        if (!cancelled && res?.updateAvailable) setUpdateAvailable(true);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  // Keep the backend's picture of "what the user can see" current: the window's
  // focus plus the run whose pane is on screen. Together they decide the one
  // run that stays quiet — every other agent notifies, in-app or not.
  useEffect(() => {
    const report = () => setUiState(document.hasFocus(), onScreenRunId).catch(() => {});
    report();
    window.addEventListener("focus", report);
    window.addEventListener("blur", report);
    return () => {
      window.removeEventListener("focus", report);
      window.removeEventListener("blur", report);
    };
  }, [onScreenRunId]);

  // Keep the native menu's context items (New Agent/Terminal, Source, and the
  // Agent menu) enabled only when they'd actually do something, so they aren't
  // clickable no-ops. The Agent menu tracks a focused *agent* — a focused
  // terminal doesn't count (it has no branch to approve/merge). Depending on
  // the derived booleans (not `runs`, whose identity changes every poll) keeps
  // this to one IPC call per actual state change.
  const hasProject = !!selectedProjectId;
  const hasFocusedAgent = runs.some((r) => r.id === focusedRunId && r.kind === "agent");
  useEffect(() => {
    setMenuContext(hasProject, hasFocusedAgent).catch(() => {});
  }, [hasProject, hasFocusedAgent]);

  // Picking a project from the tree leaves Settings up (AGE-187). Settings has
  // a Project group whose sections are all about one checkout, and its empty
  // state asks you to choose one in the sidebar; closing the screen out from
  // under that click made the request impossible to satisfy. Same reasoning as
  // leaveProject below. This is the tree's plain selection only: the tree also
  // selects on the way to starting an agent, and that goes to selectProject
  // through its onOpen, because the run it focuses has to be visible.
  function selectProjectFromTree(p: Project) {
    setProject(p);
    setSelectedProject(p.id);
  }

  // Point the app at a project on the way to showing something inside it, so
  // this closes Settings. Every caller here sets a tab and then dispatches at
  // it: ⌘⇧D's daily note, the weekly note, the palette's Workspace Guide, a
  // note/file/issue link, a tray click. With Settings left up they all landed
  // behind it, the work done and nothing on screen but Settings.
  function selectProject(p: Project) {
    setShowSettings(false);
    selectProjectFromTree(p);
  }

  // The selected project went away under the view — closed, or deleted along
  // with its worktrees. Lands on the all-projects overview like goHome, but
  // leaves Settings up: hiding the workspace closes it from inside Settings,
  // and dismissing that screen mid-toggle would just be a second surprise.
  function leaveProject() {
    setProject(null);
    setSelectedProject(null);
  }

  // Back to the all-projects overview (the no-project state).
  function goHome() {
    leaveProject();
    setShowSettings(false);
  }

  // Open a specific agent straight into its focus view. setSelectedProject
  // resets view/focus *and* restores the project's last-viewed tab, so the tab,
  // focus and view calls must all follow it; React batches them in this handler,
  // leaving the run focused in the Agents tab. Without the setTab, opening a run
  // from the sidebar while the project last sat on Docs or Files looked like a
  // dead click: the run was focused behind a tab the user couldn't see.
  // requestAgentView is the same problem one level in: a run left on its Run tab
  // reopens there, so opening it by name landed on the run panel rather than the
  // agent, and a run already focused didn't move at all.
  function openRun(p: Project, runId: string) {
    setShowSettings(false);
    setProject(p);
    setSelectedProject(p.id);
    setTab("agents");
    setFocusedRun(runId);
    setView("focus");
    requestAgentView(runId);
  }

  // Route a native-menu action (payload of the backend "menu" event) to the
  // same handlers the buttons/shortcuts use, so the menu bar never drifts from
  // the UI. Kept in a ref (below) so the event listener, bound once, always
  // calls the version closed over the latest state.
  function onMenu(action: string) {
    switch (action) {
      case "settings": openSettings(); break;
      case "palette": setPaletteOpen(true); break;
      case "new-agent": newTaskDefaultAgent(); break;
      case "new-terminal": createTerminal(); break;
      // The full add flow (dir picker + repo-setup dialog) lives in ProjectTree;
      // signal it rather than duplicating that logic here.
      case "add-project": window.dispatchEvent(new CustomEvent("agency:add-project")); break;
      case "clone-project": window.dispatchEvent(new CustomEvent("agency:clone-project")); break;
      case "source": setTab("source"); break;
      // Find routes to whichever surface is on screen and focused — the notes
      // editor, the file editor, an issue description, the issue board's
      // filter, or a terminal's scrollback (see lib/findBus).
      case "find": requestFind("find"); break;
      case "replace": requestFind("replace"); break;
      case "find-next": requestFindStep(false); break;
      case "find-prev": requestFindStep(true); break;
      case "daily-note": openDailyNote(); break;
      case "weekly-note": void generateWeeklyNote(); break;
      case "workspace-guide": void openWorkspaceGuide(); break;
      // Palette-only actions (no native-menu counterpart) share this router so
      // every palette command goes through exactly one switch.
      case "new-issue": sessionStorage.setItem(PENDING_QUICKADD_KEY, "1"); setTab("issues"); break;
      case "go-agents": setTab("agents"); break;
      case "go-issues": setTab("issues"); break;
      case "go-docs": setTab("docs"); break;
      case "go-files": setTab("files"); break;
      case "toggle-sidebar": setSidebarOpen((s) => !s); break;
      case "home": goHome(); break;
      case "approve": {
        // Approve/merge is agent-only — terminals have no branch to merge.
        const focused = runs.find((r) => r.id === focusedRunId);
        if (focused?.kind === "agent") setApproveRun(focused.id);
        break;
      }
      case "archive": {
        const focused = runs.find((r) => r.id === focusedRunId);
        if (focused?.kind === "agent") setAgentAction({ action: "archive", run: focused });
        break;
      }
      case "discard": {
        const focused = runs.find((r) => r.id === focusedRunId);
        if (focused) setAgentAction({ action: "delete", run: focused });
        break;
      }
      case "report-issue": openUrl(`${REPO_URL}/issues/new`).catch(() => {}); break;
      case "github": openUrl(REPO_URL).catch(() => {}); break;
    }
  }
  const menuRef = useRef(onMenu);
  menuRef.current = onMenu;

  // Cross-domain navigation (one-stop Phase 7): mentions panels, wikilinks,
  // and the focus header's issue chip all land here. Same handoffs the
  // palette uses: PENDING_ISSUE_KEY for issues, docs:last + agency:open-note
  // for notes (the stamp covers a freshly mounting DocsView, the event the
  // already-mounted one).
  async function onNavigate(target: NavTarget) {
    const p = (await listProjects().catch(() => [])).find((x) => x.id === target.projectId);
    if (!p) return;
    switch (target.kind) {
      case "issue":
        sessionStorage.setItem(PENDING_ISSUE_KEY, target.issueId);
        selectProject(p);
        setTab("issues");
        break;
      case "run":
        openRun(p, target.runId);
        break;
      case "note":
        try { localStorage.setItem(`docs:last:${p.id}`, target.path); } catch { /* storage unavailable */ }
        selectProject(p);
        setTab("docs");
        window.dispatchEvent(new CustomEvent("agency:open-note", { detail: { projectId: p.id, path: target.path } }));
        break;
      // Docs is always rooted at the project checkout, so the Files tab has to
      // be too, or the request would be addressed to a root that isn't showing
      // and sit in the pending slot forever. selectProject drops the run
      // selection, which is what points the Files tab back at the checkout.
      case "file":
        selectProject(p);
        setTab("files");
        requestOpenFile({ rootKey: fileRootKey({ kind: "project", id: p.id }), path: target.path });
        break;
    }
  }
  const navRef = useRef(onNavigate);
  navRef.current = onNavigate;

  // The selected project's row is a copy taken when it was clicked, and every
  // panel renders against it. A row that changes underneath (a sync adopting
  // the shared backlog's issue key, Settings renaming it) has to be re-read,
  // or the board labels, links and conflict markers keep the old key until
  // the app restarts. Replaced only when still selected and actually changed,
  // so an unchanged row does not re-render every panel.
  useEffect(() => {
    const onProjects = async () => {
      const fresh = await listProjects().catch(() => null);
      if (!fresh) return;
      setProject((prev) => {
        const next = prev && fresh.find((p) => p.id === prev.id);
        return next && JSON.stringify(next) !== JSON.stringify(prev) ? next : prev;
      });
    };
    window.addEventListener(PROJECTS_CHANGED_EVENT, onProjects);
    return () => window.removeEventListener(PROJECTS_CHANGED_EVENT, onProjects);
  }, []);

  // Backend-driven navigation: quit confirmations, tray-menu and app-menu
  // clicks arrive as Tauri events.
  useEffect(() => {
    const subs = [
      listen<number>("quit-requested", (e) => setQuitPrompt(e.payload)),
      listen<string>("menu", (e) => menuRef.current(e.payload)),
      listen<{ projectId: string; runId: string }>("tray-open-run", async (e) => {
        const p = (await listProjects().catch(() => [])).find((x) => x.id === e.payload.projectId);
        if (p) openRun(p, e.payload.runId);
      }),
      listen<string>("tray-open-project", async (e) => {
        const p = (await listProjects().catch(() => [])).find((x) => x.id === e.payload);
        if (p) selectProject(p);
      }),
      // A queued message that stopped being queued in a way the user cannot
      // see: dropped for a session that has gone, or appended to whatever was
      // on the prompt line after five minutes of waiting. The run's marker
      // covers a message while it is held; nothing covered the moment it stops
      // being held, and a marker that appears for one tick and then vanishes
      // reads as the message having gone in.
      listen<QueueNotice>("send-queue-notice", (e) => {
        const n = e.payload;
        // Dropped is a loss, not a status change: the red toast, and the long
        // dismissal, are the point.
        if (n.kind === "dropped") toastError(n.text);
        else toastInfo(n.text, 8000);
        // Nothing to refresh: the run list polls every 1.5s, so the marker
        // count follows on its own.
      }),
    ];
    // Palette "Today's Note" and the post-creation resume of that flow arrive
    // as DOM events (the palette can't call into Shell directly).
    const daily = () => { void dailyRef.current(); };
    const ready = (e: Event) => {
      const intent = (e as CustomEvent<{ intent?: string }>).detail?.intent;
      if (intent === "daily-note") void dailyRef.current();
      else if (intent === "weekly-note") void weeklyRef.current();
      else if (intent === "workspace-guide") void guideRef.current();
    };
    const navigate = (e: Event) => {
      const detail = (e as CustomEvent<NavTarget>).detail;
      if (detail) void navRef.current(detail);
    };
    window.addEventListener("agency:daily-note", daily);
    window.addEventListener("agency:workspace-ready", ready);
    window.addEventListener(NAVIGATE_EVENT, navigate);
    // The .catch matters: Tauri's injected event plugin throws
    // ("listeners[eventId].handlerId") when an unlisten races a listener that
    // was already removed (e.g. across remounts). A failed cleanup of a dead
    // listener is a no-op — don't let it surface as an error toast.
    return () => {
      subs.forEach((s) => s.then((un) => un()).catch(() => {}));
      window.removeEventListener("agency:daily-note", daily);
      window.removeEventListener("agency:workspace-ready", ready);
      window.removeEventListener(NAVIGATE_EVENT, navigate);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="shell">
      <TitleBar onOpenPalette={() => setPaletteOpen(true)} />
      <div className="body">
        {/* The sidebar stays mounted while hidden (collapsed to width 0) so
            re-expanding is instant: remounting the tree used to leave the pane
            empty while projects/readiness refetched over IPC. */}
        <div
          className={`sidebar-wrap${sidebarOpen ? "" : " closed"}`}
          style={{ width: sidebarOpen ? sidebar.width : 0 }}
        >
          <div className="sidebar-fix" style={{ width: sidebar.width }}>
            <ProjectTree
              selectedId={selectedProjectId}
              focusedRunId={focusedRunId}
              onSelect={selectProjectFromTree}
              onOpen={selectProject}
              onSelectRun={(p: Project, run: RunInfo) => openRun(p, run.id)}
              onHome={goHome}
              onSelectionGone={leaveProject}
              onToggleSidebar={() => setSidebarOpen(false)}
              onOpenSettings={() => openSettings()}
              updateAvailable={updateAvailable}
            />
          </div>
        </div>
        {sidebarOpen && (
          <Resizer size={sidebar.width} min={200} max={460} onChange={sidebar.setWidth} side="left" />
        )}
        {showSettings ? (
          <Settings
            onClose={() => setShowSettings(false)}
            section={settingsSection}
            onOpenTerminal={(runId) => project && openRun(project, runId)}
            projectId={selectedProjectId}
            projectName={project?.name ?? null}
          />
        ) : (
          <AgentsView
            project={project}
            onOpenSettings={openSettings}
            sidebarOpen={sidebarOpen}
            onToggleSidebar={() => setSidebarOpen(true)}
            onOpenRun={openRun}
            onOpenProject={selectProject}
          />
        )}
      </div>
      <StatusBar
        projectName={project?.name ?? null}
        runId={selectedRunId}
        projectId={selectedProjectId}
        onOpenSource={() => setTab("source")}
        // Unpushed commits are on the checkout's branch, so this has to leave
        // the agent first: the grid is what points source control there. The
        // run stays focused, so the Focus button goes straight back to it.
        onOpenCheckout={() => { setView("grid"); setTab("source"); }}
      />
      <Toasts />
      {paletteOpen && (
        <CommandPalette
          onClose={() => setPaletteOpen(false)}
          onAction={(id) => menuRef.current(id)}
          onOpenProject={selectProject}
          onOpenRun={(p, runId) => openRun(p, runId)}
        />
      )}
      {/* The menu only ever acts on the focused run, so the dialog's own
          aftermath (unfocus, back to the grid, source control up) applies. */}
      {agentAction && (
        <RunRemoveDialog
          run={agentAction.run}
          action={agentAction.action}
          onClose={() => setAgentAction(null)}
        />
      )}
      {weeklyNarrate && (
        <ConfirmDialog
          title="Narrate with an agent?"
          body="Dispatch a workspace agent to turn this week's facts into a short written summary in the note's Notes section."
          confirmLabel="Dispatch"
          onConfirm={() => { void confirmWeeklyNarration(); }}
          onCancel={() => setWeeklyNarrate(null)}
        />
      )}
      {weeklySetup && (
        <RepoSetupDialog
          readiness={weeklySetup.readiness}
          context="spawn"
          repoPath={weeklySetup.ws.repo_path}
          onResolved={() => {
            const { ws, path } = weeklySetup;
            setWeeklySetup(null);
            void dispatchWeeklyNarration(ws, path);
          }}
          onCancel={() => setWeeklySetup(null)}
        />
      )}
      {quitPrompt !== null && (
        <ConfirmDialog
          title="Quit Agency?"
          body={quitPrompt > 0
            ? `Quitting will stop ${quitPrompt} running session${quitPrompt === 1 ? "" : "s"}. Agents resume where they left off next time you open Agency.`
            : "All agents are idle; nothing will be interrupted."}
          confirmLabel="Quit"
          danger={quitPrompt > 0}
          onConfirm={() => { confirmQuit().catch(() => {}); }}
          onCancel={() => setQuitPrompt(null)}
        />
      )}
    </div>
  );
}

export default function App() {
  // null = still checking; true = show picker; false = main shell.
  const [needsOnboarding, setNeedsOnboarding] = useState<boolean | null>(null);

  useEffect(() => {
    agentOnboardingNeeded()
      .then(setNeedsOnboarding)
      .catch(() => setNeedsOnboarding(false));
  }, []);

  if (needsOnboarding === null) {
    // Brief check against the backend — show the chrome with a spinner rather
    // than a blank window so launch doesn't flash empty.
    return (
      <div className="shell">
        <TitleBar onOpenPalette={() => {}} bare />
        <div className="app-loading">
          <span className="spinner" />
        </div>
      </div>
    );
  }
  if (needsOnboarding) {
    return (
      <div className="shell">
        <TitleBar onOpenPalette={() => {}} bare />
        <AgentOnboarding onDone={() => setNeedsOnboarding(false)} />
        <Toasts />
      </div>
    );
  }

  return (
    <RunStoreProvider>
      <Shell />
      {/* Invisible hosts that keep agents' previews (and their MCP preview
          tools) alive while the Run tab isn't showing them. */}
      <PreviewKeeper />
    </RunStoreProvider>
  );
}
