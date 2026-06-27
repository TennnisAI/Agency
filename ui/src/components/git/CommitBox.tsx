import { useState } from "react";

export default function CommitBox({
  branch, hasUpstream, hasRemote, ahead, behind, onCommit, onCommitPush, onAmend, onSync, onPublish, onPublishRemote,
}: {
  branch: string;
  hasUpstream: boolean;
  hasRemote: boolean;
  ahead: number;
  behind: number;
  onCommit: (m: string) => void;
  onCommitPush: (m: string) => void;
  onAmend: (m: string) => void;
  onSync: () => void;
  onPublish: () => void;
  onPublishRemote: (url: string) => void;
}) {
  const [message, setMessage] = useState("");
  const [menu, setMenu] = useState(false);
  const [addingRemote, setAddingRemote] = useState(false);
  const [remoteUrl, setRemoteUrl] = useState("");
  const send = (fn: (m: string) => void) => { fn(message); setMessage(""); setMenu(false); };
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
        value={message} onChange={(e) => setMessage(e.target.value)} />
      <div className="git-commit-bar">
        <div className="git-split">
          <button className="git-primary" onClick={() => send(onCommit)}>✓ Commit</button>
          <button className="git-primary git-caret" onClick={() => setMenu((o) => !o)}>▾</button>
          {menu && (
            <div className="git-menu" onMouseLeave={() => setMenu(false)}>
              <button onClick={() => send(onCommitPush)}>Commit &amp; Push</button>
              <button onClick={() => send(onAmend)}>Commit (Amend)</button>
            </div>
          )}
        </div>
        {hasUpstream
          ? (ahead > 0 || behind > 0) &&
            <button className="git-secondary" onClick={onSync}>⟳ Sync {behind ? `↓${behind}` : ""} {ahead ? `↑${ahead}` : ""}</button>
          // No upstream: only offer Publish when there are commits to push.
          : ahead > 0 && (hasRemote
            ? <button className="git-secondary" onClick={onPublish}>☁ Publish Branch ↑{ahead}</button>
            : <button className="git-secondary" onClick={() => setAddingRemote((o) => !o)}
                title="No 'origin' remote configured — add one to publish">☁ Add Remote &amp; Publish…</button>)}
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
