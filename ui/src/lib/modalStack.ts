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
