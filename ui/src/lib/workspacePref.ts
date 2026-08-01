// Whether the pinned workspace is hidden — for people who don't want the
// journaling/notes home at all. A per-machine UI preference (localStorage),
// so it never touches the shared registry; hiding a *created* workspace
// additionally closes its project row (see Settings), which keeps records
// and files intact for a later re-enable.

const KEY = "agency:workspace-hidden";

/** Fired on every preference change so open views can re-read it. */
export const WORKSPACE_HIDDEN_EVENT = "agency:workspace-hidden-changed";

export function workspaceHidden(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

export function setWorkspaceHidden(hidden: boolean): void {
  try {
    if (hidden) localStorage.setItem(KEY, "1");
    else localStorage.removeItem(KEY);
  } catch {
    /* storage unavailable */
  }
  window.dispatchEvent(new CustomEvent(WORKSPACE_HIDDEN_EVENT));
}
