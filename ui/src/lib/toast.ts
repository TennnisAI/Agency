// Tiny module-level event bus for error toasts. A bus (not React context) so
// non-component code — store actions, api call sites — can report failures
// without threading a hook through every caller.

export interface ToastMsg {
  id: number;
  text: string;
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

/** Surface a failed action to the user instead of swallowing it. */
export function toastError(err: unknown, context?: string) {
  const raw = err instanceof Error ? err.message : String(err);
  const text = context ? `${context}: ${raw}` : raw;
  const id = nextId++;
  // Collapse duplicates so a polling loop can't stack identical toasts.
  if (toasts.some((t) => t.text === text)) return;
  toasts = [...toasts, { id, text }];
  emit();
  window.setTimeout(() => dismissToast(id), 8000);
}
