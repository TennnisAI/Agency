// Popped-out windows (AGE-252): one agent, file or note in a window of its
// own. The backend keeps the list (crates/agency-app/src/popout.rs); this is
// the shape of an entry and the questions both windows ask about it.
//
// One window shows a popped-out item at a time. While it is out, the main
// window draws a placeholder where the item would be, with a way to bring it
// back, so two windows never edit one file or fight over one terminal's size.

export type PopoutRoot = { kind: "run"; id: string } | { kind: "project"; id: string };

export type PopoutTarget =
  | { kind: "run"; projectId: string; runId: string }
  // `path` is relative to `root`, as the Files tab holds it.
  | { kind: "file"; projectId: string; root: PopoutRoot; path: string }
  // `path` is relative to the docs folder, as the Docs tab holds it.
  | { kind: "note"; projectId: string; root: PopoutRoot; path: string };

export interface PopoutEntry {
  label: string;
  target: PopoutTarget;
}

/** Payload of `popout-reattached`: an item on its way back to the main window. */
export interface Reattached {
  label: string;
  target: PopoutTarget;
  /** Unsaved edits the popout had in that file. */
  draft: string | null;
  /** Bring the item up in the main window (false when the popout was just closed). */
  show: boolean;
}

/** Every popout window's label starts with this (see popout.rs). */
export const POPOUT_LABEL_PREFIX = "popout-";

export const isPopoutLabel = (label: string | null | undefined): boolean =>
  !!label && label.startsWith(POPOUT_LABEL_PREFIX);

const sameRoot = (a: PopoutRoot, b: PopoutRoot) => a.kind === b.kind && a.id === b.id;

export function sameTarget(a: PopoutTarget, b: PopoutTarget): boolean {
  if (a.kind === "run" && b.kind === "run") return a.runId === b.runId;
  if (a.kind === "file" && b.kind === "file") return sameRoot(a.root, b.root) && a.path === b.path;
  if (a.kind === "note" && b.kind === "note") return sameRoot(a.root, b.root) && a.path === b.path;
  return false;
}

/** The popout showing `target`, if it is out. */
export function findPopout(list: PopoutEntry[], target: PopoutTarget): PopoutEntry | null {
  return list.find((e) => sameTarget(e.target, target)) ?? null;
}

/**
 * The popout showing this run, if any. A run's extra agent tabs are sessions
 * named `<runId>--<n>`, and they go out with the run: the popout carries the
 * whole tab strip, so any of them is "this agent is in another window".
 */
export function popoutForSession(list: PopoutEntry[], sessionId: string): PopoutEntry | null {
  const runId = sessionId.split("--")[0];
  return list.find((e) => e.target.kind === "run" && e.target.runId === runId) ?? null;
}

/** The last path segment: what a window titled after a file is called. */
export function popoutTitle(target: PopoutTarget, runLabel?: string): string {
  if (target.kind === "run") return runLabel?.trim() || "Agent";
  const name = target.path.split("/").filter(Boolean).pop() ?? target.path;
  return target.kind === "note" ? name.replace(/\.md$/i, "") : name;
}
