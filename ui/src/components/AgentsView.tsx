import { useState } from "react";
import { Project } from "../api";
import { useRuns } from "../store/runs";
import AgentTile from "./AgentTile";
import AgentFocus from "./AgentFocus";
import NewTaskForm from "./NewTaskForm";
import SourceControl from "./SourceControl";
import GitReviewPanel from "./GitReviewPanel";

export default function AgentsView({ project }: { project: Project }) {
  const { runs, view, setView, focusedRunId } = useRuns();
  const [tab, setTab] = useState<"agents" | "source">("agents");
  const [newOpen, setNewOpen] = useState(false);
  const [review, setReview] = useState(false);

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
        {tab === "agents" && <button onClick={() => setNewOpen(true)}>+ New task</button>}
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
                {runs.length === 0 && <div className="board empty">No agents yet — start one with "+ New task".</div>}
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

      {newOpen && <div className="settings-overlay"><NewTaskForm project={project} onDone={() => setNewOpen(false)} /></div>}
    </main>
  );
}
