import { useEffect, useState } from "react";
import { Project, RunInfo, listProjects, listRuns, runPreview } from "../api";
import { projectColor, runName } from "../agents";

const FOLD_KEY = "home:folded";

function loadFolded(): Set<string> {
  try {
    const raw = localStorage.getItem(FOLD_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    return new Set();
  }
}

function saveFolded(folded: Set<string>) {
  try {
    localStorage.setItem(FOLD_KEY, JSON.stringify([...folded]));
  } catch {
    /* quota/security errors: fold state just won't persist */
  }
}

// Landing view when no project is selected: a live overview of every agent
// across every project. Groups are ordered most-active first; clicking a tile
// jumps straight into that agent's focus mode, clicking a project header
// selects the project.
export default function HomeView({
  onOpenRun,
  onOpenProject,
}: {
  onOpenRun: (project: Project, runId: string) => void;
  onOpenProject: (project: Project) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [runsBy, setRunsBy] = useState<Record<string, RunInfo[]>>({});
  const [loaded, setLoaded] = useState(false);
  // Folded project ids persist across sessions so a big workspace stays tidy.
  const [folded, setFolded] = useState<Set<string>>(loadFolded);

  function toggleFold(id: string) {
    setFolded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      saveFolded(next);
      return next;
    });
  }

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const ps = await listProjects();
        const lists = await Promise.all(ps.map((p) => listRuns(p.id).catch(() => [] as RunInfo[])));
        if (!alive) return;
        setProjects(ps);
        setRunsBy(Object.fromEntries(ps.map((p, i) => [p.id, lists[i]])));
        setLoaded(true);
      } catch {
        /* transient backend errors: keep the last snapshot */
      }
    };
    tick();
    const t = window.setInterval(tick, 2500);
    return () => { alive = false; window.clearInterval(t); };
  }, []);

  const all = projects.flatMap((p) => runsBy[p.id] ?? []);
  const running = all.filter((r) => r.status.state === "running").length;

  if (loaded && projects.length === 0) {
    return (
      <div className="home home-blank">
        <div className="home-hero">
          <div className="home-hero-mark">▦</div>
          <h1>Welcome to Agency</h1>
          <p>Add a project with the <strong>+</strong> button in the Projects pane, then dispatch agents to work on it in parallel.</p>
        </div>
      </div>
    );
  }

  // Projects with running agents surface first; ties break alphabetically.
  const ordered = [...projects].sort((a, b) => {
    const ra = (runsBy[a.id] ?? []).filter((r) => r.status.state === "running").length;
    const rb = (runsBy[b.id] ?? []).filter((r) => r.status.state === "running").length;
    return rb - ra || a.name.localeCompare(b.name);
  });

  return (
    <div className="home">
      <div className="home-head">
        <div>
          <div className="eyebrow">ALL PROJECTS</div>
          <h1 className="home-title">Overview</h1>
        </div>
        <div className="home-stats">
          <Stat value={projects.length} label={projects.length === 1 ? "project" : "projects"} />
          <Stat value={all.length} label={all.length === 1 ? "agent" : "agents"} />
          <Stat value={running} label="running" accent={running > 0} />
        </div>
      </div>

      {ordered.map((p) => {
        const runs = [...(runsBy[p.id] ?? [])].sort(
          (a, b) => Number(b.status.state === "running") - Number(a.status.state === "running"),
        );
        const live = runs.filter((r) => r.status.state === "running").length;
        const isFolded = folded.has(p.id);
        return (
          <section key={p.id} className="home-group">
            <div className="home-group-head">
              <button
                className="home-fold"
                title={isFolded ? "Expand project" : "Collapse project"}
                onClick={() => toggleFold(p.id)}
              >
                {isFolded ? "▸" : "▾"}
              </button>
              <button className="home-group-title" onClick={() => onOpenProject(p)} title={`Open ${p.name}`}>
                <span className="proj-icon" aria-hidden style={{ background: projectColor(p.id) }}>
                  {p.name.slice(0, 1).toUpperCase()}
                </span>
                <span className="home-group-name">{p.name}</span>
                <span className="home-group-meta">
                  {runs.length === 0 ? "no agents" : `${runs.length} agent${runs.length === 1 ? "" : "s"}`}
                  {live > 0 && <span className="home-live"> · {live} running</span>}
                </span>
                <span className="home-group-open">open →</span>
              </button>
            </div>
            {!isFolded && runs.length > 0 && (
              <div className="grid home-grid">
                {runs.map((r) => (
                  <HomeTile key={r.id} run={r} onOpen={() => onOpenRun(p, r.id)} />
                ))}
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}

function Stat({ value, label, accent }: { value: number; label: string; accent?: boolean }) {
  return (
    <span className={`home-stat ${accent ? "accent" : ""}`}>
      <span className="home-stat-n">{value}</span> {label}
    </span>
  );
}

function statusLabel(s: RunInfo["status"]): { cls: string; text: string } {
  if (s.state === "running") return { cls: "running", text: "running" };
  if (s.state === "exited") return { cls: "exited", text: `exited (${s.code})` };
  return { cls: "exited", text: "gone" };
}

function badgeClass(agent: string): string {
  return ["claude", "pi", "hermes"].includes(agent) ? `badge ${agent}` : "badge";
}

// Same visual language as the per-project AgentTile, but read-only: no discard
// button (the overview is for surveying and jumping in, not managing).
function HomeTile({ run, onOpen }: { run: RunInfo; onOpen: () => void }) {
  const [preview, setPreview] = useState("");

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const p = await runPreview(run.id, 10);
        if (alive) setPreview(p);
      } catch {
        /* session may be gone; keep the tile without a preview */
      }
    };
    tick();
    const t = window.setInterval(tick, 2500);
    return () => { alive = false; window.clearInterval(t); };
  }, [run.id]);

  const st = statusLabel(run.status);
  const isTerminal = run.kind === "terminal";
  return (
    <div className="tile" onClick={onOpen} title="Open in focus mode">
      <div className="tile-head">
        <span className={`dot ${st.cls}`} />
        <span className="tile-title">{runName(run)}</span>
        <span className={isTerminal ? "badge" : badgeClass(run.agent)}>{isTerminal ? "terminal" : run.agent}</span>
      </div>
      {!isTerminal && (
        <div className="tile-meta">
          <code>{run.branch}</code>
          <span className="diffstat"><span className="add">+{run.added}</span> <span className="del">−{run.deleted}</span> · {run.files}f</span>
        </div>
      )}
      {preview && <pre className="tile-preview">{preview}</pre>}
      <div className="tile-foot">{st.text}</div>
    </div>
  );
}
