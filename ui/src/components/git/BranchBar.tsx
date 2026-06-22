import { BranchInfo } from "../../api";

export default function BranchBar({ info, onSync, onRefresh }: {
  info: BranchInfo | null;
  onSync: () => void;
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
      <span className="spacer" style={{ flex: 1 }} />
      {info.upstream && <button className="git-iconbtn" title="Sync" onClick={onSync}>⟳</button>}
      <button className="git-iconbtn" title="Refresh" onClick={onRefresh}>⟲</button>
    </div>
  );
}
