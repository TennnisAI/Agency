import { useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Issue, IssuePatch, IssueStatus, RunInfo } from "../api";
import { ISSUE_STATUSES, PRIORITY_LABELS, STATUS_COLORS, STATUS_LABELS, fmtDate, isOverdue, matchRanges } from "../lib/issues";
import { dateStamp } from "../lib/dailyNote";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { SpawnOpts } from "../store/runs";
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

// Text with the active search terms lit up. No terms (or no hit) renders the
// plain string, so a row costs nothing extra when nobody is searching.
function Hits({ text, terms }: { text: string; terms?: string[] }) {
  const ranges = terms && terms.length > 0 ? matchRanges(text, terms) : [];
  if (ranges.length === 0) return <>{text}</>;
  const out: React.ReactNode[] = [];
  let at = 0;
  ranges.forEach(([start, end], i) => {
    if (start > at) out.push(text.slice(at, start));
    out.push(<mark key={i} className="issue-hit">{text.slice(start, end)}</mark>);
    at = end;
  });
  if (at < text.length) out.push(text.slice(at));
  return <>{out}</>;
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
  drag,
  terms,
  gitless = false,
}: {
  issue: Issue;
  label: string;
  runs: RunInfo[];
  selected: boolean;
  onSelect: () => void;
  // Plain click on ▶ = default agent; the caret next to it opens the full menu.
  onStart: () => void;
  // The issue's project folder has no git repository: the agent takes the issue
  // on in the folder itself, and racing or looping it is hidden (see
  // AgentAddMenu). The cross-project board leaves this false — it has no
  // per-project readiness to hand — and relies on the backend's refusal.
  gitless?: boolean;
  onSpawnAgent: (agentId: string, opts?: SpawnOpts) => void;
  onPatch: (patch: IssuePatch) => void;
  onDelete: () => void;
  // Manual reorder within a status group (the per-project board wires this;
  // the cross-project board doesn't — reordering across projects is
  // meaningless). Pointer-based, not HTML5 DnD: Tauri's native drag-drop
  // layer swallows in-page drops on macOS, so the drop event never arrives.
  drag?: {
    over: "above" | "below" | null;
    // True while this row is the one being dragged.
    source: boolean;
    idx: number;
    status: IssueStatus;
    onMouseDown: (e: React.MouseEvent) => void;
  };
  // Active search terms, lit up in the key and title.
  terms?: string[];
}) {
  const [menu, setMenu] = useState<"status" | "priority" | "more" | null>(null);
  const [coords, setCoords] = useState<{ top: number; left?: number; right?: number }>({ top: 0, left: 0 });
  const statusRef = useRef<HTMLButtonElement>(null);
  const moreRef = useRef<HTMLButtonElement>(null);
  const prioRef = useRef<HTMLButtonElement>(null);
  // The agent menu lives inside AgentAddMenu; the row only needs to know it is
  // open so the actions don't fade out from under it.
  const [agentOpen, setAgentOpen] = useState(false);

  const activity = runActivity(runs);
  const startable = issue.status !== "done" && issue.status !== "cancelled";

  // Same fixed-coords trick as AgentAddMenu so ancestors' overflow can't clip.
  // These triggers sit at the far right of the row, so left-anchoring would run
  // the menu off the right edge (worse when the sidebar is closed and the row is
  // wide). Flip to right-anchor (open leftward) whenever there isn't room.
  const openMenu = (which: "status" | "priority" | "more", ref: React.RefObject<HTMLButtonElement>) => {
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

  // The coords above are a snapshot of where the trigger was; a resize moves
  // the row out from under the menu, so close it instead of leaving it hanging.
  useDismissOnResize(menu !== null, () => setMenu(null));

  const today = dateStamp(new Date());

  return (
    <div
      className={`issue-row${selected ? " selected" : ""}${menu || agentOpen ? " menu-open" : ""}${drag?.over ? ` drop-${drag.over}` : ""}${drag?.source ? " dragging" : ""}`}
      onClick={onSelect}
      onMouseDown={drag?.onMouseDown}
      data-issue-idx={drag?.idx}
      data-issue-status={drag?.status}
    >
      <button
        ref={prioRef}
        className="issue-prio-btn"
        title="Change priority"
        onClick={(e) => { e.stopPropagation(); openMenu("priority", prioRef); }}
      >
        <PriorityGlyph priority={issue.priority} />
      </button>
      <code className="issue-key"><Hits text={label} terms={terms} /></code>
      <span className={`issue-title${issue.status === "cancelled" ? " cancelled" : ""}`}>
        <Hits text={issue.title} terms={terms} />
      </span>
      {issue.due && (
        <span
          className={`issue-due${isOverdue(issue, today) ? " overdue" : ""}`}
          title={`Due ${issue.due}`}
        >
          ◷ {fmtDate(issue.due, today)}
        </span>
      )}
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
              onOpenChange={setAgentOpen}
              gitless={gitless}
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

      {/* Into the body, not the row. The sidebar list floats its actions strip
          with a `transform`, and a transformed ancestor becomes the containing
          block for `position: fixed` — viewport coords would be read against
          that little box and the menu would open off in the weeds. The strip
          also hides itself once the pointer leaves the row, which the menu's
          own backdrop causes. */}
      {menu && createPortal(
        <>
          <div className="agent-menu-backdrop" onClick={(e) => { e.stopPropagation(); setMenu(null); }} />
          <div className="agent-menu" style={{ position: "fixed", ...coords }} onClick={(e) => e.stopPropagation()}>
            {menu === "status" &&
              ISSUE_STATUSES.map((s) => (
                <button key={s} onClick={() => { setMenu(null); if (s !== issue.status) onPatch({ status: s }); }}>
                  <StatusDot status={s} /> {STATUS_LABELS[s]}{s === issue.status ? " ✓" : ""}
                </button>
              ))}
            {menu === "priority" &&
              PRIORITY_LABELS.map((p, n) => (
                <button key={p} onClick={() => { setMenu(null); if (n !== issue.priority) onPatch({ priority: n }); }}>
                  <PriorityGlyph priority={n} /> {p}{n === issue.priority ? " ✓" : ""}
                </button>
              ))}
            {menu === "more" && (
              <>
                {PRIORITY_LABELS.map((p, n) => (
                  <button key={p} onClick={() => { setMenu(null); if (n !== issue.priority) onPatch({ priority: n }); }}>
                    <PriorityGlyph priority={n} /> {p}{n === issue.priority ? " ✓" : ""}
                  </button>
                ))}
                <div className="agent-menu-sep" />
                <button onClick={() => { setMenu(null); onDelete(); }}>Delete issue…</button>
              </>
            )}
          </div>
        </>,
        document.body,
      )}
    </div>
  );
}
