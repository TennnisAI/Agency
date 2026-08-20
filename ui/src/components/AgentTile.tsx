import { useEffect, useState } from "react";
import { RunInfo, runPreview } from "../api";
import { useRuns } from "../store/runs";
import { runName } from "../agents";
import { inGitlessFolder, runStatus } from "../lib/runstate";
import { Removal, removalLabel, removalsFor } from "../lib/runRemoval";
import { usageLabel, usageTitle } from "../lib/usage";
import QueuedMarker from "./QueuedMarker";
import RunRemoveDialog from "./RunRemoveDialog";
import OverflowMenu from "./OverflowMenu";
import { TrashIcon, InboxIcon } from "./icons";

function badgeClass(agent: string): string {
  if (agent === "claude") return "badge claude";
  if (agent === "pi") return "badge pi";
  if (agent === "hermes") return "badge hermes";
  return "badge";
}

export default function AgentTile({ run }: { run: RunInfo }) {
  const { setFocusedRun, setView } = useRuns();
  const [preview, setPreview] = useState("");
  // Archive / discard of this run, awaiting its confirm dialog.
  const [pending, setPending] = useState<Removal | null>(null);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const p = await runPreview(run.id, 12);
        if (alive) setPreview(p);
      } catch {
        /* ignore */
      }
    };
    tick();
    const t = window.setInterval(tick, 1500);
    return () => { alive = false; window.clearInterval(t); };
  }, [run.id]);

  const st = runStatus(run);
  const isTerminal = run.kind === "terminal";
  return (
    <div className="tile" onClick={() => { setFocusedRun(run.id); setView("focus"); }}>
      <div className="tile-head">
        <span className={`dot ${st.cls}`} />
        <span className="tile-title">{runName(run)}</span>
        {/* A run script is live in this workspace — a dev server, a build. The
            Run tab says which; here it is only "something is running". */}
        {run.runScriptsLive && (
          <span className="run-dot" title="A run script is running in this workspace" />
        )}
        {run.raceId && <span className="badge race" title="Racing: same prompt, parallel attempts">∥</span>}
        {/* Agent, and the model it was started on when that was chosen. Racing
            the same agent's models is only readable if the tile says which one
            each attempt is, and a run's model is otherwise invisible once it
            is running. Nothing shown for an agent left on its own default. */}
        <span
          className={isTerminal ? "badge" : badgeClass(run.agent)}
          title={run.model ? `${run.agent} on ${run.model}` : undefined}
        >
          {isTerminal ? "terminal" : run.agent}
          {run.model && <span className="badge-model">{run.model}</span>}
        </span>
      </div>
      {!isTerminal && (
        <div className="tile-meta">
          {inGitlessFolder(run) ? (
            <span className="badge" title="Works in the project folder, which is not a git repository">in folder</span>
          ) : (
            <>
              <code>{run.branch}</code>
              {/* Without a worktree the stat is uncommitted work in the checkout,
                  not a branch's diff against its base. */}
              <span className="diffstat" title={run.worktree ? "Changes on this branch" : "Uncommitted changes in the checkout"}>
                <span className="add">+{run.added}</span> <span className="del">−{run.deleted}</span> · {run.files}f
              </span>
              {!run.worktree && <span className="badge" title="Works in the project checkout, not an isolated worktree">in checkout</span>}
            </>
          )}
          {/* Absent, not zero, for agents whose transcript we cannot read. */}
          {usageLabel(run.usage) && (
            <span className="runcost" title={usageTitle(run.usage)}>{usageLabel(run.usage)}</span>
          )}
        </div>
      )}
      <pre className="tile-preview">{preview}</pre>
      <div className="tile-foot">
        <span title={st.title}>{st.text}</span>
        {/* Same register as the status above: what the agent is doing, and what
            it has not been handed yet. */}
        <QueuedMarker run={run} />
        {/* The same close button and menu the agents rail carries, so an agent
            offers the same ways out wherever it is listed: archived, keeping
            its branch for the Archived section, or discarded outright. The
            wrapper swallows clicks — on the button, the menu, or its backdrop —
            that would otherwise open the tile. */}
        <span className="tile-close-wrap" onClick={(e) => e.stopPropagation()}>
          <OverflowMenu
            buttonClass="hover-close tile-close"
            icon={<span aria-hidden>✕</span>}
            title={isTerminal ? "Close terminal" : "Archive or discard this agent"}
            items={removalsFor(run).map((action) => ({
              label: removalLabel(run, action),
              icon: action === "archive" ? <InboxIcon /> : <TrashIcon />,
              danger: action === "discard",
              onSelect: () => setPending(action),
            }))}
          />
        </span>
      </div>
      {pending && (
        <RunRemoveDialog run={run} action={pending} onClose={() => setPending(null)} />
      )}
    </div>
  );
}
