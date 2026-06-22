import { FileChange } from "../../api";
import { decorate, type GitGroup } from "./status";

export default function FileRow({
  change, group, selected, onSelect, onPrimary, onDiscard,
}: {
  change: FileChange;
  group: GitGroup;
  selected: boolean;
  onSelect: () => void;
  onPrimary: () => void;
  onDiscard?: () => void;
}) {
  const dec = decorate(group === "index" ? change.index : " ", group === "index" ? " " : change.worktree);
  const slash = change.path.lastIndexOf("/");
  const dir = slash >= 0 ? change.path.slice(0, slash + 1) : "";
  const name = slash >= 0 ? change.path.slice(slash + 1) : change.path;
  const primaryLabel = group === "index" ? "Unstage" : "Stage";
  const primaryGlyph = group === "index" ? "−" : "+";
  return (
    <div className={`git-row ${selected ? "sel" : ""}`} onClick={onSelect} title={change.path}>
      <span className="git-letter" style={{ color: `var(${dec.varName})` }}>{dec.letter}</span>
      <span className="git-name"><span className="git-dir">{dir}</span>{name}</span>
      <span className="git-row-actions">
        {onDiscard && (
          <button className="git-iconbtn" title="Discard Changes"
            onClick={(e) => { e.stopPropagation(); onDiscard(); }}>↩</button>
        )}
        <button className="git-iconbtn" title={primaryLabel}
          onClick={(e) => { e.stopPropagation(); onPrimary(); }}>{primaryGlyph}</button>
      </span>
    </div>
  );
}
