import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Issue, Project, RunInfo, RepoReadiness, addProject, inspectRepo, listIssues, listProjects, listRuns, runPreview } from "../api";
import { projectAccent, runName } from "../agents";
import { inGitlessFolder, isWorking, needsAttention, pinnedFirst, runStatus } from "../lib/runstate";
import RepoSetupDialog from "./RepoSetupDialog";
import CloneDialog from "./CloneDialog";
import HomeIssues from "./HomeIssues";
import AttentionMarker from "./AttentionMarker";

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
// (or, in issues mode, every open issue) across every project. Groups are
// ordered most-active first; clicking a tile jumps straight into that agent's
// focus mode, clicking an issue opens its project's board, clicking a project
// header selects the project.
export default function HomeView({
  onOpenRun,
  onOpenProject,
  mode = "agents",
}: {
  onOpenRun: (project: Project, runId: string) => void;
  onOpenProject: (project: Project) => void;
  mode?: "agents" | "issues";
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [runsBy, setRunsBy] = useState<Record<string, RunInfo[]>>({});
  const [issuesBy, setIssuesBy] = useState<Record<string, Issue[]>>({});
  const [loaded, setLoaded] = useState(false);
  // Folded project ids persist across sessions so a big workspace stays tidy.
  const [folded, setFolded] = useState<Set<string>>(loadFolded);
  const [addError, setAddError] = useState("");
  const [setup, setSetup] = useState<{ path: string; name: string; readiness: RepoReadiness } | null>(null);
  const [cloning, setCloning] = useState(false);

  // Same add-project flow as the Projects pane "+" control: pick a directory,
  // add it straight away if the repo is ready, otherwise route through the
  // setup dialog. A freshly added project is auto-opened.
  async function handleAdd() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    const name = sel.split("/").filter(Boolean).pop() ?? sel;
    setAddError("");
    try {
      const r = await inspectRepo(sel);
      if (r.state === "ready" && !r.dirty) {
        onOpenProject(await addProject(name, sel));
      } else {
        setSetup({ path: sel, name, readiness: r });
      }
    } catch (e) {
      setAddError(String(e));
    }
  }

  async function finishSetup() {
    if (!setup) return;
    try {
      onOpenProject(await addProject(setup.name, setup.path));
    } catch (e) {
      setAddError(String(e));
    }
    setSetup(null);
  }

  // A cloned repo is ready immediately (has commits, clean tree) — add and open it.
  async function handleCloned(path: string) {
    setCloning(false);
    const name = path.split("/").filter(Boolean).pop() ?? path;
    setAddError("");
    try {
      onOpenProject(await addProject(name, path));
    } catch (e) {
      setAddError(String(e));
    }
  }

  function toggleFold(id: string) {
    setFolded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      saveFolded(next);
      return next;
    });
  }

  // Hoisted so the issues board can force a refresh right after a mutation
  // instead of waiting out the poll interval. The token sequences overlapping
  // ticks (a mutation refresh can interleave with the interval): only the
  // newest snapshot may land, and unmount invalidates whatever is in flight.
  const tickSeq = useRef(0);
  const tick = useCallback(async () => {
    const seq = ++tickSeq.current;
    try {
      const ps = await listProjects();
      const lists = await Promise.all(ps.map((p) => listRuns(p.id).catch(() => [] as RunInfo[])));
      // Issue lists only matter (and only cost IPC) in issues mode.
      const issueLists = mode === "issues"
        ? await Promise.all(ps.map((p) => listIssues(p.id).catch(() => [] as Issue[])))
        : null;
      if (seq !== tickSeq.current) return;
      setProjects(ps);
      setRunsBy(Object.fromEntries(ps.map((p, i) => [p.id, lists[i]])));
      if (issueLists) setIssuesBy(Object.fromEntries(ps.map((p, i) => [p.id, issueLists[i]])));
      setLoaded(true);
    } catch {
      /* transient backend errors: keep the last snapshot */
    }
  }, [mode]);

  useEffect(() => {
    tick();
    const t = window.setInterval(tick, 2500);
    return () => {
      tickSeq.current++; // drop any in-flight snapshot
      window.clearInterval(t);
    };
  }, [tick]);

  const all = projects.flatMap((p) => runsBy[p.id] ?? []);
  const working = all.filter(isWorking).length;
  // The header counts what is actually asking for the user, so settling or
  // snoozing a run takes it out of the number as well as off the tile. A count
  // that keeps a settled run in it is the same climbing badge by another name.
  const waitingCount = all.filter(needsAttention).length;

  // Hold the header (and its 0/0/0 stats) until the first poll returns, so an
  // empty overview doesn't flash before real counts or the welcome hero.
  if (!loaded) return null;

  if (projects.length === 0) {
    return (
      <div className="home home-blank">
        <div className="home-hero">
          <div className="home-hero-mark">▦</div>
          <h1>Welcome to Agency</h1>
          <p>Add a project from a local folder, or clone an existing repository, then dispatch agents to work on it in parallel.</p>
          <div className="home-hero-actions">
            <button className="btn-primary" onClick={handleAdd}>Add project</button>
            <button className="btn-secondary" onClick={() => setCloning(true)}>Clone a repository</button>
          </div>
          {addError && <div className="git-error">{addError}</div>}
        </div>
        {setup && (
          <RepoSetupDialog
            readiness={setup.readiness}
            context="add"
            repoPath={setup.path}
            onResolved={finishSetup}
            onCancel={() => setSetup(null)}
          />
        )}
        {cloning && (
          <CloneDialog onCloned={handleCloned} onCancel={() => setCloning(false)} />
        )}
      </div>
    );
  }

  if (mode === "issues") {
    return (
      <HomeIssues
        projects={projects}
        issuesBy={issuesBy}
        runsBy={runsBy}
        folded={folded}
        onToggleFold={toggleFold}
        onOpenProject={onOpenProject}
        onOpenRun={onOpenRun}
        refresh={tick}
      />
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
          <Stat value={working} label="working" accent={working > 0} />
          <Stat value={waitingCount} label="waiting" warn={waitingCount > 0} />
        </div>
      </div>

      {ordered.map((p) => {
        // Pinned runs first, in the order they were pinned; the rest running
        // first. A pin holds its place whatever the run is doing, so it is the
        // outer sort and lifecycle only orders what is left.
        const runs = pinnedFirst(
          [...(runsBy[p.id] ?? [])].sort(
            (a, b) => Number(b.status.state === "running") - Number(a.status.state === "running"),
          ),
        );
        const live = runs.filter(isWorking).length;
        const waiting = runs.filter(needsAttention).length;
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
                <span className="proj-icon" aria-hidden style={{ background: projectAccent(p) }}>
                  {p.name.slice(0, 1).toUpperCase()}
                </span>
                <span className="home-group-name">{p.name}</span>
                <span className="home-group-meta">
                  {runs.length === 0 ? "no agents" : `${runs.length} agent${runs.length === 1 ? "" : "s"}`}
                  {live > 0 && <span className="home-live"> · {live} working</span>}
                  {waiting > 0 && <span className="home-attn"> · {waiting} waiting</span>}
                </span>
                <span className="home-group-open">open →</span>
              </button>
            </div>
            {!isFolded && runs.length > 0 && (
              <div className="grid home-grid">
                {runs.map((r) => (
                  <HomeTile key={r.id} run={r} onOpen={() => onOpenRun(p, r.id)} onChanged={tick} />
                ))}
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}

export function Stat({ value, label, accent, warn }: { value: number; label: string; accent?: boolean; warn?: boolean }) {
  return (
    <span className={`home-stat ${accent ? "accent" : ""} ${warn ? "warn" : ""}`}>
      <span className="home-stat-n">{value}</span> {label}
    </span>
  );
}

function badgeClass(agent: string): string {
  return ["claude", "pi", "hermes"].includes(agent) ? `badge ${agent}` : "badge";
}

// Same visual language as the per-project AgentTile, minus the discard button:
// the overview is for surveying and jumping in, not for tearing down. Settle,
// snooze and pin are the exception, because they *are* surveying — this is the
// view whose whole job is "which of these needs me", so the answer belongs
// where the question is asked.
function HomeTile({ run, onOpen, onChanged }: { run: RunInfo; onOpen: () => void; onChanged: () => void }) {
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

  const st = runStatus(run);
  const isTerminal = run.kind === "terminal";
  return (
    <div className="tile" onClick={onOpen} title="Open in focus mode">
      <div className="tile-head">
        <span className={`dot ${st.cls}`} />
        <span className="tile-title">{runName(run)}</span>
        {run.runScriptsLive && (
          <span className="run-dot" title="A run script is running in this workspace" />
        )}
        <span className={isTerminal ? "badge" : badgeClass(run.agent)}>{isTerminal ? "terminal" : run.agent}</span>
      </div>
      {!isTerminal && (
        <div className="tile-meta">
          {inGitlessFolder(run) ? (
            <span className="badge" title="Works in the project folder, which is not a git repository">in folder</span>
          ) : (
            <>
              <code>{run.branch}</code>
              <span className="diffstat"><span className="add">+{run.added}</span> <span className="del">−{run.deleted}</span> · {run.files}f</span>
              {!run.worktree && <span className="badge" title="Works in the project checkout, not an isolated worktree">in checkout</span>}
            </>
          )}
        </div>
      )}
      {preview && <pre className="tile-preview">{preview}</pre>}
      <div className="tile-foot">
        <span title={st.title}>{st.text}</span>
        <AttentionMarker run={run} onChanged={onChanged} />
      </div>
    </div>
  );
}
