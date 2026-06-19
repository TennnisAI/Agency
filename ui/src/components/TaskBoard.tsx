import { useEffect, useState } from "react";
import { AgentProfile, listProfiles, Project } from "../api";
import TerminalPane from "./TerminalPane";
import GitPanel from "./GitPanel";

interface Props {
  project: Project;
}

export default function TaskBoard({ project }: Props) {
  const [prompt, setPrompt] = useState("");
  const [activePrompt, setActivePrompt] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [taskId, setTaskId] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [profile, setProfile] = useState("shell");
  const [activeProfile, setActiveProfile] = useState("shell");

  useEffect(() => {
    listProfiles().then((ps) => {
      setProfiles(ps);
      if (!ps.find((p) => p.name === "shell") && ps[0]) setProfile(ps[0].name);
    });
  }, []);

  function closeTask() {
    setActivePrompt(null);
    setTaskId(null);
    setStatus("");
  }

  function startTask() {
    setActiveProfile(profile);
    setActivePrompt(prompt);
  }

  return (
    <main className="board">
      <header className="board-header">
        <h2>{project.name}</h2>
        <code>{project.repo_path}</code>
      </header>
      {activePrompt === null ? (
        <div className="new-task">
          <select value={profile} onChange={(e) => setProfile(e.target.value)}>
            {profiles.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
              </option>
            ))}
          </select>
          <textarea
            placeholder="Task prompt"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
          />
          <button onClick={startTask}>Start task</button>
        </div>
      ) : (
        <div className="task-running">
          <div className="task-status">
            status: {status || "starting…"} · agent: {activeProfile}
          </div>
          <div className="task-split">
            <TerminalPane
              key={`${project.id}:${activeProfile}:${activePrompt}`}
              projectId={project.id}
              prompt={activePrompt}
              profile={activeProfile}
              onStatus={setStatus}
              onStarted={setTaskId}
            />
            {taskId && <GitPanel taskId={taskId} />}
          </div>
          <button onClick={closeTask}>Close terminal</button>
        </div>
      )}
    </main>
  );
}
