// Cross-domain navigation (one-stop Phase 7): panels and the docs editor can
// jump to any note, issue, or run in any project by dispatching one event;
// the Shell (App.tsx) listens and routes through the same handlers the
// palette and tray use. DOM events, not props, because the sources sit five
// layers under Shell (same pattern as agency:open-note / agency:add-project).

export type NavTarget =
  | { kind: "note"; projectId: string; path: string }
  // A file the Docs editor can't open — an attachment row in the Docs tree.
  // `path` is relative to the project's checkout, not to the docs dir, because
  // that is the root the Files tab is showing.
  | { kind: "file"; projectId: string; path: string }
  | { kind: "issue"; projectId: string; issueId: string }
  | { kind: "run"; projectId: string; runId: string };

export const NAVIGATE_EVENT = "agency:navigate";

export function requestNavigate(target: NavTarget) {
  window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: target }));
}
