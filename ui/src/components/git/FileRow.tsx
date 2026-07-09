import { FileChange } from "../../api";
import { decorate, type GitGroup } from "./status";
import { fileIcon } from "../../lib/fileIcon";
import { FileIcon } from "../fileIcons";

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
  const dir = slash >= 0 ? change.path.slice(0, slash) : "";
  const name = slash >= 0 ? change.path.slice(slash + 1) : change.path;
  const icon = fileIcon(name);
  const primaryLabel = group === "index" ? "Unstage" : "Stage";
  const primaryGlyph = group === "index" ? "−" : "+";
  const deleted = dec.letter === "D";
  return (
    <div className={`git-row ${selected ? "sel" : ""}`} onClick={onSelect} title={change.path}>
      <span className="git-fileicon" style={{ color: `var(${icon.color})` }}>
        <FileIcon kind={icon.kind} size={13} />
      </span>
      <span className="git-name">
        <span className={`git-basename ${deleted ? "deleted" : ""}`}
          style={{ color: `var(${dec.varName})` }}>{name}</span>
        {dir && <span className="git-dir">{dir}</span>}
      </span>
      <span className="git-row-actions">
        {onDiscard && (
          <button className="git-iconbtn" title="Discard Changes"
            onClick={(e) => { e.stopPropagation(); onDiscard(); }}>↩</button>
        )}
        <button className="git-iconbtn" title={primaryLabel}
          onClick={(e) => { e.stopPropagation(); onPrimary(); }}>{primaryGlyph}</button>
      </span>
      <span className="git-letter" style={{ color: `var(${dec.varName})` }} title={primaryLabel === "Unstage" ? "Staged" : undefined}>
        {dec.letter}
      </span>
    </div>
  );
}
