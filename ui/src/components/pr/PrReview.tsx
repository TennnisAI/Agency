import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  PrDetail,
  PrFileDiff,
  ReviewEvent,
  ReviewThread,
  prDetail,
  prDiff,
  prReviewThreads,
  replyPrComment,
  resolvePrThread,
  submitPrReview,
  unresolvePrThread,
} from "../../api";
import { toastError, toastSuccess } from "../../lib/toast";
import Markdown from "../Markdown";
import PrDiffFile from "./PrDiffFile";
import type { DraftEntry } from "./anchor";

const uid = (): string =>
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `d${Date.now()}${Math.round(Math.random() * 1e6)}`;

const DECISION_LABEL: Record<string, string> = {
  APPROVED: "Approved",
  CHANGES_REQUESTED: "Changes requested",
  REVIEW_REQUIRED: "Review required",
};

// The review surface for one PR: header + description, the full diff with inline
// threads and pending comments, and a submit bar that posts a verdict + all
// pending comments as one GitHub review.
export default function PrReview({ projectId, number }: { projectId: string; number: number }) {
  const [detail, setDetail] = useState<PrDetail | null>(null);
  const [files, setFiles] = useState<PrFileDiff[]>([]);
  const [threads, setThreads] = useState<ReviewThread[]>([]);
  const [drafts, setDrafts] = useState<DraftEntry[]>([]);
  const [summary, setSummary] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const reloadThreads = useCallback(async () => {
    setThreads(await prReviewThreads(projectId, number));
  }, [projectId, number]);

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const [d, f, t] = await Promise.all([
        prDetail(projectId, number),
        prDiff(projectId, number),
        prReviewThreads(projectId, number),
      ]);
      setDetail(d);
      setFiles(f);
      setThreads(t);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [projectId, number]);

  useEffect(() => {
    // Reset per-PR editing state when switching PRs.
    setDrafts([]);
    setSummary("");
    load();
  }, [load]);

  const onReply = useCallback(
    async (inReplyTo: number, body: string) => {
      await replyPrComment(projectId, number, inReplyTo, body);
      await reloadThreads();
    },
    [projectId, number, reloadThreads],
  );

  const onToggleResolved = useCallback(
    async (threadId: string, resolved: boolean) => {
      if (resolved) await resolvePrThread(projectId, threadId);
      else await unresolvePrThread(projectId, threadId);
      await reloadThreads();
    },
    [projectId, number, reloadThreads],
  );

  async function submit(event: ReviewEvent) {
    setSubmitting(true);
    try {
      const comments = drafts.map(({ id: _id, ...c }) => c);
      await submitPrReview(projectId, number, event, summary.trim() || null, comments);
      setDrafts([]);
      setSummary("");
      toastSuccess(
        event === "APPROVE" ? "Approved" : event === "REQUEST_CHANGES" ? "Changes requested" : "Review submitted",
      );
      await load();
    } catch (e) {
      toastError(e, "Couldn't submit review");
    } finally {
      setSubmitting(false);
    }
  }

  if (loading) return <div className="pr-review-empty"><span className="spinner" /> Loading PR…</div>;
  if (error) return <div className="git-error">{error}</div>;
  if (!detail) return <div className="pr-review-empty">PR #{number} not found.</div>;

  const badge = detail.isDraft ? "DRAFT" : detail.state;
  // Request-changes/Comment need a body or a pending comment; approve is always allowed.
  const canVerdict = summary.trim().length > 0 || drafts.length > 0;

  return (
    <div className="pr-review">
      <div className="pr-review-scroll">
        <div className="pr-review-header">
          <div className="pr-review-titlerow">
            <span className={`pr-state pr-state-${detail.state.toLowerCase()}`}>{badge}</span>
            <span className="pr-review-title">
              {detail.title} <span className="pr-review-num">#{detail.number}</span>
            </span>
            <span className="spacer" style={{ flex: 1 }} />
            <button className="settings-ghost-btn" onClick={() => openUrl(detail.url).catch((e) => toastError(e, "Couldn't open the PR"))}>
              Open ↗
            </button>
          </div>
          <div className="pr-review-meta">
            <code>{detail.headRefName}</code> → <code>{detail.baseRefName}</code>
            {detail.author.login && <span className="pr-review-author">by {detail.author.login}</span>}
            {detail.mergeable === "CONFLICTING" && <span className="pr-review-conflict">conflicts</span>}
            {detail.reviewDecision && (
              <span className="pr-review-decision">{DECISION_LABEL[detail.reviewDecision] ?? detail.reviewDecision}</span>
            )}
          </div>
          {detail.body.trim() && <Markdown className="pr-review-desc" text={detail.body} />}
        </div>

        <div className="pr-review-files">
          {files.length === 0 && <div className="pr-review-empty">No file changes in this PR.</div>}
          {files.map((f) => (
            <PrDiffFile
              key={f.path}
              file={f}
              threads={threads.filter((t) => t.path === f.path)}
              drafts={drafts.filter((d) => d.path === f.path)}
              onAddDraft={(d) => setDrafts((prev) => [...prev, { ...d, id: uid() }])}
              onRemoveDraft={(id) => setDrafts((prev) => prev.filter((d) => d.id !== id))}
              onReply={onReply}
              onToggleResolved={onToggleResolved}
            />
          ))}
        </div>
      </div>

      <div className="pr-review-submit">
        <textarea
          className="settings-input pr-review-summary"
          placeholder="Review summary (optional for approve)…"
          value={summary}
          onChange={(e) => setSummary(e.target.value)}
        />
        <div className="pr-review-verdicts">
          <span className="pr-review-draftcount">
            {drafts.length > 0 ? `${drafts.length} pending comment${drafts.length === 1 ? "" : "s"}` : "No pending comments"}
          </span>
          <span className="spacer" style={{ flex: 1 }} />
          <button
            className="git-iconbtn"
            disabled={submitting || !canVerdict}
            title={canVerdict ? "Comment without a verdict" : "Add a summary or a comment first"}
            onClick={() => submit("COMMENT")}
          >
            Comment
          </button>
          <button
            className="git-iconbtn"
            disabled={submitting || !canVerdict}
            title={canVerdict ? "Request changes" : "Add a summary or a comment first"}
            onClick={() => submit("REQUEST_CHANGES")}
          >
            Request changes
          </button>
          <button className="git-iconbtn pr-approve" disabled={submitting} onClick={() => submit("APPROVE")}>
            {submitting ? "Submitting…" : "Approve"}
          </button>
        </div>
      </div>
    </div>
  );
}
