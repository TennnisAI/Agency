import { useState } from "react";
import { Project } from "../api";
import { useRuns } from "../store/runs";
import AgentTile from "./AgentTile";
import AgentFocus from "./AgentFocus";
import NewTaskForm from "./NewTaskForm";
import GitPanel from "./GitPanel";

export default function AgentsView({ project }: { project: Project }) {
  const { runs, view, setView, focusedRunId } = useRuns();
  const [tab, setTab] = useState<"agents" | "source">("agents");
  const [newOpen, setNewOpen] = useState(false);

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
        {tab === "agents" && <button onClick={() => setNewOpen(true)}>+ New task</button>}
      </div>

      {tab === "source" && <div className="source-wrap">{focusedRunId ? <GitPanel taskId={focusedRunId} /> : <div className="board empty">Open an agent to review its changes.</div>}</div>}

      {tab === "agents" && view === "grid" && (
        <div className="grid">
          {runs.length === 0 && <div className="board empty">No agents yet — start one with "+ New task".</div>}
          {runs.map((r) => <AgentTile key={r.id} run={r} />)}
        </div>
      )}
      {tab === "agents" && view === "focus" && <AgentFocus />}

      {newOpen && <div className="settings-overlay"><NewTaskForm project={project} onDone={() => setNewOpen(false)} /></div>}
    </main>
  );
}
