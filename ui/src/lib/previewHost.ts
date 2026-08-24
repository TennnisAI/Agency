// Which runs' previews are visibly hosted right now (a RunPanel rendering the
// instrumented preview iframe). The app-level PreviewKeeper subscribes and
// mounts a hidden host for every previewable run *not* in this set, so exactly
// one preview page exists per run: the visible pane when the user has the Run
// tab open, a hidden one otherwise — either way the dispatched agent's preview
// tools stay connected (AGE-143).

const visible = new Set<string>();
const subs = new Set<() => void>();

function notify() {
  for (const fn of subs) fn();
}

export function markPreviewVisible(runId: string) {
  visible.add(runId);
  notify();
}

export function clearPreviewVisible(runId: string) {
  visible.delete(runId);
  notify();
}

export function isPreviewVisible(runId: string): boolean {
  return visible.has(runId);
}

export function subscribePreviewHosts(fn: () => void): () => void {
  subs.add(fn);
  return () => subs.delete(fn);
}
