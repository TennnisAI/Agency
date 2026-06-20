import { useState } from "react";
import { Project } from "../api";
import { AGENT_TYPES } from "../agents";
import { useRuns } from "../store/runs";
import AgentTile from "./AgentTile";
import AgentFocus from "./AgentFocus";
import MergeModal from "./MergeModal";
import SourceControl from "./SourceControl";
import GitReviewPanel from "./GitReviewPanel";

export default function AgentsView({ project: _project }: { project: Project }) {
  const { runs, view, setView, focusedRunId, tab, setTab, approveRunId, setApproveRun, createAgent } = useRuns();
  const [review, setReview] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);

  async function spawn(agentId: string) {
    setMenuOpen(false);
    await createAgent(agentId);
  }

  return (
    <main className="agents">
      <div className="content-head">
        <div className="seg">
          <button className={tab === "agents" ? "on" : ""} onClick={() => setTab("agents")}>▦ Agents</button>
          <button className={tab === "source" ? "on" : ""} onClick={() => setTab("source")}>⎇ Source Control</button>
        </div>
        {tab === "agents" && (
          <div className="seg">
            <button className={view === "grid" ? "on" : ""} onClick={() => setView("grid")}>▦ Grid</button>
            <button className={view === "focus" ? "on" : ""} onClick={() => setView("focus")}>▭ Focus</button>
          </div>
        )}
        <div className="spacer" />
        {tab === "agents" && (
          <button className={review ? "on" : ""} onClick={() => setReview((r) => !r)}>Review</button>
        )}
        {tab === "agents" && (
          <div className="agent-add">
            <button className="btn-primary" onClick={() => setMenuOpen((o) => !o)}>+ Agent ▾</button>
            {menuOpen && (
              <div className="agent-menu" onMouseLeave={() => setMenuOpen(false)}>
                {AGENT_TYPES.map((a) => (
                  <button key={a.id} onClick={() => spawn(a.id)}>{a.label}</button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {tab === "source" && (
        <div className="source-wrap">
          {focusedRunId ? <SourceControl taskId={focusedRunId} /> : <div className="board empty">Open an agent to review its changes.</div>}
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
          {review && focusedRunId && (
            <GitReviewPanel taskId={focusedRunId} onOpenSource={() => setTab("source")} />
          )}
        </div>
      )}

      {approveRunId && approveRunId === focusedRunId && (
        <MergeModal taskId={approveRunId} onClose={() => setApproveRun(null)} />
      )}
    </main>
  );
}
