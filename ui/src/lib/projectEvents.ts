// A project row changed in a way the open views cannot see for themselves.
//
// The app's `project` (the one every panel renders against) is the row the
// tree handed over when it was clicked, and the tree refetches only on mount,
// on the workspace toggle, and on add/close/delete. Nothing refetched when a
// sync adopted the shared backlog's key or Settings renamed it, so the board
// went on labelling issues with the old prefix until the app restarted: the
// conflict markers, keyed by the file's key, never matched a row, and a link
// made from that board wrote the old prefix into another project's file.

/** Fired after a change to a project row, so open views can re-read it. */
export const PROJECTS_CHANGED_EVENT = "agency:projects-changed";

export function notifyProjectsChanged(): void {
  window.dispatchEvent(new CustomEvent(PROJECTS_CHANGED_EVENT));
}
