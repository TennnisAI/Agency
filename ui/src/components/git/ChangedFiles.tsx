import { useMemo, useState } from "react";
import { CommitFile, fileRootOf } from "../../api";
import { decorate } from "./status";
import { fileIcon } from "../../lib/fileIcon";
import { revealLabel, reveal, copyAbsPath, copyRelPath } from "../../lib/fileActions";
import { FileIcon } from "../fileIcons";
import Menu, { MenuEntry } from "./Menu";

/** The file list beside a read-only diff: a commit's files, or a checkpoint's. */
export default function ChangedFiles({ taskId, files, selected, onSelect, onRevealInFiles }: {
  taskId: string;
  files: CommitFile[];
  selected: string | null;
  onSelect: (path: string) => void;
  // Show a file in the Files tab, as its working-tree copy stands now. Absent
  // where there is no Files tab, and the menu entry goes with it.
  onRevealInFiles?: (path: string) => void;
}) {
  const root = useMemo(() => fileRootOf(taskId), [taskId]);
  const [menu, setMenu] = useState<{ x: number; y: number; file: CommitFile } | null>(null);

  // Only the path actions: the changes list's stage and discard entries act on
  // the working tree, and these rows are history, which there is nothing to
  // stage from. The reveals resolve the file as it stands in the worktree now,
  // so they follow the diff header's rule and skip a file this change deleted.
  const menuItems = (f: CommitFile): MenuEntry[] => {
    const gone = decorate(f.status, " ").letter === "D";
    const slash = f.path.lastIndexOf("/");
    return [
      { kind: "header", label: slash >= 0 ? f.path.slice(slash + 1) : f.path },
      { label: "Open Changes", onClick: () => onSelect(f.path) },
      { kind: "separator" },
      ...(onRevealInFiles
        ? [{ label: "Reveal in Files", disabled: gone, onClick: () => onRevealInFiles(f.path) } as MenuEntry]
        : []),
      { label: revealLabel, disabled: gone, onClick: () => reveal(root, f.path) },
      { label: "Copy Path", disabled: gone, onClick: () => copyAbsPath(root, f.path) },
      { label: "Copy Relative Path", onClick: () => copyRelPath(f.path) },
    ];
  };

  return (
    <div className="git-commitdetail-files">
      {files.map((f) => {
        const dec = decorate(f.status, " ");
        const slash = f.path.lastIndexOf("/");
        const dir = slash >= 0 ? f.path.slice(0, slash) : "";
        const name = slash >= 0 ? f.path.slice(slash + 1) : f.path;
        const icon = fileIcon(name);
        // Right-clicking doesn't move the selection, so mark the row the menu
        // acts on, as the changes list does.
        const menuOpen = menu?.file.path === f.path;
        return (
          <div key={f.path} className={`git-row ${selected === f.path ? "sel" : ""} ${menuOpen ? "ctx" : ""}`}
            onClick={() => onSelect(f.path)}
            onContextMenu={(e) => { e.preventDefault(); setMenu({ x: e.clientX, y: e.clientY, file: f }); }}
            title={f.path}>
            <span className="git-fileicon" style={{ color: icon.color }}>
              <FileIcon kind={icon.kind} size={13} />
            </span>
            <span className="git-name">
              <span className={`git-basename ${dec.letter === "D" ? "deleted" : ""}`}
                style={{ color: `var(${dec.varName})` }}>{name}</span>
              {dir && <span className="git-dir">{dir}</span>}
            </span>
            <span className="git-letter" style={{ color: `var(${dec.varName})` }}>{dec.letter}</span>
          </div>
        );
      })}
      {menu && (
        <Menu x={menu.x} y={menu.y} items={menuItems(menu.file)} onClose={() => setMenu(null)} />
      )}
    </div>
  );
}
