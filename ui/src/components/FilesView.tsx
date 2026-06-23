import { useState } from "react";
import { FileRoot } from "../api";
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";

export default function FilesView({ root, projectName }: { root: FileRoot | null; projectName: string }) {
  const treePane = usePaneWidth("files-tree", 280, 180, 560);
  const [selected, _setSelected] = useState<string | null>(null);

  if (!root) {
    return <div className="board empty">Open a project to browse its files.</div>;
  }

  const rootLabel = root.kind === "run" ? "Agent worktree" : `${projectName} · main`;

  return (
    <div className="files-view">
      <div className="files-tree" style={{ width: treePane.width }}>
        <div className="files-root-label">{rootLabel}</div>
        {/* FileTree added in Task 6 */}
        <div className="files-tree-body">tree…</div>
      </div>
      <Resizer size={treePane.width} min={180} max={560} onChange={treePane.setWidth} />
      <div className="files-editor">
        {/* FileEditor added in Task 7 */}
        {selected ? <div>editor: {selected}</div> : <div className="diff-empty">Select a file to view.</div>}
      </div>
    </div>
  );
}
