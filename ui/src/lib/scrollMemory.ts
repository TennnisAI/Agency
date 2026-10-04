// Where the reader was in each file, so leaving a view and coming back lands
// them on the same line. The Docs and Files views unmount when another view
// takes the window, and every editor in them goes with it; a fresh editor
// opens at the top, so a long note read halfway down was back at line 1 after
// a glance at the agents (AGE-253). Session only, like the buffer stash
// (editorBuffers): a position is cheap to lose, and a cold start at the top is
// what a reopened app is expected to do.

import type { StateEffect } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

/** A bounded map, least recently remembered evicted first. */
export class ScrollMemory<T> {
  private readonly entries = new Map<string, T>();

  constructor(private readonly max: number) {}

  /** Record a position. Re-remembering a key makes it the newest again. */
  remember(key: string, value: T): void {
    this.entries.delete(key); // Map iteration order = insertion order; refresh age
    this.entries.set(key, value);
    if (this.entries.size > this.max) {
      const oldest = this.entries.keys().next().value;
      if (oldest !== undefined) this.entries.delete(oldest);
    }
  }

  /** The last recorded position, or null. A peek: a position is reused on
   * every remount, unlike a stashed buffer, which belongs to one editor. */
  recall(key: string): T | null {
    return this.entries.has(key) ? (this.entries.get(key) as T) : null;
  }
}

const MAX_POSITIONS = 200;

/**
 * Whether a scroll container's offset is worth recording. A container under a
 * `display: none` ancestor (a background tab) reads 0 for both its height and
 * its scrollTop, and recording that would forget the real place.
 */
export const isMeasurable = (el: { clientHeight: number }): boolean => el.clientHeight > 0;

// The code editors (Files) and the live-preview editor (Docs) share one store,
// keyed on the file itself (editorBuffers.bufferKey): a CodeMirror snapshot
// anchors on a document position, not a pixel offset, so a place taken in one
// lands on the same line in the other.
export const editorScroll = new ScrollMemory<StateEffect<unknown>>(MAX_POSITIONS);

/**
 * Keep `editorScroll` current for a live editor. Recorded as the reader
 * scrolls rather than when the editor is torn down: by then a background tab
 * sits under `display: none` and its scroller reads as the top. The listener
 * goes with the editor's DOM when the view is destroyed.
 */
export function trackEditorScroll(view: EditorView, key: string): void {
  view.scrollDOM.addEventListener("scroll", () => {
    if (isMeasurable(view.scrollDOM)) editorScroll.remember(key, view.scrollSnapshot());
  }, { passive: true });
}

// The rendered markdown preview in the Files view. Pixels, since it is plain
// DOM and has no document positions to anchor on.
export const previewScroll = new ScrollMemory<number>(MAX_POSITIONS);
