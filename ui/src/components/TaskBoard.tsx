import { useState } from "react";
import { Project } from "../api";
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

  function closeTask() {
    setActivePrompt(null);
    setTaskId(null);
    setStatus("");
  }

  return (
    <main className="board">
      <header className="board-header">
        <h2>{project.name}</h2>
        <code>{project.repo_path}</code>
      </header>
      {activePrompt === null ? (
        <div className="new-task">
          <textarea
            placeholder="Task prompt (the shell profile ignores it for now; real agents will use it)"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
          />
          <button onClick={() => setActivePrompt(prompt)}>Start task</button>
        </div>
      ) : (
        <div className="task-running">
          <div className="task-status">status: {status || "starting…"}</div>
          <div className="task-split">
            <TerminalPane
              key={`${project.id}:${activePrompt}`}
              projectId={project.id}
              prompt={activePrompt}
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
