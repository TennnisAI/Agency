import type { Checkpoint, CheckpointKind, CheckpointPreview } from "../../api";

/** What a checkpoint's row calls it. */
export function checkpointLabel(kind: CheckpointKind): string {
  switch (kind) {
    case "runStart": return "Run started";
    case "promptSent": return "Prompt sent";
    case "turnEnded": return "Turn ended";
    case "beforeRestore": return "Before a restore";
    default: return "Checkpoint";
  }
}

/**
 * Newest first, each paired with the checkpoint before it: the "changed this
 * turn" diff is from `prev` to `cp`. The oldest has no `prev`, since nothing
 * earlier was saved to compare it with.
 */
export function newestFirst(list: Checkpoint[]): { cp: Checkpoint; prev: Checkpoint | null }[] {
  const sorted = [...list].sort((a, b) => a.seq - b.seq);
  return sorted.map((cp, i) => ({ cp, prev: i > 0 ? sorted[i - 1] : null })).reverse();
}

const files = (n: number) => (n === 1 ? "1 file" : `${n} files`);

/**
 * The confirm dialog's account of a restore, one sentence per line, in the
 * order they are shown. `checkout` is a run working in the project checkout
 * rather than a worktree of its own, where "the files" are the user's too.
 */
export function restoreLines(p: CheckpointPreview, checkout: boolean): string[] {
  if (p.unsaved.length > 0) {
    const n = p.unsaved.length;
    const named = n > 3 ? `${p.unsaved.slice(0, 3).join(", ")} and ${n - 3} more` : p.unsaved.join(", ");
    return [
      `Restoring would overwrite or delete ${named}, which no checkpoint can hold.`,
      `${n === 1 ? "It is" : "They are"} ignored by git, too large to save, or part of a separate repository. Move ${n === 1 ? "it" : "them"} out of the workspace, then try again.`,
    ];
  }
  if (p.write + p.remove === 0) return ["The files already match this checkpoint."];
  const lines: string[] = [];
  if (p.write > 0) lines.push(`${files(p.write)} go${p.write === 1 ? "es" : ""} back to that version.`);
  if (p.remove > 0) lines.push(`${files(p.remove)} created since ${p.remove === 1 ? "is" : "are"} deleted.`);
  lines.push("Files git ignores are left as they are.");
  if (p.headMoved) {
    lines.push(
      "The branch has commits made after this point. They stay, and the restored files show as uncommitted changes.",
    );
  }
  if (checkout) {
    lines.push("This agent works in your project checkout, so anything else changed there since then goes back too.");
  }
  return lines;
}

/** Whether the preview allows going ahead at all. */
export function canRestore(p: CheckpointPreview): boolean {
  return p.unsaved.length === 0 && p.write + p.remove > 0;
}
