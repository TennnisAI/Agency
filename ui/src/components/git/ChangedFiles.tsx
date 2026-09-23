import { CommitFile } from "../../api";
import { decorate } from "./status";
import { fileIcon } from "../../lib/fileIcon";
import { FileIcon } from "../fileIcons";

/** The file list beside a read-only diff: a commit's files, or a checkpoint's. */
export default function ChangedFiles({ files, selected, onSelect }: {
  files: CommitFile[];
  selected: string | null;
  onSelect: (path: string) => void;
}) {
  return (
    <div className="git-commitdetail-files">
      {files.map((f) => {
        const dec = decorate(f.status, " ");
        const slash = f.path.lastIndexOf("/");
        const dir = slash >= 0 ? f.path.slice(0, slash) : "";
        const name = slash >= 0 ? f.path.slice(slash + 1) : f.path;
        const icon = fileIcon(name);
        return (
          <div key={f.path} className={`git-row ${selected === f.path ? "sel" : ""}`} onClick={() => onSelect(f.path)} title={f.path}>
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
    </div>
  );
}
