import { useEffect, useState } from "react";

// In-progress commit messages outlive the commit box being unmounted (switching
// tabs, toggling the review pane). Keyed by repo so each keeps its own draft for
// the session; cleared once the commit lands.
const draftStore = new Map<string, string>();

export default function CommitBox({
  taskId, branch, hasUpstream, hasRemote, ahead, behind, busy = false, restoreMessage,
  onCommit, onCommitAll, onCommitPush, onAmend, onSync, onPublish, onPublishRemote,
}: {
  taskId: string;
  branch: string;
  hasUpstream: boolean;
  hasRemote: boolean;
  ahead: number;
  behind: number;
  busy?: boolean;
  /** Set after Undo Last Commit: the undone commit's message, restored into the input. */
  restoreMessage?: { text: string; nonce: number } | null;
  // Commit callbacks resolve true on success so the message is only cleared
  // once the commit actually landed — a failed commit keeps the user's text.
  onCommit: (m: string) => Promise<boolean>;
  onCommitAll: (m: string) => Promise<boolean>;
  onCommitPush: (m: string) => Promise<boolean>;
  onAmend: (m: string) => Promise<boolean>;
  onSync: () => void;
  onPublish: () => void;
  onPublishRemote: (url: string) => void;
}) {
  const [message, setMessageState] = useState(() => draftStore.get(taskId) ?? "");
  const [menu, setMenu] = useState(false);
  const [addingRemote, setAddingRemote] = useState(false);
  const [remoteUrl, setRemoteUrl] = useState("");
  // Keep the per-repo draft store in sync so the message survives unmounts.
  const setMessage = (m: string) => {
    setMessageState(m);
    if (m) draftStore.set(taskId, m);
    else draftStore.delete(taskId);
  };
  // Switching repos without remounting: load the new repo's draft.
  useEffect(() => { setMessageState(draftStore.get(taskId) ?? ""); }, [taskId]);
  useEffect(() => {
    if (restoreMessage) setMessage(restoreMessage.text);
  }, [restoreMessage]);
  const canCommit = message.trim().length > 0;
  const send = async (fn: (m: string) => Promise<boolean>) => {
    if (!canCommit) return; // empty/whitespace message: nothing to commit with
    setMenu(false);
    if (await fn(message)) setMessage("");
  };
  const submitRemote = () => {
    const url = remoteUrl.trim();
    if (!url) return;
    onPublishRemote(url);
    setRemoteUrl("");
    setAddingRemote(false);
  };
  return (
    <div className="git-commit">
      <textarea className="git-commit-input" placeholder={`Message (commit on ${branch})`}
        value={message} onChange={(e) => setMessage(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); send(onCommit); } }} />
      <div className="git-commit-bar">
        <div className="git-split">
          <button className="git-primary" onClick={() => send(onCommit)} disabled={busy || !canCommit}
            title="Commit staged changes (⌘Enter)">
            {busy ? <span className="spinner" aria-label="working" /> : "✓"} Commit
          </button>
          <button className="git-primary git-caret" onClick={() => setMenu((o) => !o)} disabled={busy || !canCommit}>▾</button>
          {menu && (
            <div className="git-menu" onMouseLeave={() => setMenu(false)}>
              <button onClick={() => send(onCommitAll)}>Commit All</button>
              <button onClick={() => send(onCommitPush)}>Commit &amp; Push</button>
              <button onClick={() => send(onAmend)}>Commit (Amend)</button>
            </div>
          )}
        </div>
        {hasUpstream
          ? (ahead > 0 || behind > 0) &&
            <button className="git-secondary" onClick={onSync} disabled={busy}>⟳ Sync {behind ? `↓${behind}` : ""} {ahead ? `↑${ahead}` : ""}</button>
          // No upstream: only offer Publish when there are commits to push.
          : ahead > 0 && (hasRemote
            ? <button className="git-secondary" onClick={onPublish} disabled={busy}>{"☁︎"} Publish Branch ↑{ahead}</button>
            : <button className="git-secondary" onClick={() => setAddingRemote((o) => !o)} disabled={busy}
                title="No 'origin' remote configured. Add one to publish.">{"☁︎"} Add Remote &amp; Publish…</button>)}
      </div>
      {addingRemote && !hasRemote && (
        <div className="git-remote-row">
          <input className="git-remote-input" placeholder="origin URL (e.g. git@github.com:user/repo.git)"
            value={remoteUrl} autoFocus
            onChange={(e) => setRemoteUrl(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") submitRemote(); if (e.key === "Escape") setAddingRemote(false); }} />
          <button className="git-secondary" disabled={!remoteUrl.trim()} onClick={submitRemote}>Publish ↑{ahead}</button>
          <button className="git-secondary" onClick={() => setAddingRemote(false)}>Cancel</button>
        </div>
      )}
    </div>
  );
}
