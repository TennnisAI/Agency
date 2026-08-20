import { useState } from "react";
import { ReviewThread } from "../../api";
import { toastError } from "../../lib/toast";
import Markdown from "../Markdown";
import MarkdownField from "./MarkdownField";

// One review conversation rendered inline under its anchored diff line: the
// comments, a reply box, and a resolve/unresolve toggle. Replies anchor to the
// thread's first comment (GitHub groups a thread under its root comment id).
//
// A comment you wrote is editable in place. `viewer` is the authenticated gh
// login; it comes back null when that lookup failed, in which case no comment
// offers an Edit button rather than offering one that GitHub would reject.
export default function PrThread({
  thread,
  viewer,
  onReply,
  onEditComment,
  onToggleResolved,
}: {
  thread: ReviewThread;
  viewer: string | null;
  onReply: (inReplyTo: number, body: string) => Promise<void>;
  onEditComment: (commentId: number, body: string) => Promise<void>;
  onToggleResolved: (threadId: string, resolved: boolean) => Promise<void>;
}) {
  const [reply, setReply] = useState("");
  const [replying, setReplying] = useState(false);
  const [busy, setBusy] = useState(false);
  // Which comment is being rewritten (its databaseId), and the text so far.
  const [editing, setEditing] = useState<number | null>(null);
  const [editDraft, setEditDraft] = useState("");
  const root = thread.comments[0];
  const mine = (author: string) => !!viewer && author.toLowerCase() === viewer.toLowerCase();

  async function sendReply() {
    if (!root || !reply.trim()) return;
    setBusy(true);
    try {
      await onReply(root.databaseId, reply.trim());
      setReply("");
      setReplying(false);
    } catch (e) {
      toastError(e, "Couldn't post reply");
    } finally {
      setBusy(false);
    }
  }

  async function saveEdit(commentId: number) {
    const body = editDraft.trim();
    if (!body || busy) return;
    setBusy(true);
    try {
      await onEditComment(commentId, body);
      setEditing(null);
    } catch (e) {
      // The text stays in the box: a rejected save must not eat the rewrite.
      toastError(e, "Couldn't save the comment");
    } finally {
      setBusy(false);
    }
  }

  async function toggle() {
    setBusy(true);
    try {
      await onToggleResolved(thread.id, !thread.isResolved);
    } catch (e) {
      toastError(e, "Couldn't update thread");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className={`pr-thread${thread.isResolved ? " resolved" : ""}`}>
      <div className="pr-thread-head">
        {thread.isResolved && <span className="pr-thread-tag">Resolved</span>}
        {thread.isOutdated && <span className="pr-thread-tag outdated">Outdated</span>}
        <span className="spacer" style={{ flex: 1 }} />
        <button className="git-iconbtn" disabled={busy} onClick={toggle}>
          {thread.isResolved ? "Unresolve" : "Resolve"}
        </button>
      </div>
      {thread.comments.map((c) => (
        <div key={c.id} className="pr-comment">
          <div className="pr-comment-meta">
            <span className="pr-comment-author">{c.author || "unknown"}</span>
            {c.createdAt && <span className="pr-comment-date">{new Date(c.createdAt).toLocaleDateString()}</span>}
            <span className="spacer" style={{ flex: 1 }} />
            {mine(c.author) && editing !== c.databaseId && (
              <button
                className="pr-comment-edit"
                title="Rewrite this comment on GitHub"
                onClick={() => { setEditing(c.databaseId); setEditDraft(c.body); }}
              >
                Edit
              </button>
            )}
          </div>
          {editing === c.databaseId ? (
            <div className="pr-comment-editbox">
              <MarkdownField
                value={editDraft}
                onChange={setEditDraft}
                autoFocus
                minHeight={72}
                onSubmit={() => { void saveEdit(c.databaseId); }}
                onCancel={() => setEditing(null)}
              />
              <div className="pr-reply-actions">
                <span className="pr-edit-hint">⌘↵ to save</span>
                <button className="git-iconbtn" onClick={() => setEditing(null)}>Cancel</button>
                <button
                  className="git-iconbtn"
                  disabled={busy || !editDraft.trim()}
                  onClick={() => { void saveEdit(c.databaseId); }}
                >
                  {busy ? "Saving…" : "Save"}
                </button>
              </div>
            </div>
          ) : (
            <Markdown className="pr-comment-body" text={c.body} />
          )}
        </div>
      ))}
      {replying ? (
        <div className="pr-reply-box">
          <textarea
            className="settings-input"
            placeholder="Reply…"
            value={reply}
            autoFocus
            onChange={(e) => setReply(e.target.value)}
          />
          <div className="pr-reply-actions">
            <button className="git-iconbtn" disabled={busy || !reply.trim()} onClick={sendReply}>
              {busy ? "Posting…" : "Reply"}
            </button>
            <button className="git-iconbtn" onClick={() => { setReplying(false); setReply(""); }}>Cancel</button>
          </div>
        </div>
      ) : (
        <button className="pr-reply-open" onClick={() => setReplying(true)}>Reply…</button>
      )}
    </div>
  );
}
