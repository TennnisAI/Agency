import { FileChange } from "../../api";
import { decorateIn, type GitGroup } from "./status";
import { fileIcon } from "../../lib/fileIcon";
import { FileIcon } from "../fileIcons";

export default function FileRow({
  change, group, selected, menuOpen = false, onSelect, onPrimary, onDiscard, onContextMenu,
}: {
  change: FileChange;
  group: GitGroup;
  selected: boolean;
  // This row's context menu is open — highlight it, since right-clicking
  // doesn't move the selection and the menu acts on this row, not the selected one.
  menuOpen?: boolean;
  onSelect: () => void;
  onPrimary: () => void;
  onDiscard?: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
}) {
  const dec = decorateIn(change, group);
  // A folder git reports as a single entry ends in "/" (see
  // expand_untracked_dirs in crates/agency-core/src/git.rs for the two that
  // survive). Splitting that on the last slash left the name empty and put the
  // folder's own name in the dimmed parent slot, so the row read as blank.
  const isFolder = change.path.endsWith("/");
  const shown = isFolder ? change.path.slice(0, -1) : change.path;
  const slash = shown.lastIndexOf("/");
  const dir = slash >= 0 ? shown.slice(0, slash) : "";
  const name = (slash >= 0 ? shown.slice(slash + 1) : shown) + (isFolder ? "/" : "");
  // fileIcon() hands back a "var(--token)" string, so pass it through as-is:
  // wrapping it in var() again made var(var(--blue)), which is invalid CSS and
  // left every icon inheriting the row colour.
  const icon = isFolder ? { kind: "folder" as const, color: "var(--blue)" } : fileIcon(name);
  const primaryLabel = group === "index" ? "Unstage" : "Stage";
  const primaryGlyph = group === "index" ? "−" : "+";
  const deleted = dec.letter === "D";
  return (
    <div className={`git-row ${selected ? "sel" : ""} ${menuOpen ? "ctx" : ""}`}
      onClick={onSelect} onContextMenu={onContextMenu} title={change.path}>
      <span className="git-fileicon" style={{ color: icon.color }}>
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
