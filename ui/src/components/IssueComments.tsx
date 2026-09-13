import { useEffect, useRef, useState } from "react";
import { IssueComment } from "../api";
import { fmtStamp } from "../lib/issues";
import { toastError } from "../lib/toast";
import Markdown from "./Markdown";
import ConfirmDialog from "./ConfirmDialog";
import { shortcutLabel } from "../lib/platform";

// The discussion under an issue: the thread, then a box to add to it. Comment
// bodies are markdown, rendered read-only the way PR comments are (the
// description is the only editor in this pane); posting, editing and deleting
// each go straight to the issue file through their own call.
//
// Comments are addressed by `createdAt`, which the backend keeps unique within
// an issue — there is no id to carry around.
export default function IssueComments({
  comments,
  onPost,
  onEdit,
  onDelete,
}: {
  comments: IssueComment[];
  onPost: (body: string) => Promise<void>;
  onEdit: (createdAt: number, body: string) => Promise<void>;
  onDelete: (createdAt: number) => Promise<void>;
}) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  // Which comment is being rewritten, and the text it is being rewritten to.
  const [editing, setEditing] = useState<number | null>(null);
  const [editDraft, setEditDraft] = useState("");
  const [confirm, setConfirm] = useState<IssueComment | null>(null);
  const editRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => { if (editing !== null) editRef.current?.focus(); }, [editing]);

  const post = async () => {
    const text = draft.trim();
    if (!text || busy) return;
    setBusy(true);
    try {
      await onPost(text);
      // Only cleared once the write lands: a failed post must not eat what was
      // typed.
      setDraft("");
    } catch (e) {
      toastError(e, "Couldn't post the comment");
    } finally {
      setBusy(false);
    }
  };

  const saveEdit = async (at: number) => {
    const text = editDraft.trim();
    if (!text || busy) return;
    setBusy(true);
    try {
      await onEdit(at, text);
      setEditing(null);
    } catch (e) {
      toastError(e, "Couldn't save the comment");
    } finally {
      setBusy(false);
    }
  };

  const remove = async (c: IssueComment) => {
    setConfirm(null);
    setBusy(true);
    try {
      await onDelete(c.createdAt);
    } catch (e) {
      toastError(e, "Couldn't delete the comment");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="issue-comments">
      <div className="issue-detail-section-head">
        <h3>Comments</h3>
        {comments.length > 0 && <span className="issue-comment-count">{comments.length}</span>}
      </div>
      {comments.map((c) => (
        <article key={c.createdAt} className="issue-comment">
          <div className="issue-comment-meta">
            <span className="issue-comment-author">{c.author}</span>
            <span className="issue-comment-when">{fmtStamp(c.createdAt)}</span>
            <div className="spacer" />
            {editing !== c.createdAt && (
              <span className="issue-comment-actions">
                <button
                  title="Edit this comment"
                  onClick={() => { setEditing(c.createdAt); setEditDraft(c.body); }}
                >
                  Edit
                </button>
                <button title="Delete this comment" onClick={() => setConfirm(c)}>Delete</button>
              </span>
            )}
          </div>
          {editing === c.createdAt ? (
            <div className="issue-comment-edit">
              <textarea
                ref={editRef}
                className="issue-comment-input"
                value={editDraft}
                onChange={(e) => setEditDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault();
                    void saveEdit(c.createdAt);
                  } else if (e.key === "Escape") {
                    e.stopPropagation();
                    setEditing(null);
                  }
                }}
              />
              <div className="issue-comment-buttons">
                <button className="ghost" onClick={() => setEditing(null)}>Cancel</button>
                <button
                  className="issue-comment-post"
                  disabled={busy || !editDraft.trim()}
                  onClick={() => { void saveEdit(c.createdAt); }}
                >
                  Save
                </button>
              </div>
            </div>
          ) : (
            <Markdown className="issue-comment-body" text={c.body} />
          )}
        </article>
      ))}
      <div className="issue-comment-compose">
        <textarea
          className="issue-comment-input"
          placeholder="Write a comment…"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              void post();
            }
          }}
        />
        {draft.trim() && (
          <div className="issue-comment-buttons">
            <span className="issue-comment-hint">{shortcutLabel("⌘↵")} to post</span>
            <button className="ghost" onClick={() => setDraft("")}>Cancel</button>
            <button className="issue-comment-post" disabled={busy} onClick={() => { void post(); }}>Comment</button>
          </div>
        )}
      </div>
      {confirm && (
        <ConfirmDialog
          title="Delete comment?"
          body={`Delete ${confirm.author}'s comment of ${fmtStamp(confirm.createdAt)}. This cannot be undone.`}
          confirmLabel="Delete"
          danger
          onConfirm={() => { void remove(confirm); }}
          onCancel={() => setConfirm(null)}
        />
      )}
    </div>
  );
}
