// Tiny module-level event bus for error toasts. A bus (not React context) so
// non-component code — store actions, api call sites — can report failures
// without threading a hook through every caller.

export type ToastKind = "error" | "info" | "success";

export interface ToastMsg {
  id: number;
  text: string;
  kind: ToastKind;
}

type Listener = (toasts: ToastMsg[]) => void;

let toasts: ToastMsg[] = [];
let listeners: Listener[] = [];
let nextId = 1;

function emit() {
  for (const l of listeners) l(toasts);
}

export function subscribeToasts(l: Listener): () => void {
  listeners.push(l);
  l(toasts);
  return () => {
    listeners = listeners.filter((x) => x !== l);
  };
}

export function dismissToast(id: number) {
  toasts = toasts.filter((t) => t.id !== id);
  emit();
}

function push(text: string, kind: ToastKind, ttl: number) {
  const id = nextId++;
  // Collapse duplicates so a polling loop can't stack identical toasts.
  if (toasts.some((t) => t.text === text && t.kind === kind)) return;
  toasts = [...toasts, { id, text, kind }];
  emit();
  window.setTimeout(() => dismissToast(id), ttl);
}

/** Surface a failed action to the user instead of swallowing it. */
export function toastError(err: unknown, context?: string) {
  const raw = err instanceof Error ? err.message : String(err);
  const text = context ? `${context}: ${raw}` : raw;
  push(text, "error", 8000);
}

/** Confirm a completed action (e.g. "Committed", "Pushed"). Auto-dismisses fast. */
export function toastSuccess(text: string) {
  push(text, "success", 3500);
}

/** Neutral, transient status note. */
export function toastInfo(text: string) {
  push(text, "info", 3500);
}
