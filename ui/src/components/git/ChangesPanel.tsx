import {
  FileChange, BranchInfo, gitStage, gitUnstage, gitStageAll, gitUnstageAll,
  gitDiscard, gitDiscardAll, gitCommit, gitCommitAmend, gitPush,
} from "../../api";
import { partition } from "./status";
import ResourceGroup from "./ResourceGroup";
import CommitBox from "./CommitBox";

export default function ChangesPanel({
  taskId, changes, branch, onAct, selectedPath, onSelectFile,
}: {
  taskId: string;
  changes: FileChange[];
  branch: BranchInfo | null;
  onAct: (fn: () => Promise<unknown>) => void;
  selectedPath: string | null;
  onSelectFile: (path: string, group: "index" | "workingTree" | "merge" | "untracked") => void;
}) {
  const g = partition(changes);
  return (
    <div className="git-changes">
      <CommitBox
        branch={branch?.branch ?? "?"}
        hasUpstream={!!branch?.upstream}
        ahead={branch?.ahead ?? 0}
        behind={branch?.behind ?? 0}
        onCommit={(m) => onAct(() => gitCommit(taskId, m))}
        onCommitPush={(m) => onAct(async () => { await gitCommit(taskId, m); await gitPush(taskId); })}
        onAmend={(m) => onAct(() => gitCommitAmend(taskId, m))}
        onSync={() => onAct(() => gitPush(taskId))}
        onPublish={() => onAct(() => gitPush(taskId))}
      />
      <ResourceGroup id="merge" label="Merge Changes" changes={g.merge}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "merge")}
        onFilePrimary={(c) => onAct(() => gitStage(taskId, c.path))} />
      <ResourceGroup id="index" label="Staged Changes" changes={g.index}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "index")}
        onFilePrimary={(c) => onAct(() => gitUnstage(taskId, c.path))}
        onUnstageAll={() => onAct(() => gitUnstageAll(taskId))} />
      <ResourceGroup id="workingTree" label="Changes" changes={g.workingTree}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "workingTree")}
        onFilePrimary={(c) => onAct(() => gitStage(taskId, c.path))}
        onFileDiscard={(c) => onAct(() => gitDiscard(taskId, c.path, false))}
        onStageAll={() => onAct(() => gitStageAll(taskId))}
        onDiscardAll={() => onAct(() => gitDiscardAll(taskId))} />
      <ResourceGroup id="untracked" label="Untracked Changes" changes={g.untracked}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "untracked")}
        onFilePrimary={(c) => onAct(() => gitStage(taskId, c.path))}
        onFileDiscard={(c) => onAct(() => gitDiscard(taskId, c.path, true))}
        onStageAll={() => onAct(() => gitStageAll(taskId))} />
    </div>
  );
}
