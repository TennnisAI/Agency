import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AgentProfile, Issue, getSettings, listProfiles, listProjectBranches } from "../api";
import { agentLabel } from "../agents";
import { useRuns, SpawnOpts } from "../store/runs";
import { effectiveMergeTarget } from "../lib/branchTargets";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { useAgentModels } from "../hooks/useAgentModels";
import BranchSelect from "./BranchSelect";
import ModelSelect from "./ModelSelect";
import RaceDialog from "./RaceDialog";
import LoopDialog from "./LoopDialog";
import GhImportDialog from "./GhImportDialog";

export default function AgentAddMenu({
  onSpawn,
  onTerminal,
  variant = "button",
  projectId,
  issue,
  issueLabel,
  gitless = false,
  onOpenChange,
}: {
  onSpawn: (agentId: string, opts?: SpawnOpts) => void;
  // Required (not optional) so every call site exposes the same options — the two
  // add-menus (top-right "+ Agent" and the rail "Agents +" header) cannot drift
  // out of sync.
  onTerminal: () => void;
  variant?: "button" | "icon" | "header";
  projectId?: string;
  // Issue-dispatch context: the same menu (agents, race, loop, branch pickers)
  // minus the entries that make no sense for an issue (terminal, GitHub
  // imports). onSpawn then routes to startIssueRun at the call site.
  issue?: Issue;
  issueLabel?: string;
  // Project folder with no git repository: agents still spawn, but in the
  // folder itself, so everything that needs a branch to exist (races, loops,
  // GitHub import, the base/target pickers) is hidden.
  gitless?: boolean;
  // Lets a host that hides its controls on hover (the issue rows) keep them up
  // while this menu is open.
  onOpenChange?: (open: boolean) => void;
}) {
  // While a workspace is being created, the triggers are disabled so the
  // slow first spawn can't be double-fired.
  const { spawning } = useRuns();
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; left?: number; right?: number }>();
  const [agents, setAgents] = useState<AgentProfile[]>([]);
  const [raceOpen, setRaceOpen] = useState(false);
  const [loopOpen, setLoopOpen] = useState(false);
  const [importMode, setImportMode] = useState<"issue" | "pr" | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);

  // Model per agent, seeded from what each was last launched on, so the row
  // reads as the spawn it is about to do. Picking one here changes only this
  // menu's next spawn; it becomes the remembered choice once a run starts on
  // it. Refreshed on open, alongside the agent list.
  const { models, reload: reloadModels, remembered } = useAgentModels();
  const [picked, setPicked] = useState<Record<string, string | null>>({});
  const modelFor = (agent: string) =>
    agent in picked ? picked[agent] : remembered(agent);

  // Kept in a ref so a host passing a fresh closure each render doesn't
  // re-announce the same state.
  const announce = useRef(onOpenChange);
  announce.current = onOpenChange;
  useEffect(() => {
    announce.current?.(open);
    return () => { if (open) announce.current?.(false); };
  }, [open]);

  // Agent options are derived from the defined profiles (not hardcoded) so newly
  // added definitions show up. Both add-menus read the same source, so they stay
  // in sync. Refresh on open so a profile added moments ago appears immediately.
  const loadAgents = () =>
    listProfiles()
      .then(setAgents)
      .catch(() => {});
  useEffect(() => { loadAgents(); }, []);

  // Branch-picker state (only used when projectId is supplied).
  const [branches, setBranches] = useState<string[]>([]);
  const [current, setCurrent] = useState<string>("");
  const [base, setBase] = useState<string>("");
  const [targetOverride, setTargetOverride] = useState<string | null>(null);
  // Isolate the agent in its own worktree + branch, or let it work in the
  // project checkout as it stands. Reset to the Settings default every time the
  // menu opens, so a box ticked for one spawn never carries silently into the
  // next. The last known default is kept in a ref as the fallback for a
  // re-read that fails.
  const [worktree, setWorktree] = useState(true);
  const defaultWorktree = useRef(true);
  const loadWorktreeDefault = () =>
    getSettings()
      .then((s) => { defaultWorktree.current = s.defaultWorktree; setWorktree(s.defaultWorktree); })
      .catch(() => setWorktree(defaultWorktree.current));
  useEffect(() => { loadWorktreeDefault(); }, []);

  const showPicker = !!projectId;
  // Issue dispatch merges the run's branch to close the issue, so it is always
  // isolated and the checkbox is hidden for it. With no repository there is
  // nothing to branch from at all, so every spawn works in the folder.
  const wantsWorktree = !gitless && (!!issue || worktree);
  const mergeTarget = effectiveMergeTarget(base, targetOverride);
  const diverged = targetOverride !== null && targetOverride !== base;

  // Load branches when the menu opens (cheap; reflects any branch the user just
  // made). A folder with no repository has none to list.
  useEffect(() => {
    if (!open || !projectId || gitless) return;
    let live = true;
    listProjectBranches(projectId)
      .then((pb) => {
        if (!live) return;
        setBranches(pb.branches);
        setCurrent(pb.current);
        setBase((b) => (b && pb.branches.includes(b) ? b : pb.current));
      })
      .catch(() => { /* leave selects empty; spawn falls back to HEAD */ });
    return () => { live = false; };
  }, [open, projectId]);

  // Anchor the menu in viewport coordinates so an ancestor's `overflow: hidden`
  // (e.g. the focus rail) can't clip it. The icon/header buttons sit at the left
  // of their pane and open rightward; the primary button sits at the top-right
  // and opens leftward.
  const toggle = () => {
    setOpen((o) => {
      const next = !o;
      if (next) {
        loadAgents();
        // Re-read: Settings may have changed since this menu last mounted.
        loadWorktreeDefault();
        // And a run started elsewhere may have moved an agent's model on.
        reloadModels();
        setPicked({});
      }
      if (next && btnRef.current) {
        const r = btnRef.current.getBoundingClientRect();
        // Icon/header triggers open rightward, but near the right edge (e.g. an
        // issue row with the sidebar closed) that clips. Flip to right-anchor
        // when the menu wouldn't fit. The primary button always opens leftward.
        const MENU_W = 300; // .agent-menu max-width
        const fitsRight = r.left + MENU_W <= window.innerWidth - 8;
        setCoords(
          variant === "button" || !fitsRight
            ? { top: r.bottom + 4, right: window.innerWidth - r.right }
            : { top: r.bottom + 4, left: r.left },
        );
      }
      return next;
    });
  };

  // Those coords are measured once, at open. Rather than let a resize leave the
  // menu behind where its trigger used to be, close it.
  useDismissOnResize(open, () => setOpen(false));

  const choose = (id: string) => {
    setOpen(false);
    // Stated on every spawn, like the worktree flag below: what the row showed
    // is what starts, whether it came from the last run or from a pick just
    // made here. `null` is the agent's own default, which is a real choice.
    const model = modelFor(id);
    if (!showPicker) return onSpawn(id, { model });
    // The flag is always stated, never left to the Settings default: the box
    // in front of the user is what this spawn does, whichever way it was set.
    // Issue dispatch has no box (it always cuts a worktree), so a default of
    // "off" must not leak into it. Without a worktree the branch selects are
    // inert, so base falls back to the live branch.
    onSpawn(id, {
      base: (wantsWorktree ? base : current) || "HEAD",
      mergeTarget,
      worktree: wantsWorktree,
      model,
    });
  };
  const chooseTerminal = () => { setOpen(false); onTerminal(); };

  return (
    <div className="agent-add">
      {variant === "header" ? (
        <button ref={btnRef} className="rail-head-add" title="Add agent" disabled={spawning} onClick={toggle}>Agents <span className="rail-head-plus">+</span></button>
      ) : variant === "icon" ? (
        <button ref={btnRef} className="icon-btn" title={issue ? "Start agent…" : "Add agent"} disabled={spawning} onClick={(e) => { e.stopPropagation(); toggle(); }}>{issue ? "▾" : "+"}</button>
      ) : (
        // Parts, not one string: the label collapses away in a narrow header
        // while the "+" and the caret stay, and nothing can wrap mid-button.
        <button
          ref={btnRef}
          className="btn-primary btn-add"
          title="New agent"
          disabled={spawning}
          onClick={toggle}
        >
          {spawning ? (
            <span className="btn-add-label">Starting…</span>
          ) : (
            <>
              <span className="btn-add-plus" aria-hidden>+</span>
              <span className="btn-add-label">Agent</span>
              <span className="btn-caret" aria-hidden>▾</span>
            </>
          )}
        </button>
      )}
      {/* Into the body, so nothing between here and the viewport can move or
          hide the menu. The coords are the viewport's, and any transformed
          ancestor would become the containing block for `position: fixed` and
          re-read them as its own — which is exactly what the sidebar issue
          list's floated actions strip does to the trigger sitting in it. React
          events still bubble along the component tree. */}
      {open && createPortal(
        <>
          <div className="agent-menu-backdrop" onClick={() => setOpen(false)} />
          <div className="agent-menu" style={{ position: "fixed", ...coords }}>
            {/* Agent and model on one row: the name starts the agent, the chip
                beside it says which model it will start on and opens the list.
                Agents whose CLI takes no model flag show no chip. */}
            {agents.map((a) => (
              <div key={a.name} className="agent-menu-row">
                <button className="agent-menu-name" onClick={() => choose(a.name)}>
                  {agentLabel(a.name)}
                </button>
                <ModelSelect
                  compact
                  info={models[a.name]}
                  projectId={projectId ?? null}
                  value={modelFor(a.name)}
                  onChange={(m) => setPicked((p) => ({ ...p, [a.name]: m }))}
                />
              </div>
            ))}
            {/* A race gives every attempt its own branch and a loop respawns on
                one, so both need a repository to exist. */}
            {!gitless && (
              <>
                <div className="agent-menu-sep" />
                <button onClick={() => { setOpen(false); setRaceOpen(true); }}>∥ Race agents…</button>
                <button onClick={() => { setOpen(false); setLoopOpen(true); }}>⟳ Loop agent…</button>
              </>
            )}
            {!issue && (
              <>
                {/* Importing checks the issue's or PR's branch out. */}
                {!gitless && (
                  <>
                    <button onClick={() => { setOpen(false); setImportMode("issue"); }}>◈ GitHub issue…</button>
                    <button onClick={() => { setOpen(false); setImportMode("pr"); }}>⇋ GitHub PR…</button>
                  </>
                )}
                <div className="agent-menu-sep" />
                <button onClick={chooseTerminal}>≳ New terminal</button>
              </>
            )}
            {showPicker && branches.length > 0 && !gitless && (
              <>
                <div className="agent-menu-sep" />
                <div className="branch-picker">
                  {/* Issue dispatch stays worktree-only: merging the run's
                      branch is what closes the issue. */}
                  {!issue && (
                    <label className="branch-row branch-check" onClick={(e) => e.stopPropagation()}>
                      <input
                        type="checkbox"
                        checked={worktree}
                        onChange={(e) => setWorktree(e.target.checked)}
                      />
                      <span>Own worktree</span>
                    </label>
                  )}
                  {/* Both states occupy the same grid cell, so the menu keeps
                      the taller one's height and doesn't resize as the box is
                      ticked. `visibility` also takes the inert one out of the
                      tab order. */}
                  <div className="branch-swap">
                    <div className={`branch-fields${wantsWorktree ? "" : " off"}`}>
                      <div className="branch-row">
                        <span>from</span>
                        <BranchSelect
                          value={base}
                          options={branches}
                          onChange={setBase}
                          label="base branch"
                        />
                      </div>
                      <div className="branch-row">
                        <span>into</span>
                        <BranchSelect
                          value={mergeTarget}
                          options={branches}
                          onChange={(b) => setTargetOverride(b === base ? null : b)}
                          label="merge target"
                        />
                        {diverged && (
                          <button
                            type="button"
                            className="branch-reset"
                            title="Sync merge target to base"
                            onClick={(e) => { e.stopPropagation(); setTargetOverride(null); }}
                          >↺</button>
                        )}
                      </div>
                    </div>
                    <div className={`branch-note${wantsWorktree ? " off" : ""}`}>
                      Works in your checkout on <code>{current || "the current branch"}</code>,
                      nothing to merge.
                    </div>
                  </div>
                </div>
              </>
            )}
          </div>
        </>,
        document.body,
      )}
      {raceOpen && <RaceDialog onClose={() => setRaceOpen(false)} issue={issue} issueLabel={issueLabel} />}
      {loopOpen && <LoopDialog onClose={() => setLoopOpen(false)} issue={issue} issueLabel={issueLabel} />}
      {importMode && <GhImportDialog mode={importMode} onClose={() => setImportMode(null)} />}
    </div>
  );
}
