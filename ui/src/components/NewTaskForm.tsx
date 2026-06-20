import { useEffect, useState } from "react";
import { AgentProfile, Project, createRun, listProfiles } from "../api";
import { useRuns } from "../store/runs";

export default function NewTaskForm({ project, onDone }: { project: Project; onDone: () => void }) {
  const { refreshRuns } = useRuns();
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [agent, setAgent] = useState("claude");
  const [prompt, setPrompt] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    listProfiles().then((ps) => {
      setProfiles(ps);
      if (!ps.find((p) => p.name === "claude") && ps[0]) setAgent(ps[0].name);
    });
  }, []);

  async function start() {
    try {
      await createRun(project.id, prompt, agent, "HEAD");
      await refreshRuns();
      onDone();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="new-task card">
      <h3>New agent</h3>
      {error && <div className="git-error">{error}</div>}
      <label>Agent type</label>
      <select value={agent} onChange={(e) => setAgent(e.target.value)}>
        {profiles.map((p) => <option key={p.name} value={p.name}>{p.name}</option>)}
      </select>
      <label>Prompt</label>
      <textarea value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder="What should this agent do?" />
      <div className="row-actions">
        <button onClick={start}>Start agent</button>
        <button className="ghost" onClick={onDone}>Cancel</button>
      </div>
    </div>
  );
}
