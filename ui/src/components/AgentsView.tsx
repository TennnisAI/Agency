import { useEffect, useRef, useState } from "react";
import { Issue, Project, inspectRepo, RepoReadiness, FileRoot, agentInstalled, startIssueRun } from "../api";
import type { SpawnOpts } from "../store/runs";
import { fileRootKey, requestOpenFile } from "../lib/openFile";
import { useRuns } from "../store/runs";
import AgentTile from "./AgentTile";
import AgentFocus from "./AgentFocus";
import MergeModal from "./MergeModal";
import GitPanel, { GitSelection } from "./git/GitPanel";
import PrReviewPanel from "./pr/PrReviewPanel";
import RepoSetupDialog from "./RepoSetupDialog";
import AgentAddMenu from "./AgentAddMenu";
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";
import FilesView from "./FilesView";
import DocsView from "./DocsView";
import HomeView from "./HomeView";
import IssuesView from "./IssuesView";
import SidebarToggle from "./SidebarToggle";
import RightPanelToggle from "./RightPanelToggle";
import InstallAgentDialog from "./InstallAgentDialog";
import QuickOpen from "./QuickOpen";

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
  const { runs, view, setView, focusedRunId, tab, setTab, approveRunId, setApproveRun, createAgent, createTerminal, spawning, spawnProgress, setFocusedRun, refreshRuns } = useRuns();
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;
  const [review, setReview] = useState(false);
  const [error, setError] = useState("");
  const [pendingSpawn, setPendingSpawn] = useState<{ agentId: string; readiness: RepoReadiness; repoPath: string; opts?: SpawnOpts; issue?: Issue } | null>(null);
  const [missingAgent, setMissingAgent] = useState<string | null>(null);
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
  const [projReadiness, setProjReadiness] = useState<RepoReadiness | null>(null);
  // Request token: a slow inspectRepo from a previous project (or an older
  // refresh) must not land over the current one's readiness.
  const readinessSeq = useRef(0);
  const refreshReadiness = () => {
    const seq = ++readinessSeq.current;
    if (!project) { setProjReadiness(null); return; }
    inspectRepo(project.repo_path).then((r) => {
      if (seq === readinessSeq.current) setProjReadiness(r);
    }).catch(() => {});
  };
  useEffect(() => {
    setProjReadiness(null);
    refreshReadiness();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.id]);
  const gitlessWorkspace = project?.kind === "workspace" && projReadiness?.state === "notARepo";
  // The workspace is a markdown vault: the Files tab would duplicate Docs
  // (with a manual-save editor, no less), so it's hidden there entirely.
  const isWorkspace = project?.kind === "workspace";

  // Per-project tab memory can restore "source" from before git was declined,
  // or "files" from before the workspace hid it.
  useEffect(() => {
    if (gitlessWorkspace && tab === "source") setTab("agents");
    if (isWorkspace && tab === "files") setTab("docs");
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
        if (!quickOpenGate.current.enabled) return;
        e.preventDefault();
        setQuickOpen((s) => !s);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => { setQuickOpen(false); }, [project?.id, tab]);

  async function spawn(agentId: string, opts?: SpawnOpts, issue?: Issue) {
    if (!project) return;
    setError("");
    try {
      // A preconfigured agent whose CLI is missing would spawn a session that
      // dies instantly; intercept and offer the install flow instead.
      const installed = await agentInstalled(agentId).catch(() => true);
      if (!installed) {
        setMissingAgent(agentId);
        return;
      }
      const r = await inspectRepo(project.repo_path);
      // A dirty checkout only blocks cutting a worktree (the new branch would
      // miss the uncommitted work). An agent that stays in the checkout is
      // being started *because* there is work in progress there.
      const dirtyBlocks = opts?.worktree !== false;
      if (r.state === "ready" && !(r.dirty && dirtyBlocks)) {
        if (issue) await startIssue(issue, agentId, opts);
        else await createAgent(agentId, opts);
      } else {
        setPendingSpawn({ agentId, readiness: r, repoPath: project.repo_path, opts, issue });
      }
    } catch (e) {
      setError(String(e));
    }
  }

  // Dispatch an issue to an agent, then jump to the run — the issue-flavored
  // tail of the same flow createAgent handles for promptless runs.
  async function startIssue(issue: Issue, agentId: string, opts?: SpawnOpts) {
    const run = await startIssueRun(issue.id, agentId, opts?.base, opts?.mergeTarget);
    await refreshRuns();
    setFocusedRun(run.id);
    setView("focus");
    setTab("agents");
  }

  return (
    <main className="agents">
      <div className="content-head">
        {!sidebarOpen && <SidebarToggle open={false} onToggle={onToggleSidebar} />}
        <div className="seg">
          <button className={tab === "agents" ? "on" : ""} onClick={() => setTab("agents")}>▦ Agents</button>
          <button className={tab === "issues" ? "on" : ""} onClick={() => setTab("issues")}>▧ Issues</button>
          <button className={tab === "docs" ? "on" : ""} onClick={() => setTab("docs")}>▥ Docs</button>
          {!gitlessWorkspace && (
            <button className={tab === "source" ? "on" : ""} onClick={() => setTab("source")}>⎇ Source Control</button>
          )}
          {!isWorkspace && (
            <button className={tab === "files" ? "on" : ""} onClick={() => setTab("files")}>▤ Files</button>
          )}
        </div>
        {project && tab === "agents" && (
          <div className="seg">
            <button className={view === "grid" ? "on" : ""} onClick={() => setView("grid")}>▦ Grid</button>
            <button className={view === "focus" ? "on" : ""} onClick={() => setView("focus")}>▭ Focus</button>
          </div>
        )}
        {project && tab === "source" && (
          <div className="seg">
            <button className={srcTab === "changes" ? "on" : ""} onClick={() => setSrcTab("changes")}>Changes</button>
            <button className={srcTab === "prs" ? "on" : ""} onClick={() => setSrcTab("prs")}>Pull Requests</button>
          </div>
        )}
        <div className="spacer" />
        {project && tab === "agents" && !gitlessWorkspace && (
          <RightPanelToggle open={review} onToggle={() => setReview((r) => !r)} />
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
              <FilesView root={filesRoot} projectId={project.id} projectName={project.name} />
            </div>
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

      {pendingSpawn && (
        <RepoSetupDialog
          readiness={pendingSpawn.readiness}
          context="spawn"
          repoPath={pendingSpawn.repoPath}
          onResolved={async () => {
            const { agentId, opts, issue } = pendingSpawn;
            setPendingSpawn(null);
            refreshReadiness();
            try {
              if (issue) await startIssue(issue, agentId, opts);
              else await createAgent(agentId, opts);
            } catch (e) {
              setError(String(e));
            }
          }}
          onCancel={() => setPendingSpawn(null)}
        />
      )}

      {missingAgent && project && (
        <InstallAgentDialog
          agent={missingAgent}
          projectId={project.id}
          onInstalling={async (run) => {
            setMissingAgent(null);
            await refreshRuns();
            setFocusedRun(run.id);
            setView("focus");
          }}
          onCancel={() => setMissingAgent(null)}
        />
      )}

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
          onArchived={() => {
            setApproveRun(null);
            setFocusedRun(null);
            refreshRuns();
          }}
        />
      )}
    </main>
  );
}
