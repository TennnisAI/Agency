import { useEffect, useRef, useState } from "react";
import { useHushed } from "../../lib/hushed";
import ConfirmDialog from "../ConfirmDialog";
import { CloudUploadIcon } from "./gitIcons";
import {
  commitGuard, stagingAll, type CommitAction, type CommitState, type StageableAction,
} from "./commitGuard";
import { setGitOp } from "./ops";

// In-progress commit messages outlive the commit box being unmounted (switching
// tabs, toggling the review pane). Keyed by repo so each keeps its own draft for
// the session; cleared once the commit lands.
const draftStore = new Map<string, string>();

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many);

/** "3 files and 1 folder have", for the "nothing staged" prompt's count. */
const changedSubject = (files: number, folders: number) => {
  const parts: string[] = [];
  if (files) parts.push(`${files} ${plural(files, "file", "files")}`);
  if (folders) parts.push(`${folders} ${plural(folders, "folder", "folders")}`);
  return `${parts.join(" and ")} ${files + folders === 1 ? "has" : "have"}`;
};

export default function CommitBox({
  taskId, branch, readStatus, hasUpstream, hasRemote, ahead, behind, busy = false, restoreMessage,
  onCommit, onCommitAll, onCommitPush, onCommitAllPush, onAmend, onStageAll,
  onSync, onPublish, onPublishRemote,
}: {
  taskId: string;
  branch: string;
  /**
   * The working tree's status, read fresh, to say why a commit can't run before
   * running it. Null when git couldn't be read, which lets the commit through.
   */
  readStatus: () => Promise<CommitState | null>;
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
  onCommitAllPush: (m: string) => Promise<boolean>;
  onAmend: (m: string) => Promise<boolean>;
  /** Stage everything without committing: the "stage, then let me look" way out. */
  onStageAll: () => void;
  onSync: () => void;
  onPublish: () => void;
  onPublishRemote: (url: string) => void;
}) {
  const [message, setMessageState] = useState(() => draftStore.get(taskId) ?? "");
  const [menu, setMenu] = useState(false);
  const [addingRemote, setAddingRemote] = useState(false);
  const [remoteUrl, setRemoteUrl] = useState("");
  // The action the user asked for, held while the "nothing is staged" prompt
  // asks what to do about it.
  const [offer, setOffer] =
    useState<{ action: StageableAction; files: number; folders: number } | null>(null);
  const [dontAsk, setDontAsk] = useHushed("commit-stage-all");
  const [hushNext, setHushNext] = useState(false);
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
  // `busy` only goes up once the op starts, so without this a second press
  // during the status read below would start a second commit behind the first.
  const checking = useRef(false);

  const runners: Record<CommitAction, (m: string) => Promise<boolean>> = {
    commit: onCommit,
    commitAll: onCommitAll,
    commitPush: onCommitPush,
    commitAllPush: onCommitAllPush,
    amend: onAmend,
  };
  const send = async (action: CommitAction) => {
    if (await runners[action](message)) setMessage("");
  };

  // Every commit goes through here so none of them can reach git in a state git
  // will only refuse: an empty index, a clean tree, an unresolved conflict. The
  // reason goes where the raw failure used to (the panel's banner), and the one
  // that has a way out gets the prompt instead (AGE-205).
  const run = async (action: CommitAction) => {
    // No message (empty or whitespace), or a press that landed while the last
    // one was still reading git.
    if (!canCommit || checking.current) return;
    setMenu(false);
    // Read git now rather than guarding on the panel's poll. That poll backs
    // off to 10s once there are thousands of changes and keeps its last good
    // value when a refresh throws, and both make the guard refuse a commit git
    // would have taken: a `git add` in a terminal inside the window read as
    // "nothing is staged", and with the prompt hushed that staged and committed
    // everything the user had deliberately left out.
    checking.current = true;
    let state: CommitState | null;
    try {
      state = await readStatus();
    } finally {
      checking.current = false;
    }
    const guard = commitGuard(action, state);
    if (guard.kind === "ok") { await send(action); return; }
    if (guard.kind === "conflicts") {
      const n = guard.count;
      setGitOp(taskId, {
        error: `Nothing was committed: ${n} ${plural(n, "file", "files")} still ${plural(n, "has", "have")} merge conflicts. Resolve each one, then stage it to mark it resolved.`,
      });
      return;
    }
    if (guard.kind === "empty") {
      setGitOp(taskId, { error: "Nothing to commit: this working tree has no changes." });
      return;
    }
    // Asked once and told not to ask again: do what the prompt would have done.
    if (dontAsk) { await send(stagingAll(guard.action)); return; }
    setHushNext(false);
    setOffer({ action: guard.action, files: guard.files, folders: guard.folders });
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
        onKeyDown={(e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); run("commit"); } }} />
      <div className="git-commit-bar">
        <div className="git-split">
          <button className="git-primary" onClick={() => run("commit")} disabled={busy || !canCommit}
            title="Commit staged changes (⌘Enter)">
            {busy ? <span className="spinner" aria-label="working" /> : "✓"} Commit
          </button>
          <button className="git-primary git-caret" onClick={() => setMenu((o) => !o)} disabled={busy || !canCommit}>▾</button>
          {menu && (
            <div className="git-menu" onMouseLeave={() => setMenu(false)}>
              <button onClick={() => run("commitAll")}>Commit All</button>
              <button onClick={() => run("commitPush")}>Commit &amp; Push</button>
              <button onClick={() => run("amend")}>Commit (Amend)</button>
            </div>
          )}
        </div>
        {hasUpstream
          ? (ahead > 0 || behind > 0) &&
            <button className="git-secondary" onClick={onSync} disabled={busy}>⟳ Sync {behind ? `↓${behind}` : ""} {ahead ? `↑${ahead}` : ""}</button>
          // No upstream: only offer Publish when there are commits to push.
          : ahead > 0 && (hasRemote
            ? <button className="git-secondary git-iconlabel" onClick={onPublish} disabled={busy}
                title={`Push ${ahead} commit${ahead === 1 ? "" : "s"} to a new origin/${branch}`}>
                <CloudUploadIcon />
                {/* "Publish Branch ↑N" did not fit beside the Commit split at the
                    compact panel's width. The branch it publishes is named in the
                    branch bar directly above, and in full in this button's title. */}
                <span>Publish ↑{ahead}</span></button>
            : <button className="git-secondary git-iconlabel" onClick={() => setAddingRemote((o) => !o)} disabled={busy}
                title="No 'origin' remote configured. Add one to publish.">
                <CloudUploadIcon />
                {/* Same width problem as its sibling above: "Add Remote & Publish…"
                    is 180px of button beside a 109px Commit split. The ellipsis says
                    an input follows, and the title says why it is needed. */}
                <span>Add Remote…</span></button>)}
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
      {offer && (
        <ConfirmDialog
          title="Nothing is staged"
          body={`Nothing is staged, so this commit would be empty. ${changedSubject(offer.files, offer.folders)} changes you can include.${offer.folders ? " Staging a folder includes every file inside it." : ""} Stage everything and commit it, or just stage it and look at the diff first.`}
          confirmLabel={offer.action === "commitPush" ? "Stage All, Commit & Push" : "Stage All & Commit"}
          altLabel="Stage All"
          // Stage All is a choice, not a cancellation, so a ticked "Don't ask
          // again" is honoured here too rather than quietly thrown away.
          onAlt={() => { setOffer(null); if (hushNext) setDontAsk(true); onStageAll(); }}
          hushLabel="Don't ask again"
          hushed={hushNext}
          onHush={setHushNext}
          onConfirm={() => {
            setOffer(null);
            if (hushNext) setDontAsk(true);
            void send(stagingAll(offer.action));
          }}
          onCancel={() => setOffer(null)}
        />
      )}
    </div>
  );
}
