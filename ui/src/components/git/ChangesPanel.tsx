import { useState } from "react";
import {
  FileChange, BranchInfo, gitStage, gitUnstage, gitStageAll, gitUnstageAll,
  gitDiscard, gitDiscardAll, gitCommit, gitCommitAmend, gitPush, gitSetRemote,
} from "../../api";
import { partition } from "./status";
import ResourceGroup from "./ResourceGroup";
import CommitBox from "./CommitBox";
import ConfirmDialog from "../ConfirmDialog";

export default function ChangesPanel({
  taskId, changes, branch, onAct, busy = false, selectedPath, onSelectFile,
}: {
  taskId: string;
  changes: FileChange[];
  branch: BranchInfo | null;
  onAct: (fn: () => Promise<unknown>, label?: string) => Promise<boolean>;
  busy?: boolean;
  selectedPath: string | null;
  onSelectFile: (path: string, group: "index" | "workingTree" | "merge" | "untracked") => void;
}) {
  const g = partition(changes);
  // Discard is destructive (git restore / clean -f); confirm before running.
  const [pending, setPending] = useState<{ body: string; run: () => Promise<unknown> } | null>(null);
  const confirmDiscard = (body: string, run: () => Promise<unknown>) => setPending({ body, run });

  return (
    <div className="git-changes">
      <CommitBox
        branch={branch?.branch ?? "?"}
        hasUpstream={!!branch?.upstream}
        hasRemote={!!branch?.hasRemote}
        ahead={branch?.ahead ?? 0}
        behind={branch?.behind ?? 0}
        busy={busy}
        onCommit={(m) => onAct(() => gitCommit(taskId, m), "Committed")}
        onCommitPush={(m) => onAct(async () => { await gitCommit(taskId, m); await gitPush(taskId); }, "Committed & pushed")}
        onAmend={(m) => onAct(() => gitCommitAmend(taskId, m), "Amended")}
        onSync={() => onAct(() => gitPush(taskId), "Synced")}
        onPublish={() => onAct(() => gitPush(taskId), "Branch published")}
        onPublishRemote={(url) => onAct(async () => { await gitSetRemote(taskId, url); await gitPush(taskId); }, "Branch published")}
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
        onFileDiscard={(c) => confirmDiscard(
          `Discard changes to ${c.path}? This cannot be undone.`,
          () => gitDiscard(taskId, c.path, false))}
        onStageAll={() => onAct(() => gitStageAll(taskId))}
        onDiscardAll={() => confirmDiscard(
          "Discard all changes to tracked files? This cannot be undone.",
          () => gitDiscardAll(taskId))} />
      <ResourceGroup id="untracked" label="Untracked Changes" changes={g.untracked}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "untracked")}
        onFilePrimary={(c) => onAct(() => gitStage(taskId, c.path))}
        onFileDiscard={(c) => confirmDiscard(
          `Delete untracked file ${c.path}? This cannot be undone.`,
          () => gitDiscard(taskId, c.path, true))}
        onStageAll={() => onAct(() => gitStageAll(taskId))} />

      {pending && (
        <ConfirmDialog
          title="Discard changes"
          body={pending.body}
          confirmLabel="Discard"
          danger
          onConfirm={() => { onAct(pending.run); setPending(null); }}
          onCancel={() => setPending(null)}
        />
      )}
    </div>
  );
}
