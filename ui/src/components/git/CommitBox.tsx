import { useState } from "react";

export default function CommitBox({
  branch, hasUpstream, ahead, behind, onCommit, onCommitPush, onAmend, onSync, onPublish,
}: {
  branch: string;
  hasUpstream: boolean;
  ahead: number;
  behind: number;
  onCommit: (m: string) => void;
  onCommitPush: (m: string) => void;
  onAmend: (m: string) => void;
  onSync: () => void;
  onPublish: () => void;
}) {
  const [message, setMessage] = useState("");
  const [menu, setMenu] = useState(false);
  const send = (fn: (m: string) => void) => { fn(message); setMessage(""); setMenu(false); };
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
        {!hasUpstream
          ? <button className="git-secondary" onClick={onPublish}>☁ Publish Branch</button>
          : (ahead > 0 || behind > 0) &&
            <button className="git-secondary" onClick={onSync}>⟳ Sync {behind ? `↓${behind}` : ""} {ahead ? `↑${ahead}` : ""}</button>}
      </div>
    </div>
  );
}
