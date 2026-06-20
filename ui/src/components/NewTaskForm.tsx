import { useEffect, useState } from "react";
import { AgentProfile, Project, createRun, listProfiles } from "../api";
import { useRuns } from "../store/runs";

export default function NewTaskForm({ project, onDone }: { project: Project; onDone: () => void }) {
  const { refreshRuns } = useRuns();
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [agent, setAgent] = useState("claude");
  const [prompt, setPrompt] = useState("");
  const [base, setBase] = useState("HEAD");
  const [error, setError] = useState("");

  useEffect(() => {
    listProfiles().then((ps) => {
      setProfiles(ps);
      if (!ps.find((p) => p.name === "claude") && ps[0]) setAgent(ps[0].name);
    });
  }, []);

  async function start() {
    try {
      await createRun(project.id, prompt, agent, base);
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
      <label>Base branch</label>
      <input value={base} onChange={(e) => setBase(e.target.value)} />
      <div className="nt-preview">
        <div>worktree: <code>{project.repo_path.split("/").filter(Boolean).pop()}/.agency/worktrees/&lt;new&gt;</code></div>
        <div>branch: <code>agent/&lt;new&gt;</code></div>
      </div>
      <div className="row-actions">
        <button onClick={start}>Start agent</button>
        <button className="ghost" onClick={onDone}>Cancel</button>
      </div>
    </div>
  );
}
