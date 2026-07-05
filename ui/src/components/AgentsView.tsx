import { useEffect, useState } from "react";
import { Project, inspectRepo, RepoReadiness, FileRoot, agentInstalled } from "../api";
import { useRuns } from "../store/runs";
import AgentTile from "./AgentTile";
import AgentFocus from "./AgentFocus";
import MergeModal from "./MergeModal";
import GitPanel, { GitSelection } from "./git/GitPanel";
import RepoSetupDialog from "./RepoSetupDialog";
import AgentAddMenu from "./AgentAddMenu";
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";
import FilesView from "./FilesView";
import HomeView from "./HomeView";
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
  const { runs, view, setView, focusedRunId, tab, setTab, approveRunId, setApproveRun, createAgent, createTerminal, setFocusedRun, refreshRuns } = useRuns();
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;
  const [review, setReview] = useState(false);
  const [error, setError] = useState("");
  const [pendingSpawn, setPendingSpawn] = useState<{ agentId: string; readiness: RepoReadiness; repoPath: string; opts?: { base: string; mergeTarget: string } } | null>(null);
  const [missingAgent, setMissingAgent] = useState<string | null>(null);
  const [gitSel, setGitSel] = useState<GitSelection>(null);
  useEffect(() => { setGitSel(null); }, [focusedRunId, project?.id]);
  const reviewPane = usePaneWidth("review", 360, 280, 640);

  // Which working tree source control operates on: the focused run's worktree
  // (terminals share the project checkout) or, with no run selected, the
  // project's main checkout via a "project:<id>" token. Null only at the
  // all-projects home screen.
  const gitRoot = focusedRunId ?? (project ? `project:${project.id}` : null);
  const allowComments = focused?.kind === "agent";

  async function spawn(agentId: string, opts?: { base: string; mergeTarget: string }) {
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
        await createAgent(agentId, opts);
      } else {
        setPendingSpawn({ agentId, readiness: r, repoPath: project.repo_path, opts });
      }
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <main className="agents">
      <div className="content-head">
        {!sidebarOpen && <SidebarToggle open={false} onToggle={onToggleSidebar} />}
        <div className="seg">
          <button className={tab === "agents" ? "on" : ""} onClick={() => setTab("agents")}>▦ Agents</button>
          <button className={tab === "source" ? "on" : ""} onClick={() => setTab("source")}>⎇ Source Control</button>
          <button className={tab === "files" ? "on" : ""} onClick={() => setTab("files")}>▤ Files</button>
        </div>
        {project && tab === "agents" && (
          <div className="seg">
            <button className={view === "grid" ? "on" : ""} onClick={() => setView("grid")}>▦ Grid</button>
            <button className={view === "focus" ? "on" : ""} onClick={() => setView("focus")}>▭ Focus</button>
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
        tab === "agents" ? (
          <HomeView onOpenRun={onOpenRun} onOpenProject={onOpenProject} />
        ) : (
          <div className="board empty">
            {tab === "source"
              ? "Select a project to browse its source control."
              : "Select a project to browse its files."}
          </div>
        )
      ) : (
        <>
          {tab === "source" && gitRoot && (
            <div className="source-wrap">
              <GitPanel taskId={gitRoot} layout="full" selection={gitSel} onSelect={setGitSel} allowComments={allowComments} />
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
                    {runs.length === 0 && <div className="board empty">No agents yet — add one with "+ Agent".</div>}
                    {runs.map((r) => <AgentTile key={r.id} run={r} />)}
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
            const { agentId, opts } = pendingSpawn;
            setPendingSpawn(null);
            try {
              await createAgent(agentId, opts);
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
