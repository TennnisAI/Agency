import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Issue, IssuePatch, RunInfo } from "../api";
import { runName } from "../agents";
import { ISSUE_STATUSES, PRIORITY_LABELS, STATUS_LABELS, fmtDate, isOverdue } from "../lib/issues";
import { dateStamp } from "../lib/dailyNote";
import { PriorityGlyph, StatusDot } from "./IssueRow";
import DatePicker from "./DatePicker";

function ts(secs: number): string {
  return new Date(secs * 1000).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}

// A date as a quiet property pill: reads as text ("◷ Due Aug 1"), the click
// opens the in-app calendar popover (native pickers can't be dismissed
// without choosing a date in this webview). Unset renders a ghost prompt;
// clearing is the ✕ that appears once a date is set.
function DateProp({
  glyph,
  label,
  value,
  overdue,
  onChange,
}: {
  glyph: string;
  label: string;
  value: string | null;
  overdue?: boolean;
  onChange: (v: string | null) => void;
}) {
  const pillRef = useRef<HTMLSpanElement>(null);
  const [picker, setPicker] = useState<{ top: number; left?: number; right?: number } | null>(null);
  const today = dateStamp(new Date());

  // Same edge-flip anchoring as the row menus: the detail pane hugs the
  // window's right edge, so the popover usually opens leftward.
  const openPicker = () => {
    const r = pillRef.current?.getBoundingClientRect();
    if (!r) return;
    const MENU_W = 240; // .date-picker width
    const fitsRight = r.left + MENU_W <= window.innerWidth - 8;
    setPicker(
      fitsRight
        ? { top: r.bottom + 4, left: r.left }
        : { top: r.bottom + 4, right: window.innerWidth - r.right },
    );
  };

  return (
    <>
      <span
        ref={pillRef}
        className={`issue-prop-pill date-pill${value ? "" : " date-empty"}${overdue ? " overdue" : ""}`}
        role="button"
        tabIndex={0}
        title={value ? `${label} ${value} (click to change)` : `Set ${label.toLowerCase()} date`}
        onClick={openPicker}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openPicker(); } }}
      >
        <span aria-hidden>{glyph}</span>
        {value ? `${label} ${fmtDate(value, today)}` : label}
        {value && (
          <button
            className="date-clear"
            title={`Clear ${label.toLowerCase()} date`}
            onClick={(e) => { e.stopPropagation(); onChange(null); }}
          >
            ✕
          </button>
        )}
      </span>
      {picker && (
        <DatePicker
          value={value}
          coords={picker}
          onPick={(d) => { setPicker(null); if (d !== value) onChange(d); }}
          onClear={() => { setPicker(null); onChange(null); }}
          onClose={() => setPicker(null)}
        />
      )}
    </>
  );
}

// Right-hand detail pane of the Issues view. Title/body commit on blur (and
// Enter for the title); status/priority commit immediately.
export default function IssueDetail({
  issue,
  label,
  runs,
  onPatch,
  onDelete,
  onOpenRun,
  onClose,
}: {
  issue: Issue;
  label: string;
  runs: RunInfo[];
  onPatch: (patch: IssuePatch) => void;
  onDelete: () => void;
  onOpenRun: (runId: string) => void;
  onClose: () => void;
}) {
  const [title, setTitle] = useState(issue.title);
  const [body, setBody] = useState(issue.body);
  const titleRef = useRef<HTMLTextAreaElement>(null);
  const [menu, setMenu] = useState<"status" | "priority" | null>(null);
  const [coords, setCoords] = useState<{ top: number; left: number }>({ top: 0, left: 0 });
  const statusRef = useRef<HTMLButtonElement>(null);
  const priorityRef = useRef<HTMLButtonElement>(null);

  const openMenu = (which: "status" | "priority", ref: React.RefObject<HTMLButtonElement>) => {
    const r = ref.current?.getBoundingClientRect();
    if (r) setCoords({ top: r.bottom + 4, left: r.left });
    setMenu(which);
  };

  // Reset drafts when another issue is selected — but never clobber an edit
  // in progress with poll results for the same issue.
  useEffect(() => {
    setTitle(issue.title);
    setBody(issue.body);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issue.id]);

  // Grow the title textarea to fit its wrapped content (no scroll, no clip).
  useLayoutEffect(() => {
    const el = titleRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [title]);

  const commitTitle = () => {
    const t = title.trim();
    if (t && t !== issue.title) onPatch({ title: t });
    else setTitle(issue.title);
  };
  const commitBody = () => {
    if (body !== issue.body) onPatch({ body });
  };

  return (
    <aside className="issue-detail">
      <div className="issue-detail-head">
        <code className="issue-key">{label}</code>
        <div className="spacer" />
        <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
      </div>
      <textarea
        ref={titleRef}
        className="issue-detail-title"
        rows={1}
        placeholder="Issue title"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={commitTitle}
        onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); (e.target as HTMLTextAreaElement).blur(); } }}
      />
      <div className="issue-detail-props">
        <button
          ref={statusRef}
          className="issue-prop-pill"
          title="Change status"
          onClick={() => openMenu("status", statusRef)}
        >
          <StatusDot status={issue.status} />
          {STATUS_LABELS[issue.status]}
        </button>
        <button
          ref={priorityRef}
          className="issue-prop-pill"
          title="Change priority"
          onClick={() => openMenu("priority", priorityRef)}
        >
          <PriorityGlyph priority={issue.priority} />
          {PRIORITY_LABELS[issue.priority]}
        </button>
        <DateProp
          glyph="◷"
          label="Due"
          value={issue.due}
          overdue={isOverdue(issue, dateStamp(new Date()))}
          onChange={(due) => onPatch({ due })}
        />
        <DateProp
          glyph="⧖"
          label="Scheduled"
          value={issue.scheduled}
          onChange={(scheduled) => onPatch({ scheduled })}
        />
      </div>

      {menu && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setMenu(null)} />
          <div className="agent-menu" style={{ position: "fixed", ...coords }}>
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
          </div>
        </>
      )}
      <textarea
        className="issue-detail-body"
        placeholder="Add description…"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        onBlur={commitBody}
      />
      {runs.length > 0 && (
        <div className="issue-detail-runs">
          <h3>Agents on this issue</h3>
          {runs.map((r) => (
            <button key={r.id} className="issue-detail-run" onClick={() => onOpenRun(r.id)}>
              <span className={`dot ${r.status.state === "running" ? "running" : "exited"}`} />
              <span className="issue-detail-run-name">{runName(r)}</span>
              <span className="badge">{r.agent}</span>
            </button>
          ))}
        </div>
      )}
      <div className="issue-detail-foot">
        <span title={`Updated ${ts(issue.updatedAt)}`}>Created {ts(issue.createdAt)}</span>
        <div className="spacer" />
        <button className="ghost" onClick={onDelete}>Delete</button>
      </div>
    </aside>
  );
}
