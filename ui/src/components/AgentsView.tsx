import { useEffect, useState } from "react";
import { Project, inspectRepo, RepoReadiness, FileRoot } from "../api";
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

export default function AgentsView({ project }: { project: Project }) {
  const { runs, view, setView, focusedRunId, tab, setTab, approveRunId, setApproveRun, createAgent } = useRuns();
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;
  const [review, setReview] = useState(false);
  const [error, setError] = useState("");
  const [pendingSpawn, setPendingSpawn] = useState<{ agentId: string; readiness: RepoReadiness; repoPath: string; opts?: { base: string; mergeTarget: string } } | null>(null);
  const [gitSel, setGitSel] = useState<GitSelection>(null);
  useEffect(() => { setGitSel(null); }, [focusedRunId]);
  const reviewPane = usePaneWidth("review", 360, 280, 640);

  async function spawn(agentId: string, opts?: { base: string; mergeTarget: string }) {
    setError("");
    try {
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
        <div className="seg">
          <button className={tab === "agents" ? "on" : ""} onClick={() => setTab("agents")}>▦ Agents</button>
          <button className={tab === "source" ? "on" : ""} onClick={() => setTab("source")}>⎇ Source Control</button>
          <button className={tab === "files" ? "on" : ""} onClick={() => setTab("files")}>▤ Files</button>
        </div>
        {tab === "agents" && (
          <div className="seg">
            <button className={view === "grid" ? "on" : ""} onClick={() => setView("grid")}>▦ Grid</button>
            <button className={view === "focus" ? "on" : ""} onClick={() => setView("focus")}>▭ Focus</button>
          </div>
        )}
        <div className="spacer" />
        {tab === "agents" && focused?.kind === "agent" && (
          <button className={review ? "on" : ""} onClick={() => setReview((r) => !r)}>Review</button>
        )}
        {tab === "agents" && (
          <AgentAddMenu projectId={project.id} onSpawn={spawn} />
        )}
      </div>

      {error && <div className="git-error">{error}</div>}

      {tab === "source" && (
        <div className="source-wrap">
          {focusedRunId && focused?.kind === "agent"
            ? <GitPanel taskId={focusedRunId} layout="full" selection={gitSel} onSelect={setGitSel} />
            : <div className="board empty">{focused?.kind === "terminal" ? "Terminals have no source control." : "Open an agent to review its changes."}</div>}
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
            {view === "focus" && <AgentFocus />}
          </div>
          {review && focusedRunId && focused?.kind === "agent" && (
            <>
              <Resizer size={reviewPane.width} min={280} max={640} onChange={reviewPane.setWidth} side="right" />
              <GitPanel
                taskId={focusedRunId}
                layout="compact"
                width={reviewPane.width}
                selection={gitSel}
                onSelect={(sel) => { setGitSel(sel); if (sel) setTab("source"); }}
              />
            </>
          )}
        </div>
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

      {approveRunId && approveRunId === focusedRunId && (
        <MergeModal taskId={approveRunId} onClose={() => setApproveRun(null)} />
      )}
    </main>
  );
}
