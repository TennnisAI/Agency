// The in-flight git action, per repo. A push started in one project keeps
// running when you switch to another, so its progress can't live in GitPanel's
// component state: the panel is reused across projects (same tree position,
// new taskId) and project B would render project A's bar. Keyed here instead,
// so each repo shows only its own op, and switching back to a still-pushing
// repo picks its progress right back up.

import { useSyncExternalStore } from "react";
import { CANCELLED, type CloneProgress } from "../../api";

export type GitOp = {
  /** An action is running: buttons disable, the progress bar shows. */
  busy: boolean;
  /** Streamed push/sync progress; null when the op reports none (yet). */
  progress: CloneProgress | null;
  /** Sticky error from the last action — survives the follow-up refresh. */
  error: string;
};

const IDLE: GitOp = { busy: false, progress: null, error: "" };

const ops = new Map<string, GitOp>();
const listeners = new Set<() => void>();

const isIdle = (op: GitOp): boolean => !op.busy && op.progress === null && !op.error;

/** The repo's current op. Idle repos share one frozen object, so the snapshot
 *  identity useSyncExternalStore compares stays stable while nothing changes. */
export function gitOp(taskId: string): GitOp {
  return ops.get(taskId) ?? IDLE;
}

export function setGitOp(taskId: string, patch: Partial<GitOp>): void {
  const next = { ...gitOp(taskId), ...patch };
  // Drop idle entries rather than accumulating one per repo ever touched.
  if (isIdle(next)) ops.delete(taskId);
  else ops.set(taskId, next);
  listeners.forEach((notify) => notify());
}

function subscribe(notify: () => void): () => void {
  listeners.add(notify);
  return () => { listeners.delete(notify); };
}

export function useGitOp(taskId: string): GitOp {
  return useSyncExternalStore(subscribe, () => gitOp(taskId));
}

/** Whether an op's error is the user pressing Cancel rather than a failure to
 *  report: the backend fails a killed push with exactly this word. Matched
 *  exactly, because git's own output can mention cancelling (a server-side hook
 *  rejecting a push, say) and swallowing that would leave the panel looking as
 *  if nothing had happened. */
export function isCancelled(error: string): boolean {
  return error.trim() === CANCELLED;
}

/** Test hook. */
export function clearGitOps(): void {
  ops.clear();
}
