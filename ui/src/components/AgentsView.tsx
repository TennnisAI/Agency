import { useEffect, useRef, useState } from "react";
import { Project, FileRoot, projectTarget } from "../api";
import { fileRootKey, onAnyOpenFile, requestOpenFile } from "../lib/openFile";
import { SectionId } from "../lib/settingsSections";
import { terminalHasFocus } from "../lib/terminalFocus";
import { useRuns } from "../store/runs";
import AgentTile from "./AgentTile";
import AgentFocus from "./AgentFocus";
import MergeModal from "./MergeModal";
import GitPanel, { GitSelection } from "./git/GitPanel";
import PrReviewPanel from "./pr/PrReviewPanel";
import AgentAddMenu from "./AgentAddMenu";
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";
import { useRepoReadiness, isGitless } from "../hooks/useRepoReadiness";
import { useFolderMissing } from "../hooks/useFolderMissing";
import { useSpawnAgent } from "../hooks/useSpawnAgent";
import FilesView from "./FilesView";
import DocsView from "./DocsView";
import HomeView from "./HomeView";
import IssuesView from "./IssuesView";
import MapView from "./MapView";
import RunPanel from "./RunPanel";
import ProjectMissingView from "./ProjectMissingView";
import SidebarToggle from "./SidebarToggle";
import RightPanelToggle from "./RightPanelToggle";
import QuickOpen from "./QuickOpen";
import { useElementWidth } from "../hooks/useElementWidth";
import { notifyProjectsChanged } from "../lib/projectEvents";

// Main content area. With a project selected this is that project's agents /
// source control / files; with none it hosts the all-projects overview under
// the same, always-visible header bar.
export default function AgentsView({
  project,
  sidebarOpen,
  onToggleSidebar,
  onOpenRun,
  onOpenProject,
  onOpenSettings,
}: {
  project: Project | null;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onOpenRun: (project: Project, runId: string) => void;
  onOpenProject: (project: Project) => void;
  onOpenSettings: (section?: SectionId) => void;
}) {
  const { runs, projectRunLive, view, setView, focusedRunId, selectedRunId, sourcePanelOpen, setSourcePanelOpen, tab, setTab, approveRunId, setApproveRun, createTerminal, spawning, spawnProgress, setFocusedRun, refreshRuns } = useRuns();
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;
  // The run whose worktree this view works in — nothing while the grid is up.
  const selected = runs.find((r) => r.id === selectedRunId) ?? null;
  // Agents alongside the file tree, the Files-tab twin of the Docs side panel.
  const [filesAgents, setFilesAgents] = useState(false);
  const [gitSel, setGitSel] = useState<GitSelection>(null);
  // Source Control has two sub-views: Changes (the git panel) and Pull Requests
  // (in-app review). `reviewPr` deep-links a specific PR from the Approve window.
  const [srcTab, setSrcTab] = useState<"changes" | "prs">("changes");
  // Files has two of its own: the tree and editor, and the codebase Map.
  const [filesTab, setFilesTab] = useState<"files" | "map">("files");
  const [reviewPr, setReviewPr] = useState<number | null>(null);
  const reviewPane = usePaneWidth("review", 360, 280, 640);

  // A project can have no repository at all: a workspace that declined git, or
  // a plain folder added as a project. Agents and terminals still run there,
  // in the folder itself, but everything that needs a branch to exist (Source
  // Control, the review panel) hides until a repo is initialized.
  const { readiness: projReadiness, refresh: refreshReadiness } = useRepoReadiness(project);
  const gitless = isGitless(projReadiness);
  // A project whose folder has gone from disk entirely. Every tab below reads
  // that folder, so this is checked before any of them is rendered.
  const { missing: folderMissing, refresh: refreshFolder } = useFolderMissing(project);
  // The spawn pre-flight (missing CLI, unready repo) and its dialogs, shared
  // with the agents side panel in the Docs / Files tabs.
  const { spawn, error, dialogs: spawnDialogs } = useSpawnAgent(project, refreshReadiness);
  // The workspace is a markdown vault: the Files tab would duplicate Docs
  // (with a manual-save editor, no less), so it's hidden there entirely.
  const isWorkspace = project?.kind === "workspace";

  // Per-project tab memory can restore "source" from before git was declined,
  // or "files" from before the workspace hid it.
  useEffect(() => {
    if (gitless && tab === "source") setTab("agents");
    if (isWorkspace && (tab === "files" || tab === "run")) setTab("docs");
  }, [gitless, isWorkspace, tab, setTab]);

  // Which working tree source control operates on: the selected run's worktree
  // (terminals share the project checkout) or, standing in the grid with no run
  // selected, the project's own checkout via a "project:<id>" token. Null only
  // at the all-projects home screen.
  const gitRoot = selectedRunId ?? (project ? `project:${project.id}` : null);
  const allowComments = selected?.kind === "agent";
  // A different working tree means a different file list and history, so the
  // open diff doesn't survive the switch.
  useEffect(() => { setGitSel(null); }, [gitRoot]);

  // The root the Files tab shows (and quick-open must list): the selected run's
  // worktree, else the project's own checkout.
  const filesRoot: FileRoot | null = project
    ? selectedRunId
      ? { kind: "run", id: selectedRunId }
      : { kind: "project", id: project.id }
    : null;

  // Source control → Files: the two are rooted on the same working tree (both
  // follow the run selection), so the tab switch plus the open request is the
  // whole move. Undefined in a workspace, which has no Files tab at all — the
  // request would be redirected to Docs and left dangling, so the entries that
  // depend on this are simply not offered there.
  const revealInFilesTab = isWorkspace || !filesRoot ? undefined : (path: string) => {
    setTab("files");
    requestOpenFile({ rootKey: fileRootKey(filesRoot), path, reveal: true });
  };

  // A file was asked for from anywhere (quick open, the palette, a terminal
  // link, the map's own "Open in Files"), so the tree has to be the sub-view
  // showing. Same reason the source panel forces Changes below: with the other
  // sub-view left active the jump landed on it and the file never showed.
  useEffect(() => onAnyOpenFile(() => setFilesTab("files")), []);

  // Where the Docs / Files checkout bar goes when clicked: Source Control on
  // the tree it names. The Files tree is rooted at whatever source control
  // already targets (both follow the run selection, and a run without a
  // worktree resolves to the checkout either way), so the tab is the whole
  // move. Docs is always the project checkout, so from an agent's worktree it
  // has to drop the selection first — the grid is what points source control at
  // the checkout, the same way the status bar's unpushed-commits button does.
  const openFilesCheckout = () => setTab("source");
  const openDocsCheckout = () => {
    if (selected?.worktree) setView("grid");
    setTab("source");
  };

  // ⌘P quick-open, everywhere except the Docs tab — DocsView owns ⌘P there
  // (its note switcher) and is only mounted on that tab, so exactly one
  // handler acts per keypress.
  const [quickOpen, setQuickOpen] = useState(false);
  const quickOpenGate = useRef({ enabled: false });
  // The workspace has no Files tab, so quick-open (which lands there) is off;
  // ⌘P inside Docs is the note switcher.
  quickOpenGate.current.enabled = !!project && tab !== "docs" && !isWorkspace;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "p") {
        // Ctrl+P belongs to the shell whenever a terminal has focus — the
        // Files tab's agents panel hosts one.
        if (!quickOpenGate.current.enabled || terminalHasFocus()) return;
        e.preventDefault();
        setQuickOpen((s) => !s);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => { setQuickOpen(false); }, [project?.id, tab]);

  // Toolbar density. The header never wraps; as it narrows it drops the long
  // form of a label, then the labels themselves, leaving the glyphs (each
  // button keeps a title, so the name is a hover away).
  const [headRef, headWidth] = useElementWidth<HTMLDivElement>();
  // 0 = not measured yet: assume roomy so the first paint isn't collapsed.
  // Thresholds sit just under what each step needs: ~785px for the full row
  // (the Run tab added one more), ~730 once the long labels shorten, ~540 once
  // the tabs are glyphs only.
  const density = headWidth === 0 ? "" : headWidth < 750 ? " is-tight" : headWidth < 850 ? " is-compact" : "";

  // The folder is gone: one screen saying so, with the two things that fix it,
  // in place of six tabs that would each fail differently against a path that
  // is not there. The header keeps only the sidebar toggle, because every
  // control on it (the tabs, the add menu, the panel toggles) acts on the
  // folder.
  if (project && folderMissing) {
    return (
      <main className="agents">
        <div className="content-head" ref={headRef}>
          {!sidebarOpen && <SidebarToggle open={false} onToggle={onToggleSidebar} />}
          {/* The tabs, the view switches and the add menu all act on the
              folder, so none of them belongs here. The name stays: with the
              sidebar closed it is the only thing saying which project this is. */}
          <span className="content-head-name">{project.name}</span>
          <div className="spacer" />
        </div>
        <ProjectMissingView
          project={project}
          onReconnected={() => {
            // The row every panel renders against still holds the old path;
            // this is what re-reads it, and the probe above follows the new one.
            notifyProjectsChanged();
            refreshFolder();
            refreshReadiness();
          }}
          onRemoved={notifyProjectsChanged}
        />
      </main>
    );
  }

  return (
    <main className="agents">
      <div className={`content-head${density}`} ref={headRef}>
        {!sidebarOpen && <SidebarToggle open={false} onToggle={onToggleSidebar} />}
        <div className="seg">
          <button className={tab === "agents" ? "on" : ""} title="Agents" onClick={() => setTab("agents")}>
            <span className="seg-ico" aria-hidden>▦</span><span className="seg-label">Agents</span>
          </button>
          <button className={tab === "issues" ? "on" : ""} title="Issues" onClick={() => setTab("issues")}>
            <span className="seg-ico" aria-hidden>▧</span><span className="seg-label">Issues</span>
          </button>
          <button className={tab === "docs" ? "on" : ""} title="Docs" onClick={() => setTab("docs")}>
            <span className="seg-ico" aria-hidden>▥</span><span className="seg-label">Docs</span>
          </button>
          {!gitless && (
            <button className={tab === "source" ? "on" : ""} title="Source Control" onClick={() => setTab("source")}>
              <span className="seg-ico" aria-hidden>⎇</span>
              <span className="seg-label-short">Source</span>
              <span className="seg-label">Source Control</span>
            </button>
          )}
          {!isWorkspace && (
            <button className={tab === "files" ? "on" : ""} title="Files" onClick={() => setTab("files")}>
              <span className="seg-ico" aria-hidden>▤</span><span className="seg-label">Files</span>
            </button>
          )}
          {!isWorkspace && (
            <button
              className={tab === "run" ? "on" : ""}
              title={projectRunLive
                ? "Run: a script is running in your checkout"
                : "Run the project's scripts in your own checkout"}
              onClick={() => setTab("run")}
            >
              <span className="seg-ico" aria-hidden>▷</span><span className="seg-label">Run</span>
              {/* The checkout's own scripts, not any agent's — those show on
                  their tiles. Survives the density ladder: at the tightest step
                  the label goes, the dot stays. */}
              {projectRunLive && <span className="run-dot" aria-hidden />}
            </button>
          )}
        </div>
        {project && tab === "agents" && (
          <div className="seg">
            <button className={view === "grid" ? "on" : ""} title="Grid view" onClick={() => setView("grid")}>
              <span className="seg-ico" aria-hidden>▦</span><span className="seg-label">Grid</span>
            </button>
            <button className={view === "focus" ? "on" : ""} title="Focus view" onClick={() => setView("focus")}>
              <span className="seg-ico" aria-hidden>▭</span><span className="seg-label">Focus</span>
            </button>
          </div>
        )}
        {project && tab === "source" && (
          <div className="seg">
            <button className={srcTab === "changes" ? "on" : ""} title="Changes" onClick={() => setSrcTab("changes")}>
              <span className="seg-label-short">Diff</span>
              <span className="seg-label">Changes</span>
            </button>
            <button className={srcTab === "prs" ? "on" : ""} title="Pull Requests" onClick={() => setSrcTab("prs")}>
              <span className="seg-label-short">PRs</span>
              <span className="seg-label">Pull Requests</span>
            </button>
          </div>
        )}
        {project && tab === "files" && (
          <div className="seg">
            <button className={filesTab === "files" ? "on" : ""} title="Files" onClick={() => setFilesTab("files")}>
              <span className="seg-label-short">Tree</span>
              <span className="seg-label">Files</span>
            </button>
            <button className={filesTab === "map" ? "on" : ""} title="Map of the codebase" onClick={() => setFilesTab("map")}>
              <span className="seg-label-short">Map</span>
              <span className="seg-label">Map</span>
            </button>
          </div>
        )}
        <div className="spacer" />
        {project && tab === "agents" && !gitless && (
          <RightPanelToggle open={sourcePanelOpen} onToggle={() => setSourcePanelOpen(!sourcePanelOpen)} />
        )}
        {project && tab === "files" && filesTab === "files" && (
          <button
            className={`icon-btn${filesAgents ? " on" : ""}`}
            title={filesAgents ? "Hide agents" : "Show agents alongside the files"}
            onClick={() => setFilesAgents((a) => !a)}
          >▦</button>
        )}
        {project && tab === "agents" && (
          <AgentAddMenu
            projectId={project.id}
            onSpawn={spawn}
            onTerminal={createTerminal}
            gitless={gitless}
          />
        )}
      </div>

      {error && <div className="git-error">{error}</div>}

      {!project ? (
        tab === "agents" || tab === "issues" ? (
          <HomeView onOpenRun={onOpenRun} onOpenProject={onOpenProject} mode={tab === "issues" ? "issues" : "agents"} />
        ) : (
          <div className="board empty">
            {tab === "source"
              ? "Select a project to browse its source control."
              : tab === "docs"
                ? "Select a project to browse its docs."
                : tab === "run"
                  ? "Select a project to run its scripts."
                  : "Select a project to browse its files."}
          </div>
        )
      ) : (
        <>
          {tab === "issues" && (
            <IssuesView
              project={project}
              onStartIssue={(issue, agentId, opts) => spawn(agentId, opts, issue)}
              onOpenBacklogSettings={() => onOpenSettings("backlog")}
            />
          )}

          {tab === "docs" && (
            <div className="source-wrap">
              <DocsView project={project} onOpenCheckout={openDocsCheckout} />
            </div>
          )}

          {tab === "source" && (
            <div className="source-wrap">
              {srcTab === "changes" && gitRoot && (
                <GitPanel taskId={gitRoot} layout="full" selection={gitSel} onSelect={setGitSel}
                  allowComments={allowComments} onRevealInFiles={revealInFilesTab} />
              )}
              {srcTab === "prs" && (
                <PrReviewPanel projectId={project.id} initialPr={reviewPr} onConsumeInitial={() => setReviewPr(null)} />
              )}
            </div>
          )}

          {tab === "files" && (
            <div className="source-wrap">
              {filesTab === "files" && (
                <FilesView root={filesRoot} project={project} agentsOpen={filesAgents} onOpenCheckout={openFilesCheckout} />
              )}
              {/* Keyed by project so a switch tears the map down rather than
                  painting one project's tree under another's breadcrumb. */}
              {filesTab === "map" && <MapView key={project.id} project={project} />}
            </div>
          )}

          {/* The project's own checkout, not any agent's worktree: the same run
              scripts, run where you work. Keyed by project so switching one
              tears the panel (and its log pane) down rather than reusing it. */}
          {tab === "run" && (
            <RunPanel
              key={`run-${project.id}`}
              target={projectTarget(project.id)}
              where="the project checkout"
            />
          )}

          {tab === "agents" && (
            <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
              <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, overflow: "hidden" }}>
                {view === "grid" && (
                  <div className="grid">
                    {runs.length === 0 && !spawning && (
                      <div className="board empty">
                        {gitless
                          ? "No agents yet. Add one with \"+ Agent\"; with no git repository here, they work in the folder itself rather than an isolated branch."
                          : "No agents yet. Add one with \"+ Agent\"."}
                      </div>
                    )}
                    {runs.map((r) => <AgentTile key={r.id} run={r} />)}
                    {spawning && (
                      // Placeholder while the worktree + session are created —
                      // the real tile appears once createRun resolves. On a large
                      // repo the worktree checkout takes a while, so show its
                      // streamed progress instead of a static "starting…".
                      <div className="tile tile-spawning">
                        <div className="tile-head">
                          <span className="dot running" />
                          <span className="tile-title">
                            {spawnProgress ? spawnProgress.phase : "starting…"}
                          </span>
                          {spawnProgress?.percent != null && (
                            <span className="clone-progress-pct">{spawnProgress.percent}%</span>
                          )}
                        </div>
                        <div className="clone-progress-track">
                          <div
                            className={`clone-progress-bar${spawnProgress?.percent == null ? " indeterminate" : ""}`}
                            style={spawnProgress?.percent != null ? { width: `${spawnProgress.percent}%` } : undefined}
                          />
                        </div>
                        {spawnProgress?.detail && (
                          <div className="clone-progress-detail">{spawnProgress.detail}</div>
                        )}
                      </div>
                    )}
                  </div>
                )}
                {view === "focus" && <AgentFocus onSpawn={spawn} gitless={gitless} />}
              </div>
              {sourcePanelOpen && gitRoot && !gitless && (
                <>
                  <Resizer size={reviewPane.width} min={280} max={640} onChange={reviewPane.setWidth} side="right" />
                  <GitPanel
                    taskId={gitRoot}
                    layout="compact"
                    width={reviewPane.width}
                    selection={gitSel}
                    onSelect={(sel) => {
                      setGitSel(sel);
                      // The selection only renders in the Changes sub-view, so
                      // force it: with Pull Requests left active, this jump
                      // landed on the PR list and the clicked file never showed.
                      if (sel) { setSrcTab("changes"); setTab("source"); }
                    }}
                    allowComments={allowComments}
                    onRevealInFiles={revealInFilesTab}
                  />
                </>
              )}
            </div>
          )}
        </>
      )}

      {quickOpen && filesRoot && (
        <QuickOpen
          root={filesRoot}
          onOpen={(path) => {
            setTab("files");
            requestOpenFile({ rootKey: fileRootKey(filesRoot), path });
          }}
          onClose={() => setQuickOpen(false)}
        />
      )}

      {spawnDialogs}

      {approveRunId && approveRunId === focusedRunId && focused?.kind === "agent" && focused.worktree && (
        <MergeModal
          taskId={approveRunId}
          onClose={() => setApproveRun(null)}
          onReviewPr={(number) => {
            // Deep-link into Source Control → Pull Requests focused on this PR.
            setApproveRun(null);
            setTab("source");
            setSrcTab("prs");
            setReviewPr(number);
          }}
          // Archived or deleted: nothing is left to focus, so drop back to
          // this project's agent grid (the same place switching projects
          // lands) instead of an agents tab with no agent selected. The grid
          // points source control at your own checkout, and the panel comes up
          // with it: the merge that just landed is sitting there unpushed, and
          // that is the whole reminder to push it.
          onRemoved={() => {
            setApproveRun(null);
            setFocusedRun(null);
            setView("grid");
            setTab("agents");
            setSourcePanelOpen(true);
            refreshRuns();
          }}
        />
      )}
    </main>
  );
}
