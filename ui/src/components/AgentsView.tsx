import { useEffect, useState } from "react";
import { Issue, Project, inspectRepo, RepoReadiness, FileRoot, agentInstalled, startIssueRun } from "../api";
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
  const [pendingSpawn, setPendingSpawn] = useState<{ agentId: string; readiness: RepoReadiness; repoPath: string; opts?: { base: string; mergeTarget: string }; issue?: Issue } | null>(null);
  const [missingAgent, setMissingAgent] = useState<string | null>(null);
  const [gitSel, setGitSel] = useState<GitSelection>(null);
  // Source Control has two sub-views: Changes (the git panel) and Pull Requests
  // (in-app review). `reviewPr` deep-links a specific PR from the Approve window.
  const [srcTab, setSrcTab] = useState<"changes" | "prs">("changes");
  const [reviewPr, setReviewPr] = useState<number | null>(null);
  useEffect(() => { setGitSel(null); }, [focusedRunId, project?.id]);
  const reviewPane = usePaneWidth("review", 360, 280, 640);

  // Which working tree source control operates on: the focused run's worktree
  // (terminals share the project checkout) or, with no run selected, the
  // project's main checkout via a "project:<id>" token. Null only at the
  // all-projects home screen.
  const gitRoot = focusedRunId ?? (project ? `project:${project.id}` : null);
  const allowComments = focused?.kind === "agent";

  async function spawn(agentId: string, opts?: { base: string; mergeTarget: string }, issue?: Issue) {
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
      if (r.state === "ready" && !r.dirty) {
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
  async function startIssue(issue: Issue, agentId: string, opts?: { base: string; mergeTarget: string }) {
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
          <button className={tab === "source" ? "on" : ""} onClick={() => setTab("source")}>⎇ Source Control</button>
          <button className={tab === "files" ? "on" : ""} onClick={() => setTab("files")}>▤ Files</button>
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
        {project && tab === "agents" && (
          <RightPanelToggle open={review} onToggle={() => setReview((r) => !r)} />
        )}
        {project && tab === "agents" && (
          <AgentAddMenu projectId={project.id} onSpawn={spawn} onTerminal={createTerminal} />
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
              <FilesView
                root={
                  focusedRunId
                    ? ({ kind: "run", id: focusedRunId } as FileRoot)
                    : ({ kind: "project", id: project.id } as FileRoot)
                }
                projectName={project.name}
              />
            </div>
          )}

          {tab === "agents" && (
            <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
              <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, overflow: "hidden" }}>
                {view === "grid" && (
                  <div className="grid">
                    {runs.length === 0 && !spawning && <div className="board empty">No agents yet — add one with "+ Agent".</div>}
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
              {review && gitRoot && (
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

      {pendingSpawn && (
        <RepoSetupDialog
          readiness={pendingSpawn.readiness}
          context="spawn"
          repoPath={pendingSpawn.repoPath}
          onResolved={async () => {
            const { agentId, opts, issue } = pendingSpawn;
            setPendingSpawn(null);
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

      {approveRunId && approveRunId === focusedRunId && focused?.kind === "agent" && (
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
