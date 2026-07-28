import { useEffect, useMemo, useState } from "react";
import { Project, listProjects } from "../api";
import { useRuns } from "../store/runs";
import { PaletteEntry, filterEntries } from "../lib/paletteFilter";
import { PENDING_QUICKADD_KEY } from "../lib/issues";
import { workspaceHidden } from "../lib/workspacePref";

export default function CommandPalette({ onClose }: { onClose: () => void }) {
  const { runs, selectedProjectId, setSelectedProject, setFocusedRun, setView, setTab } = useRuns();
  const [projects, setProjects] = useState<Project[]>([]);
  const [query, setQuery] = useState("");
  const [hi, setHi] = useState(0);

  useEffect(() => {
    listProjects().then(setProjects).catch(() => setProjects([]));
  }, []);

  const entries: PaletteEntry[] = useMemo(() => {
    // "Today's Note" is global (the workspace is app-level, created on first
    // use — and absent entirely when the user hides the workspace); the rest
    // of the actions need a project to act on.
    const as: PaletteEntry[] = [
      ...(workspaceHidden()
        ? []
        : [{
            kind: "action" as const,
            id: "daily-note",
            projectId: selectedProjectId ?? "",
            label: "Today's Note",
            sublabel: "Open today's journal entry in the workspace",
          }]),
      ...(selectedProjectId
        ? [{
            kind: "action" as const,
            id: "new-issue",
            projectId: selectedProjectId,
            label: "New Issue",
            sublabel: "Capture an issue in this project",
          }]
        : []),
    ];
    const ps: PaletteEntry[] = projects.map((p) => ({
      kind: "project",
      id: p.id,
      projectId: p.id,
      label: p.name,
      sublabel: p.repo_path,
    }));
    const rs: PaletteEntry[] = runs.map((r) => ({
      kind: "run",
      id: r.id,
      projectId: r.projectId,
      label: `${r.agent}: ${r.prompt || r.branch}`,
      sublabel: r.branch,
    }));
    return [...as, ...ps, ...rs];
  }, [projects, runs, selectedProjectId]);

  const filtered = useMemo(() => filterEntries(query, entries), [query, entries]);

  useEffect(() => {
    setHi(0);
  }, [query]);

  function activate(e: PaletteEntry) {
    if (e.kind === "action") {
      if (e.id === "daily-note") {
        // Routed through Shell (it owns the workspace/daily-note flow).
        window.dispatchEvent(new CustomEvent("agency:daily-note"));
      } else {
        // "New Issue": jump to the board with quick-add focused (the flag is
        // consumed by IssuesView on tab activation).
        sessionStorage.setItem(PENDING_QUICKADD_KEY, "1");
        setTab("issues");
      }
    } else if (e.kind === "project") {
      setSelectedProject(e.id);
    } else {
      setSelectedProject(e.projectId);
      setFocusedRun(e.id);
      setView("focus");
    }
    onClose();
  }

  function onKey(ev: React.KeyboardEvent) {
    if (ev.key === "Escape") {
      onClose();
    } else if (ev.key === "ArrowDown") {
      ev.preventDefault();
      setHi((h) => (filtered.length === 0 ? 0 : Math.min(h + 1, filtered.length - 1)));
    } else if (ev.key === "ArrowUp") {
      ev.preventDefault();
      setHi((h) => Math.max(h - 1, 0));
    } else if (ev.key === "Enter") {
      ev.preventDefault();
      if (filtered[hi]) activate(filtered[hi]);
    }
  }

  return (
    <div className="palette-overlay" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <input
          className="palette-input"
          autoFocus
          placeholder="Search projects and agents…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKey}
        />
        <ul className="palette-list">
          {filtered.length === 0 && <li className="palette-empty">No matches</li>}
          {filtered.map((e, i) => (
            <li
              key={`${e.kind}:${e.id}`}
              className={`palette-row ${i === hi ? "on" : ""}`}
              onMouseEnter={() => setHi(i)}
              onClick={() => activate(e)}
            >
              <span className={`palette-kind ${e.kind}`}>{e.kind === "project" ? "▢" : e.kind === "action" ? "＋" : "▸"}</span>
              <span className="palette-label">{e.label}</span>
              <span className="palette-sub">{e.sublabel}</span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
