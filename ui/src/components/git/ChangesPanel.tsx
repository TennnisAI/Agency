import { useMemo, useState } from "react";
import {
  FileChange, BranchInfo, StashEntry, fileRootOf, gitStage, gitUnstage, gitStageAll, gitUnstageAll,
  gitDiscard, gitDiscardAll, gitCommit, gitCommitAmend, gitPush, gitSetRemote,
  gitStashApply, gitStashDrop, gitStashPop,
} from "../../api";
import { baseName } from "../../lib/filePath";
import { revealLabel, reveal, copyAbsPath, copyRelPath, ignorePath } from "../../lib/fileActions";
import { decorateIn, partition, type GitGroup } from "./status";
import ResourceGroup from "./ResourceGroup";
import CommitBox from "./CommitBox";
import ConfirmDialog from "../ConfirmDialog";
import Menu, { MenuEntry } from "./Menu";
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
  // The open row context menu. Keyed by group as well as path: a path with both
  // staged and unstaged edits has a row (and its own menu) in two groups.
  const [menu, setMenu] = useState<{ x: number; y: number; change: FileChange; group: GitGroup } | null>(null);
  const root = useMemo(() => fileRootOf(taskId), [taskId]);

  const openMenu = (change: FileChange, group: GitGroup, e: React.MouseEvent) => {
    e.preventDefault();
    setMenu({ x: e.clientX, y: e.clientY, change, group });
  };
  const menuPathFor = (group: GitGroup) => (menu?.group === group ? menu.change.path : null);

  const discardAllItem: MenuEntry = {
    label: "Discard All Changes", danger: true,
    onClick: () => confirmDiscard(
      "Discard all changes to tracked files? This cannot be undone.",
      () => gitDiscardAll(taskId)),
  };

  const menuItems = (c: FileChange, group: GitGroup): MenuEntry[] => {
    // Reveal and Copy Path resolve the file on disk, so they can't work once
    // it's gone; Copy Relative Path is just the string and always can.
    const gone = decorateIn(c, group).letter === "D";
    const items: MenuEntry[] = [
      { kind: "header", label: baseName(c.path) },
      { label: "Open Changes", onClick: () => onSelectFile(c.path, group) },
      { kind: "separator" },
    ];
    if (group === "index") {
      items.push(
        { label: "Unstage Changes", glyph: "−", onClick: () => onAct(() => gitUnstage(taskId, c.path)) },
        { label: "Unstage All Changes", onClick: () => onAct(() => gitUnstageAll(taskId)) },
      );
    } else {
      items.push(
        {
          label: group === "merge" ? "Stage Changes (Mark Resolved)" : "Stage Changes",
          glyph: "+",
          onClick: () => onAct(() => gitStage(taskId, c.path)),
        },
        { label: "Stage All Changes", onClick: () => onAct(() => gitStageAll(taskId)) },
      );
    }
    if (group === "workingTree") {
      items.push(
        { kind: "separator" },
        {
          label: "Discard Changes", glyph: "↩", danger: true,
          onClick: () => confirmDiscard(
            `Discard changes to ${c.path}? This cannot be undone.`,
            () => gitDiscard(taskId, c.path, false)),
        },
        discardAllItem,
      );
    }
    if (group === "untracked") {
      items.push(
        { kind: "separator" },
        // Untracked: nothing is committed, so discarding means deleting the file.
        {
          label: "Delete File", glyph: "↩", danger: true,
          onClick: () => confirmDiscard(
            `Delete untracked file ${c.path}? This cannot be undone.`,
            () => gitDiscard(taskId, c.path, true)),
        },
        // Only offered here: adding a tracked path to .gitignore doesn't untrack
        // it, so the row would stay put and the entry would look broken.
        { label: "Add to .gitignore", onClick: () => onAct(() => ignorePath(root, c.path)) },
      );
    }
    items.push(
      { kind: "separator" },
      { label: revealLabel, disabled: gone, onClick: () => reveal(root, c.path) },
      { label: "Copy Path", disabled: gone, onClick: () => copyAbsPath(root, c.path) },
      { label: "Copy Relative Path", onClick: () => copyRelPath(c.path) },
    );
    return items;
  };

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
        menuPath={menuPathFor("merge")}
        onFileContextMenu={(c, e) => openMenu(c, "merge", e)}
        onFilePrimary={(c) => onAct(() => gitStage(taskId, c.path))} />
      <ResourceGroup id="index" label="Staged Changes" changes={g.index}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "index")}
        menuPath={menuPathFor("index")}
        onFileContextMenu={(c, e) => openMenu(c, "index", e)}
        onFilePrimary={(c) => onAct(() => gitUnstage(taskId, c.path))}
        onUnstageAll={() => onAct(() => gitUnstageAll(taskId))} />
      <ResourceGroup id="workingTree" label="Changes" changes={g.workingTree}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "workingTree")}
        menuPath={menuPathFor("workingTree")}
        onFileContextMenu={(c, e) => openMenu(c, "workingTree", e)}
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
        menuPath={menuPathFor("untracked")}
        onFileContextMenu={(c, e) => openMenu(c, "untracked", e)}
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

      {menu && (
        <Menu x={menu.x} y={menu.y} items={menuItems(menu.change, menu.group)}
          onClose={() => setMenu(null)} />
      )}
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
