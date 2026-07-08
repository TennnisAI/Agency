import { BranchInfo } from "../../api";

export default function BranchBar({ info, busy = false, onSync, onFetch, onPull, onRefresh }: {
  info: BranchInfo | null;
  busy?: boolean;
  onSync: () => void;
  onFetch: () => void;
  onPull: () => void;
  onRefresh: () => void;
}) {
  if (!info) return null;
  return (
    <div className="git-branchbar">
      <span className="git-branch">⎇ {info.branch}</span>
      {(info.ahead > 0 || info.behind > 0) && (
        <span className="git-aheadbehind">
          {info.behind > 0 && <span title="behind">↓{info.behind}</span>}
          {info.ahead > 0 && <span title="ahead">↑{info.ahead}</span>}
        </span>
      )}
      {busy && <span className="spinner" aria-label="working" />}
      <span className="spacer" style={{ flex: 1 }} />
      {/* Fetch updates remote-tracking refs so ↓behind reflects origin. */}
      {info.hasRemote && (
        <button className="git-iconbtn" title="Fetch from origin" aria-label="Fetch from origin"
          onClick={onFetch} disabled={busy}>{"↧"}</button>
      )}
      {/* Pull (fast-forward) only when there is an upstream with commits behind. */}
      {info.upstream && info.behind > 0 && (
        <button className="git-iconbtn" title={`Pull ${info.behind} commit${info.behind === 1 ? "" : "s"} from origin`}
          aria-label="Pull from origin" onClick={onPull} disabled={busy}>↓{info.behind}</button>
      )}
      {info.upstream && (
        <button className="git-iconbtn" title="Push" aria-label="Push to origin" onClick={onSync} disabled={busy}>⟳</button>
      )}
      <button className="git-iconbtn" title="Refresh" aria-label="Refresh" onClick={onRefresh}>⟲</button>
    </div>
  );
}
