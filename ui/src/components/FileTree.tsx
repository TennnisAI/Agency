import { useEffect, useState } from "react";
import { DirEntry, FileRoot, listDir } from "../api";
import { joinPath } from "../lib/filePath";
import { fileIcon } from "../lib/fileIcon";
import { FileIcon } from "./fileIcons";

function TreeNode({
  root, path, name, isDir, depth, selected, onSelect,
}: {
  root: FileRoot;
  path: string;
  name: string;
  isDir: boolean;
  depth: number;
  selected: string | null;
  onSelect: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [children, setChildren] = useState<DirEntry[] | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!isDir || !open || children) return;
    listDir(root, path).then(setChildren).catch((e) => setError(String(e)));
  }, [isDir, open, children, root, path]);

  const pad = { paddingLeft: 8 + depth * 12 };

  if (!isDir) {
    const { kind, color } = fileIcon(name);
    return (
      <div
        className={`tree-row file ${selected === path ? "on" : ""}`}
        style={pad}
        onClick={() => onSelect(path)}
      >
        <span className="tree-icon file-icon" style={{ color }}>
          <FileIcon kind={kind} />
        </span>{" "}
        {name}
      </div>
    );
  }

  return (
    <>
      <div className="tree-row dir" style={pad} onClick={() => setOpen((o) => !o)}>
        <span className="tree-icon">{open ? "▾" : "▸"}</span> {name}
      </div>
      {open && error && <div className="tree-row error" style={pad}>{error}</div>}
      {open && children?.map((c) => (
        <TreeNode
          key={c.name}
          root={root}
          path={joinPath(path, c.name)}
          name={c.name}
          isDir={c.isDir}
          depth={depth + 1}
          selected={selected}
          onSelect={onSelect}
        />
      ))}
    </>
  );
}

export default function FileTree({
  root, selected, onSelect,
}: {
  root: FileRoot;
  selected: string | null;
  onSelect: (path: string) => void;
}) {
  const [entries, setEntries] = useState<DirEntry[] | null>(null);
  const [error, setError] = useState("");

  // Re-root whenever the FileRoot changes (focused agent ↔ project main).
  const rootKey = `${root.kind}:${root.id}`;
  useEffect(() => {
    setEntries(null);
    setError("");
    listDir(root, "").then(setEntries).catch((e) => setError(String(e)));
    // Re-fetch only when the root identity (kind+id) changes, not on every new root object reference.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rootKey]);

  if (error) return <div className="tree-row error">{error}</div>;
  if (!entries) return <div className="tree-row">loading…</div>;

  return (
    <>
      {entries.map((c) => (
        <TreeNode
          key={c.name}
          root={root}
          path={c.name}
          name={c.name}
          isDir={c.isDir}
          depth={0}
          selected={selected}
          onSelect={onSelect}
        />
      ))}
    </>
  );
}
