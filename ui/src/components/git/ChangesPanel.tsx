import { useMemo, useState } from "react";
import {
  FileChange, BranchInfo, StashEntry, gitStage, gitUnstage, gitStageAll, gitUnstageAll,
  gitDiscard, gitDiscardAll, gitCommit, gitCommitAmend, gitPush, gitSetRemote,
  gitStashApply, gitStashDrop, gitStashPop,
} from "../../api";
import { partition } from "./status";
import ResourceGroup from "./ResourceGroup";
import CommitBox from "./CommitBox";
import ConfirmDialog from "../ConfirmDialog";
import StashGroup from "./StashGroup";

export default function ChangesPanel({
  taskId, changes, branch, stashes, restoreMessage, onAct, busy = false, selectedPath, onSelectFile,
}: {
  taskId: string;
  changes: FileChange[];
  branch: BranchInfo | null;
  stashes: StashEntry[];
  restoreMessage?: { text: string; nonce: number } | null;
  onAct: (fn: () => Promise<unknown>, label?: string) => Promise<boolean>;
  busy?: boolean;
  selectedPath: string | null;
  onSelectFile: (path: string, group: "index" | "workingTree" | "merge" | "untracked") => void;
}) {
  const g = useMemo(() => partition(changes), [changes]);
  // Discard/drop are destructive (git restore / clean -f / stash drop); confirm first.
  const [pending, setPending] = useState<{ title?: string; label?: string; body: string; run: () => Promise<unknown> } | null>(null);
  const confirmDiscard = (body: string, run: () => Promise<unknown>) => setPending({ body, run });

  return (
    <div className="git-changes">
      <CommitBox
        taskId={taskId}
        branch={branch?.branch ?? "?"}
        hasUpstream={!!branch?.upstream}
        hasRemote={!!branch?.hasRemote}
        ahead={branch?.ahead ?? 0}
        behind={branch?.behind ?? 0}
        busy={busy}
        restoreMessage={restoreMessage}
        onCommit={(m) => onAct(() => gitCommit(taskId, m), "Committed")}
        onCommitAll={(m) => onAct(async () => { await gitStageAll(taskId); await gitCommit(taskId, m); }, "Committed all changes")}
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
      <StashGroup stashes={stashes}
        onApply={(s) => onAct(() => gitStashApply(taskId, s.index), "Stash applied")}
        onPop={(s) => onAct(() => gitStashPop(taskId, s.index), "Stash popped")}
        onDrop={(s) => setPending({
          title: "Drop stash",
          label: "Drop",
          body: `Drop stash "${s.message}"? This cannot be undone.`,
          run: () => gitStashDrop(taskId, s.index),
        })} />

      {pending && (
        <ConfirmDialog
          title={pending.title ?? "Discard changes"}
          body={pending.body}
          confirmLabel={pending.label ?? "Discard"}
          danger
          onConfirm={() => { onAct(pending.run); setPending(null); }}
          onCancel={() => setPending(null)}
        />
      )}
    </div>
  );
}
