import { useCallback, useEffect, useState } from "react";
import { ReviewComment, listReviewComments, deleteReviewComment, sendReviewComments } from "../../api";

export default function ReviewComments({ taskId }: { taskId: string }) {
  const [items, setItems] = useState<ReviewComment[]>([]);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try {
      setItems(await listReviewComments(taskId));
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId]);

  useEffect(() => { load(); }, [load]);

  if (items.length === 0) return null;
  const unsent = items.filter((c) => !c.sent);

  return (
    <div className="review-comments">
      <div className="review-head">
        <span>Review comments</span>
        <span className="spacer" style={{ flex: 1 }} />
        {unsent.length > 0 && (
          <button
            className="git-iconbtn"
            onClick={async () => {
              try { await sendReviewComments(taskId); await load(); }
              catch (e) { setError(String(e)); }
            }}
          >Send {unsent.length} to agent</button>
        )}
      </div>
      {error && <div className="git-error">{error}</div>}
      {items.map((c) => (
        <div key={c.id} className={`review-row ${c.sent ? "sent" : ""}`}>
          <code className="review-loc">
            {c.path}:{c.lineStart}{c.lineEnd !== c.lineStart ? `-${c.lineEnd}` : ""}
          </code>
          <span className="review-body">{c.body}</span>
          <button
            className="icon-btn"
            title="Delete comment"
            onClick={async () => { await deleteReviewComment(c.id); await load(); }}
          >✕</button>
        </div>
      ))}
    </div>
  );
}
