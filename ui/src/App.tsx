import { useState } from "react";
import ProjectSidebar from "./components/ProjectSidebar";
import TaskBoard from "./components/TaskBoard";
import Settings from "./components/Settings";
import { Project } from "./api";

export default function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  return (
    <div className="shell">
      {/* Title bar */}
      <header className="titlebar">
        <button className="titlebar-icon" aria-label="Toggle panel">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <rect x="2" y="3" width="12" height="1.5" rx="0.75" fill="currentColor" />
            <rect x="2" y="7.25" width="12" height="1.5" rx="0.75" fill="currentColor" />
            <rect x="2" y="11.5" width="12" height="1.5" rx="0.75" fill="currentColor" />
          </svg>
        </button>
        <span className="titlebar-mark">Agency</span>
        <span className="titlebar-spacer" />
        <button className="titlebar-icon" aria-label="Settings" onClick={() => setShowSettings(true)}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <path
              d="M8 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4Z"
              stroke="currentColor"
              strokeWidth="1.4"
              fill="none"
            />
            <path
              d="M13.3 6.6l-.7-.4a5.2 5.2 0 0 0 0-1.6l.7-.4a.7.7 0 0 0 .3-.9l-.8-1.4a.7.7 0 0 0-.9-.3l-.7.4A5.3 5.3 0 0 0 9.8 1.6V.8A.7.7 0 0 0 9.1.1H6.9a.7.7 0 0 0-.7.7v.8a5.3 5.3 0 0 0-1.4.8l-.7-.4a.7.7 0 0 0-.9.3L2.4 3.7a.7.7 0 0 0 .3.9l.7.4a5.2 5.2 0 0 0 0 1.6l-.7.4a.7.7 0 0 0-.3.9l.8 1.4a.7.7 0 0 0 .9.3l.7-.4c.4.3.9.6 1.4.8v.8c0 .4.3.7.7.7h2.2c.4 0 .7-.3.7-.7v-.8a5.3 5.3 0 0 0 1.4-.8l.7.4c.3.2.7 0 .9-.3l.8-1.4a.7.7 0 0 0-.3-.9Z"
              stroke="currentColor"
              strokeWidth="1.4"
              fill="none"
            />
          </svg>
        </button>
      </header>

      {/* Content region */}
      <div className="content">
        <div className="sidebar-wrap">
          <ProjectSidebar selectedId={project?.id ?? null} onSelect={setProject} />
        </div>
        {project ? (
          <TaskBoard project={project} />
        ) : (
          <main className="board empty">Select or add a project to begin.</main>
        )}
      </div>

      {/* Status bar */}
      <footer className="statusbar">
        <span className="statusbar-project">{project ? project.name : "no project"}</span>
        <span className="statusbar-hints">⌘N new · ⌘G source · ⌘↵ approve</span>
      </footer>

      {showSettings && <Settings onClose={() => setShowSettings(false)} />}
    </div>
  );
}
