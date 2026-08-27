import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  PrDetail,
  PrFileDiff,
  ReviewEvent,
  ReviewThread,
  editPr,
  editPrComment,
  ghCurrentLogin,
  prDetail,
  prDiff,
  prReviewThreads,
  replyPrComment,
  resolvePrThread,
  submitPrReview,
  unresolvePrThread,
} from "../../api";
import { toastError, toastInfo, toastSuccess } from "../../lib/toast";
import Markdown from "../Markdown";
import AgentReviewDialog from "./AgentReviewDialog";
import MarkdownField from "./MarkdownField";
import MergePrDialog from "./MergePrDialog";
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
export default function PrReview({
  projectId,
  number,
  onMerged,
}: {
  projectId: string;
  number: number;
  // Lets the host panel refresh its list after a merge (the PR drops from open).
  onMerged?: () => void;
}) {
  const [detail, setDetail] = useState<PrDetail | null>(null);
  const [viewer, setViewer] = useState<string | null>(null);
  const [files, setFiles] = useState<PrFileDiff[]>([]);
  const [threads, setThreads] = useState<ReviewThread[]>([]);
  const [drafts, setDrafts] = useState<DraftEntry[]>([]);
  const [summary, setSummary] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [showMerge, setShowMerge] = useState(false);
  const [showAgentReview, setShowAgentReview] = useState(false);
  // Editing the PR's own title/description. Null when not editing; the drafts
  // are seeded from the loaded detail when the editor opens.
  const [edit, setEdit] = useState<{ title: string; body: string } | null>(null);
  const [savingEdit, setSavingEdit] = useState(false);

  const reloadThreads = useCallback(async () => {
    setThreads(await prReviewThreads(projectId, number));
  }, [projectId, number]);

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const [d, f, t, v] = await Promise.all([
        prDetail(projectId, number),
        prDiff(projectId, number),
        prReviewThreads(projectId, number),
        ghCurrentLogin(projectId).catch(() => null), // best-effort; only gates self-review UI
      ]);
      setDetail(d);
      setFiles(f);
      setThreads(t);
      setViewer(v);
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
    setEdit(null);
    load();
  }, [load]);

  const onReply = useCallback(
    async (inReplyTo: number, body: string) => {
      await replyPrComment(projectId, number, inReplyTo, body);
      await reloadThreads();
    },
    [projectId, number, reloadThreads],
  );

  const onEditComment = useCallback(
    async (commentId: number, body: string) => {
      await editPrComment(projectId, commentId, body);
      await reloadThreads();
    },
    [projectId, reloadThreads],
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
      // A 422 on a non-comment review most often means a line no longer maps to
      // the diff or a self-authored PR — give a more useful hint than gh's raw
      // "Unprocessable Entity".
      const msg = String(e);
      if (/422|unprocessable/i.test(msg) && event !== "COMMENT") {
        toastError(e, "GitHub rejected the review. You can't approve or request changes on your own PR, and comment lines must still be part of the diff.");
      } else {
        toastError(e, "Couldn't submit review");
      }
    } finally {
      setSubmitting(false);
    }
  }

  // Save the title/description rewrite. Only the fields that actually changed
  // are sent, so retyping nothing sends nothing and an untouched description is
  // never rewritten with the copy this pane happened to load.
  async function saveEdit() {
    if (!detail || !edit || !edit.title.trim()) return;
    const title = edit.title.trim() === detail.title ? null : edit.title.trim();
    const body = edit.body === detail.body ? null : edit.body;
    if (title === null && body === null) {
      setEdit(null);
      return;
    }
    setSavingEdit(true);
    try {
      await editPr(projectId, number, title, body);
      setEdit(null);
      await load();
    } catch (e) {
      // The drafts stay open: a rejected save must not discard the rewrite.
      toastError(e, "Couldn't save the pull request");
    } finally {
      setSavingEdit(false);
    }
  }

  if (loading) return <div className="pr-review-empty"><span className="spinner" /> Loading PR…</div>;
  if (error) return <div className="git-error">{error}</div>;
  if (!detail) return <div className="pr-review-empty">PR #{number} not found.</div>;

  const badge = detail.isDraft ? "DRAFT" : detail.state;
  // Request-changes/Comment need a body or a pending comment.
  const canVerdict = summary.trim().length > 0 || drafts.length > 0;
  // GitHub forbids approving or requesting changes on your own PR — only a
  // Comment review is allowed. Detect it so we disable those buttons up front
  // instead of failing with a 422.
  const isOwnPr = !!viewer && detail.author.login.toLowerCase() === viewer.toLowerCase();
  const ownPrHint = "You authored this PR. GitHub only lets you leave a Comment review on your own PR.";
  // The PR is mergeable when it's open, not a draft, and GitHub doesn't report a
  // conflict. UNKNOWN (still computing) is allowed — gh will refuse if it can't.
  const canMerge = detail.state === "OPEN" && !detail.isDraft && detail.mergeable !== "CONFLICTING";
  const mergeHint = detail.isDraft
    ? "This PR is a draft. Mark it ready before merging."
    : detail.mergeable === "CONFLICTING"
      ? "This PR has conflicts that must be resolved first."
      : "Merge this PR into its base branch.";

  return (
    <div className="pr-review">
      <div className="pr-review-scroll">
        <div className="pr-review-header">
          {edit ? (
            <div className="pr-review-edit">
              <div className="pr-review-titlerow">
                <span className={`pr-state pr-state-${detail.state.toLowerCase()}`}>{badge}</span>
                <input
                  className="settings-input pr-review-title-input"
                  value={edit.title}
                  autoFocus
                  placeholder="Title"
                  onChange={(e) => setEdit({ ...edit, title: e.target.value })}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") { e.preventDefault(); void saveEdit(); }
                    else if (e.key === "Escape") { e.stopPropagation(); setEdit(null); }
                  }}
                />
                <span className="pr-review-num">#{detail.number}</span>
              </div>
              <MarkdownField
                value={edit.body}
                onChange={(body) => setEdit({ ...edit, body })}
                minHeight={160}
                placeholder="Describe this pull request…"
                onSubmit={() => { void saveEdit(); }}
                onCancel={() => setEdit(null)}
              />
              <div className="pr-review-edit-actions">
                <span className="pr-edit-hint">⌘↵ to save</span>
                <button className="settings-ghost-btn" onClick={() => setEdit(null)}>Cancel</button>
                <button
                  className="settings-ghost-btn"
                  disabled={savingEdit || !edit.title.trim()}
                  title={edit.title.trim() ? "Save the title and description to GitHub" : "A pull request needs a title"}
                  onClick={() => { void saveEdit(); }}
                >
                  {savingEdit ? "Saving…" : "Save"}
                </button>
              </div>
            </div>
          ) : (
            <>
              <div className="pr-review-titlerow">
                <span className={`pr-state pr-state-${detail.state.toLowerCase()}`}>{badge}</span>
                <span className="pr-review-title">
                  {detail.title} <span className="pr-review-num">#{detail.number}</span>
                </span>
                <div className="pr-review-actions">
                  <button
                    className="settings-ghost-btn"
                    title="Edit the title and description here, in markdown, instead of in a browser."
                    onClick={() => setEdit({ title: detail.title, body: detail.body })}
                  >
                    Edit
                  </button>
                  <button
                    className="settings-ghost-btn"
                    title="Have an agent review this PR, then work with it to fix what it finds."
                    onClick={() => setShowAgentReview(true)}
                  >
                    Review with agent
                  </button>
                  {detail.state === "OPEN" && (
                    <button
                      className="settings-ghost-btn pr-merge-btn"
                      disabled={!canMerge}
                      title={mergeHint}
                      onClick={() => setShowMerge(true)}
                    >
                      Merge
                    </button>
                  )}
                  <button
                    className="settings-ghost-btn"
                    title="Open this pull request on github.com."
                    onClick={() => openUrl(detail.url).catch((e) => toastError(e, "Couldn't open the PR"))}
                  >
                    Open ↗
                  </button>
                </div>
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
            </>
          )}
        </div>

        <div className="pr-review-files">
          {files.length === 0 && <div className="pr-review-empty">No file changes in this PR.</div>}
          {files.map((f) => (
            <PrDiffFile
              key={f.path}
              file={f}
              threads={threads.filter((t) => t.path === f.path)}
              drafts={drafts.filter((d) => d.path === f.path)}
              viewer={viewer}
              onAddDraft={(d) => setDrafts((prev) => [...prev, { ...d, id: uid() }])}
              onRemoveDraft={(id) => setDrafts((prev) => prev.filter((d) => d.id !== id))}
              onReply={onReply}
              onEditComment={onEditComment}
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
            {isOwnPr
              ? "Your PR: comment only"
              : drafts.length > 0
                ? `${drafts.length} pending comment${drafts.length === 1 ? "" : "s"}`
                : "No pending comments"}
          </span>
          <div className="pr-review-verdict-btns">
            <button
              className="settings-ghost-btn"
              disabled={submitting || !canVerdict}
              title={
                canVerdict
                  ? "Submit feedback without a verdict. Leaves your comments but doesn't approve or block the merge."
                  : "Add a summary or a comment first"
              }
              onClick={() => submit("COMMENT")}
            >
              Comment
            </button>
            <button
              className="settings-ghost-btn"
              disabled={submitting || !canVerdict || isOwnPr}
              title={
                isOwnPr
                  ? ownPrHint
                  : canVerdict
                    ? "Block the merge until addressed. Marks the PR “Changes requested”; you'll need to re-review to clear it."
                    : "Add a summary or a comment first"
              }
              onClick={() => submit("REQUEST_CHANGES")}
            >
              Request changes
            </button>
            <button
              className="settings-ghost-btn pr-approve"
              disabled={submitting || isOwnPr}
              title={isOwnPr ? ownPrHint : "Sign off on the PR. Marks it “Approved” and counts toward required approvals."}
              onClick={() => submit("APPROVE")}
            >
              {submitting ? "Submitting…" : "Approve"}
            </button>
          </div>
        </div>
      </div>

      {showAgentReview && (
        <AgentReviewDialog
          projectId={projectId}
          number={number}
          title={detail.title}
          onStarted={() => setShowAgentReview(false)}
          onCancel={() => setShowAgentReview(false)}
        />
      )}

      {showMerge && (
        <MergePrDialog
          projectId={projectId}
          number={number}
          title={detail.title}
          onMerged={(result) => {
            setShowMerge(false);
            toastSuccess(`Merged #${number}`);
            // Branch cleanup is best effort; the merge already landed, so this
            // is a note rather than an error.
            if (result.warning) toastInfo(result.warning);
            onMerged?.();
            load();
          }}
          onCancel={() => setShowMerge(false)}
        />
      )}
    </div>
  );
}
