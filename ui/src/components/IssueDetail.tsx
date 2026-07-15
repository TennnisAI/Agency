import { useEffect, useState } from "react";
import { Issue, IssuePatch, IssueStatus, RunInfo } from "../api";
import { runName } from "../agents";
import { ISSUE_STATUSES, PRIORITY_LABELS, STATUS_LABELS } from "../lib/issues";

function ts(secs: number): string {
  return new Date(secs * 1000).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
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

  // Reset drafts when another issue is selected — but never clobber an edit
  // in progress with poll results for the same issue.
  useEffect(() => {
    setTitle(issue.title);
    setBody(issue.body);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issue.id]);

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
      <input
        className="settings-input issue-detail-title"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={commitTitle}
        onKeyDown={(e) => { if (e.key === "Enter") (e.target as HTMLInputElement).blur(); }}
      />
      <div className="issue-detail-fields">
        <label className="branch-row">
          <span>status</span>
          <select value={issue.status} onChange={(e) => onPatch({ status: e.target.value as IssueStatus })}>
            {ISSUE_STATUSES.map((s) => (
              <option key={s} value={s}>{STATUS_LABELS[s]}</option>
            ))}
          </select>
        </label>
        <label className="branch-row">
          <span>priority</span>
          <select value={issue.priority} onChange={(e) => onPatch({ priority: Number(e.target.value) })}>
            {PRIORITY_LABELS.map((p, n) => (
              <option key={p} value={n}>{p}</option>
            ))}
          </select>
        </label>
      </div>
      <textarea
        className="settings-input issue-detail-body"
        placeholder="Add a description — it is sent to the agent as part of the prompt."
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
