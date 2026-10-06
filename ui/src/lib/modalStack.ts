// Module-level stack of open modals, so stacked dialogs (e.g. a ConfirmDialog
// on top of the MergeModal) don't all close on one Escape: only the handler on
// top of the stack fires. Kept free of DOM/React so it's unit-testable.

type Handler = () => void;

const stack: Handler[] = [];

/** Register an open modal's close handler. Returns its unregister function. */
export function pushModal(h: Handler): () => void {
  stack.push(h);
  return () => {
    const i = stack.indexOf(h);
    if (i >= 0) stack.splice(i, 1);
  };
}

/** The close handler of the topmost (most recently opened) modal, if any. */
export function topModal(): Handler | null {
  return stack.length > 0 ? stack[stack.length - 1] : null;
}

/**
 * Whether a keydown is an Escape the modal on top should close for. One that
 * something inside the modal has already acted on is not: Escape that closed
 * the description's link suggestions in the new-issue composer also closed the
 * composer, which files the issue once it has a title, so dismissing a
 * `[[` popup mid-sentence created a half-written issue. CodeMirror marks a key
 * it handled with preventDefault but lets it bubble on to the window.
 */
export function escapeCloses(e: { key: string; defaultPrevented: boolean }): boolean {
  return e.key === "Escape" && !e.defaultPrevented;
}
