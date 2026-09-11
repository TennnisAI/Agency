import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { unlistenQuietly } from "../lib/unlisten";

/**
 * Files dragged in from Finder (or the platform's file manager). Tauri
 * intercepts OS drag-drop at the webview boundary, so HTML5 drop events never
 * reach the page — we subscribe to Tauri's own stream instead.
 *
 * The event is window-global and fires for every mounted subscriber, so each
 * caller decides from the cursor position whether a drop is theirs: `targetAt`
 * returns the target the drop would land on, or null for "not mine". The
 * returned value is whatever is currently under the cursor, for hover feedback.
 *
 * The payload says PhysicalPosition, but on macOS wry reports NSView
 * coordinates (logical points) and tauri-runtime-wry wraps them unscaled — so
 * the values are already CSS pixels and dividing by devicePixelRatio would make
 * the hit-test miss on Retina.
 */
export function useFileDrop<T>(
  targetAt: (x: number, y: number) => T | null,
  onDrop: (paths: string[], target: T) => void,
): T | null {
  const [over, setOver] = useState<T | null>(null);
  // Both callbacks are re-created every render; holding them in refs keeps the
  // subscription to a single install for the component's whole life.
  const targetRef = useRef(targetAt);
  targetRef.current = targetAt;
  const dropRef = useRef(onDrop);
  dropRef.current = onDrop;

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (disposed) return;
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") {
          setOver(targetRef.current(p.position.x, p.position.y));
          return;
        }
        setOver(null);
        if (p.type !== "drop" || p.paths.length === 0) return;
        const target = targetRef.current(p.position.x, p.position.y);
        if (target !== null) dropRef.current(p.paths, target);
      })
      .then((u) => { if (disposed) unlistenQuietly(u); else unlisten = u; });
    return () => { disposed = true; unlistenQuietly(unlisten); };
  }, []);

  return over;
}

/**
 * The `data-drop-dir` value at a point, or null when the point isn't inside
 * `host`. Rows tag themselves with the directory a drop on them lands in ("" is
 * the tree root), and the scroll container carries the root's own tag so blank
 * space below the last row is still a target.
 */
export function dirAtPoint(host: HTMLElement | null, x: number, y: number): string | null {
  if (!host) return null;
  const el = document.elementFromPoint(x, y);
  // Anything the drop can't reach — a modal backdrop, another pane — fails the
  // containment test, so an open dialog neutralizes the tree underneath it.
  if (!el || !host.contains(el)) return null;
  const marked = el.closest<HTMLElement>("[data-drop-dir]");
  return marked && host.contains(marked) ? marked.dataset.dropDir ?? "" : null;
}
