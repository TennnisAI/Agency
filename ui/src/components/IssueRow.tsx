import { useRef, useState } from "react";
import { Issue, IssuePatch, IssueStatus, RunInfo } from "../api";
import { ISSUE_STATUSES, PRIORITY_LABELS, STATUS_COLORS, STATUS_LABELS } from "../lib/issues";
import AgentAddMenu from "./AgentAddMenu";

// Priority as Linear-style signal bars: 1-3 bars for low/medium/high, an
// exclamation block for urgent, a dash for none.
export function PriorityGlyph({ priority }: { priority: number }) {
  if (priority === 4) return <span className="issue-prio urgent" title="Urgent">!</span>;
  if (priority === 0) return <span className="issue-prio none" title="No priority">–</span>;
  return (
    <span className="issue-prio" title={PRIORITY_LABELS[priority]}>
      {[1, 2, 3].map((n) => (
        <i key={n} className={n <= priority ? "on" : ""} style={{ height: 3 + n * 2 }} />
      ))}
    </span>
  );
}

export function StatusDot({ status }: { status: IssueStatus }) {
  return (
    <span
      className={`issue-status-dot${status === "cancelled" ? " cancelled" : ""}`}
      style={{ background: `var(--${STATUS_COLORS[status]})` }}
    />
  );
}

// The live activity light for an issue's linked runs: pulsing while any
// linked agent session is running, else quiet. Nothing linked = nothing shown.
function runActivity(runs: RunInfo[]): { cls: string; title: string } | null {
  if (runs.length === 0) return null;
  const running = runs.filter((r) => r.status.state === "running").length;
  if (running > 0) return { cls: "running", title: `${running} agent${running === 1 ? "" : "s"} running` };
  return { cls: "exited", title: `${runs.length} linked run${runs.length === 1 ? "" : "s"}` };
}

export default function IssueRow({
  issue,
  label,
  runs,
  selected,
  onSelect,
  onStart,
  onSpawnAgent,
  onPatch,
  onDelete,
}: {
  issue: Issue;
  label: string;
  runs: RunInfo[];
  selected: boolean;
  onSelect: () => void;
  // Plain click on ▶ = default agent; the caret next to it opens the full menu.
  onStart: () => void;
  onSpawnAgent: (agentId: string, opts?: { base: string; mergeTarget: string }) => void;
  onPatch: (patch: IssuePatch) => void;
  onDelete: () => void;
}) {
  const [menu, setMenu] = useState<"status" | "more" | null>(null);
  const [coords, setCoords] = useState<{ top: number; left?: number; right?: number }>({ top: 0, left: 0 });
  const statusRef = useRef<HTMLButtonElement>(null);
  const moreRef = useRef<HTMLButtonElement>(null);

  const activity = runActivity(runs);
  const startable = issue.status !== "done" && issue.status !== "cancelled";

  // Same fixed-coords trick as AgentAddMenu so ancestors' overflow can't clip.
  // These triggers sit at the far right of the row, so left-anchoring would run
  // the menu off the right edge (worse when the sidebar is closed and the row is
  // wide). Flip to right-anchor (open leftward) whenever there isn't room.
  const openMenu = (which: "status" | "more", ref: React.RefObject<HTMLButtonElement>) => {
    const r = ref.current?.getBoundingClientRect();
    if (r) {
      const MENU_W = 300; // .agent-menu max-width
      const fitsRight = r.left + MENU_W <= window.innerWidth - 8;
      setCoords(
        fitsRight
          ? { top: r.bottom + 4, left: r.left }
          : { top: r.bottom + 4, right: window.innerWidth - r.right },
      );
    }
    setMenu(which);
  };

  return (
    <div className={`issue-row${selected ? " selected" : ""}`} onClick={onSelect}>
      <PriorityGlyph priority={issue.priority} />
      <code className="issue-key">{label}</code>
      <span className={`issue-title${issue.status === "cancelled" ? " cancelled" : ""}`}>{issue.title}</span>
      {activity && <span className={`dot ${activity.cls}`} title={activity.title} />}
      <span className="issue-row-actions" onClick={(e) => e.stopPropagation()}>
        {startable && (
          <>
            <button className="icon-btn" title="Start agent on this issue" onClick={onStart}>▶</button>
            <AgentAddMenu
              variant="icon"
              projectId={issue.projectId}
              issue={issue}
              issueLabel={label}
              onSpawn={onSpawnAgent}
              onTerminal={() => {}}
            />
          </>
        )}
        <button
          ref={statusRef}
          className="issue-status-pill"
          title="Change status"
          onClick={() => openMenu("status", statusRef)}
        >
          <StatusDot status={issue.status} />
          {STATUS_LABELS[issue.status]}
        </button>
        <button ref={moreRef} className="icon-btn" title="More" onClick={() => openMenu("more", moreRef)}>⋯</button>
      </span>

      {menu && (
        <>
          <div className="agent-menu-backdrop" onClick={(e) => { e.stopPropagation(); setMenu(null); }} />
          <div className="agent-menu" style={{ position: "fixed", ...coords }} onClick={(e) => e.stopPropagation()}>
            {menu === "status" &&
              ISSUE_STATUSES.map((s) => (
                <button key={s} onClick={() => { setMenu(null); if (s !== issue.status) onPatch({ status: s }); }}>
                  <StatusDot status={s} /> {STATUS_LABELS[s]}{s === issue.status ? " ✓" : ""}
                </button>
              ))}
            {menu === "more" && (
              <>
                {PRIORITY_LABELS.map((p, n) => (
                  <button key={p} onClick={() => { setMenu(null); if (n !== issue.priority) onPatch({ priority: n }); }}>
                    {p}{n === issue.priority ? " ✓" : ""}
                  </button>
                ))}
                <div className="agent-menu-sep" />
                <button onClick={() => { setMenu(null); onDelete(); }}>Delete issue…</button>
              </>
            )}
          </div>
        </>
      )}
    </div>
  );
}
