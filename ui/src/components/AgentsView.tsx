import { useEffect, useRef, useState } from "react";
import { Project, FileRoot, projectTarget } from "../api";
import { fileRootKey, requestOpenFile } from "../lib/openFile";
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
import { useRepoReadiness, isGitlessWorkspace } from "../hooks/useRepoReadiness";
import { useSpawnAgent } from "../hooks/useSpawnAgent";
import FilesView from "./FilesView";
import DocsView from "./DocsView";
import HomeView from "./HomeView";
import IssuesView from "./IssuesView";
import RunPanel from "./RunPanel";
import SidebarToggle from "./SidebarToggle";
import RightPanelToggle from "./RightPanelToggle";
import QuickOpen from "./QuickOpen";
import { useElementWidth } from "../hooks/useElementWidth";

// Main content area. With a project selected this is that project's agents /
// source control / files; with none it hosts the all-projects overview under
// the same, always-visible header bar.
export default function AgentsView({
  project,
  sidebarOpen,
  onToggleSidebar,
  onOpenRun,
  onOpenProject,
}: {
  project: Project | null;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onOpenRun: (project: Project, runId: string) => void;
  onOpenProject: (project: Project) => void;
}) {
  const { runs, view, setView, focusedRunId, tab, setTab, approveRunId, setApproveRun, createTerminal, spawning, spawnProgress, setFocusedRun, refreshRuns } = useRuns();
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;
  const [review, setReview] = useState(false);
  // Agents alongside the file tree, the Files-tab twin of the Docs side panel.
  const [filesAgents, setFilesAgents] = useState(false);
  const [gitSel, setGitSel] = useState<GitSelection>(null);
  // Source Control has two sub-views: Changes (the git panel) and Pull Requests
  // (in-app review). `reviewPr` deep-links a specific PR from the Approve window.
  const [srcTab, setSrcTab] = useState<"changes" | "prs">("changes");
  const [reviewPr, setReviewPr] = useState<number | null>(null);
  useEffect(() => { setGitSel(null); }, [focusedRunId, project?.id]);
  const reviewPane = usePaneWidth("review", 360, 280, 640);

  // The workspace can decline git; everything git-shaped (Source Control, the
  // review panel, agent spawn — agents need worktrees) hides for it then.
  // Terminals stay: they run in the checkout, no branch required.
  const { readiness: projReadiness, refresh: refreshReadiness } = useRepoReadiness(project);
  const gitlessWorkspace = isGitlessWorkspace(project, projReadiness);
  // The spawn pre-flight (missing CLI, unready repo) and its dialogs, shared
  // with the agents side panel in the Docs / Files tabs.
  const { spawn, error, dialogs: spawnDialogs } = useSpawnAgent(project, refreshReadiness);
  // The workspace is a markdown vault: the Files tab would duplicate Docs
  // (with a manual-save editor, no less), so it's hidden there entirely.
  const isWorkspace = project?.kind === "workspace";

  // Per-project tab memory can restore "source" from before git was declined,
  // or "files" from before the workspace hid it.
  useEffect(() => {
    if (gitlessWorkspace && tab === "source") setTab("agents");
    if (isWorkspace && (tab === "files" || tab === "run")) setTab("docs");
  }, [gitlessWorkspace, isWorkspace, tab, setTab]);

  // Which working tree source control operates on: the focused run's worktree
  // (terminals share the project checkout) or, with no run selected, the
  // project's main checkout via a "project:<id>" token. Null only at the
  // all-projects home screen.
  const gitRoot = focusedRunId ?? (project ? `project:${project.id}` : null);
  const allowComments = focused?.kind === "agent";

  // The root the Files tab shows (and quick-open must list): the focused run's
  // worktree, else the project's main checkout.
  const filesRoot: FileRoot | null = project
    ? focusedRunId
      ? { kind: "run", id: focusedRunId }
      : { kind: "project", id: project.id }
    : null;

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
          {!gitlessWorkspace && (
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
              title="Run the project's scripts in your own checkout"
              onClick={() => setTab("run")}
            >
              <span className="seg-ico" aria-hidden>▷</span><span className="seg-label">Run</span>
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
        <div className="spacer" />
        {project && tab === "agents" && !gitlessWorkspace && (
          <RightPanelToggle open={review} onToggle={() => setReview((r) => !r)} />
        )}
        {project && tab === "files" && (
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
            terminalOnly={gitlessWorkspace}
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
            />
          )}

          {tab === "docs" && (
            <div className="source-wrap">
              <DocsView project={project} />
            </div>
          )}

          {tab === "source" && (
            <div className="source-wrap">
              {srcTab === "changes" && gitRoot && (
                <GitPanel taskId={gitRoot} layout="full" selection={gitSel} onSelect={setGitSel} allowComments={allowComments} />
              )}
              {srcTab === "prs" && (
                <PrReviewPanel projectId={project.id} initialPr={reviewPr} onConsumeInitial={() => setReviewPr(null)} />
              )}
            </div>
          )}

          {tab === "files" && (
            <div className="source-wrap">
              <FilesView root={filesRoot} project={project} agentsOpen={filesAgents} />
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
                        {gitlessWorkspace
                          ? "Agents need git to work in isolated branches. Initialize a repository in the workspace (Settings ▸ Workspace) to dispatch them here."
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
                {view === "focus" && <AgentFocus onSpawn={spawn} />}
              </div>
              {review && gitRoot && !gitlessWorkspace && (
                <>
                  <Resizer size={reviewPane.width} min={280} max={640} onChange={reviewPane.setWidth} side="right" />
                  <GitPanel
                    taskId={gitRoot}
                    layout="compact"
                    width={reviewPane.width}
                    selection={gitSel}
                    onSelect={(sel) => { setGitSel(sel); if (sel) setTab("source"); }}
                    allowComments={allowComments}
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
          // lands) instead of an agents tab with no agent selected.
          onRemoved={() => {
            setApproveRun(null);
            setFocusedRun(null);
            setView("grid");
            setTab("agents");
            refreshRuns();
          }}
        />
      )}
    </main>
  );
}
