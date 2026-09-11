/**
 * Tearing down a Tauri event subscription without the teardown surfacing as an
 * error toast.
 *
 * `UnlistenFn` is typed `() => void` and is not: it is
 * `async () => _unlisten(event, eventId)`, and the first thing `_unlisten` does
 * is call the unregister function Tauri injects into the page. That script
 * guards the event's listener map but not the entry it then reads:
 *
 *     const listeners = (window['...'] || {})[event]
 *     if (listeners) {
 *       window.__TAURI_INTERNALS__.unregisterCallback(listeners[eventId].handlerId)
 *     }
 *
 * (`tauri-2.11.3/src/event/mod.rs::unlisten_js_script`; the emit script beside
 * it does check its entry.) So unlistening an id the map has already dropped
 * throws `undefined is not an object (evaluating 'listeners[eventId].handlerId')`
 * — inside an `async` function, which makes it a rejected promise rather than a
 * throw at the call site. Nothing awaits an unlisten, so it lands on
 * `window.unhandledrejection`, and `main.tsx` turns that into "Unexpected
 * error" over the app. Observed while toggling the in-agent terminal, which
 * mounts and unmounts a pane and re-runs its drag-drop subscription with it.
 *
 * A failed unregister of a listener that is already gone leaves nothing to
 * clean up, so there is nothing here to tell the user and nothing they could
 * do. It is swallowed rather than reported, and swallowed at the one call shape
 * that produces it rather than by widening `main.tsx`'s benign-error list,
 * which would hide the same message when it means something else.
 *
 * Note the leak this cannot fix: the throw is on the first line of `_unlisten`,
 * so the `invoke('plugin:event|unlisten')` after it never runs and the Rust side
 * keeps its half of the subscription. Only the guard upstream can recover that,
 * because the entry this side would name is one the page has already forgotten.
 *
 * Delete this module once the app is on a `tauri` release that carries the fix.
 * It is merged but unreleased: tauri-apps/tauri#15799, fixed by #15800 (merged
 * 2026-09-07, tagged `patch:bug`), where the script grew the same entry check
 * the emit path already had. The newest published crate is 2.11.5 from
 * 2026-07-01 and we are on 2.11.3, so bumping alone will not do it — check that
 * `unlisten_js_script` in the vendored source reads `listeners[eventId]` into a
 * local and tests it before dropping this.
 */
export function unlistenQuietly(un: (() => void) | undefined): void {
  if (!un) return;
  try {
    // Typed `void`, actually a promise: take the rejection if there is one.
    const pending = un() as unknown as Promise<void> | undefined;
    void pending?.catch?.(() => {});
  } catch {
    // A future version that throws straight out instead of rejecting.
  }
}
