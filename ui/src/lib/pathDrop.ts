/**
 * Where a path dragged out of a sidebar can land.
 *
 * Dropping a file from Finder onto an agent pane types its path at the prompt,
 * which is how a path gets into a conversation without anyone reading it out
 * loud. A file already open in Agency's own tree had no such gesture: the way
 * to hand the agent beside it the note you are reading was to copy the path
 * from a context menu and paste it (AGE-200).
 *
 * The two halves are registered apart because the tree and the terminal never
 * meet: a terminal registers itself as a sink while it is mounted, the tree
 * asks who is under the cursor mid-drag, and neither imports the other. Same
 * shape as `findBus`, and for the same reason.
 *
 * The drag itself is pointer-based in the trees, not HTML5 drag-and-drop —
 * Tauri intercepts drops at the NSView level on macOS, so an in-page HTML5
 * drag lifts but its drop never fires (see `FileTree`). All this module needs
 * from that is a point.
 */

export interface PathSink {
  /** The element this sink owns; a drop inside it is a drop on this sink. */
  host: () => HTMLElement | null;
  /**
   * What the drag hint calls it, as a noun phrase: "the agent", "the
   * terminal". It goes into "Add the path to …" while the pointer hovers.
   */
  label: string;
  /** Take the dropped paths (already absolute). */
  accept: (paths: string[]) => void;
  /** Hover feedback, mirroring the highlight an OS drop over the same pane gets. */
  setOver?: (over: boolean) => void;
}

const sinks = new Set<PathSink>();

/** Register while mounted. Returns the unregister for the effect's cleanup. */
export function registerPathSink(sink: PathSink): () => void {
  sinks.add(sink);
  return () => {
    sinks.delete(sink);
    sink.setOver?.(false);
  };
}

/**
 * The sink under a viewport point, or null. A hidden sink (an inactive tab's
 * terminal is `display: none`) contains nothing at any point, so it can never
 * win — `elementFromPoint` returns whatever is actually painted there.
 */
export function pathSinkAt(x: number, y: number): PathSink | null {
  const el = document.elementFromPoint(x, y);
  if (!el) return null;
  for (const sink of sinks) {
    const host = sink.host();
    if (host && host.contains(el)) return sink;
  }
  return null;
}

/**
 * Pointer bookkeeping for one drag: which sink is under the cursor, with the
 * hover feedback kept exclusive so a drag that crosses two panes never leaves
 * the first one lit. A tree calls `over` on each mousemove and `clear` when
 * the drag ends, however it ends.
 */
export function createSinkTracker(): {
  over: (x: number, y: number) => PathSink | null;
  clear: () => void;
} {
  let current: PathSink | null = null;
  const set = (next: PathSink | null): PathSink | null => {
    if (next !== current) {
      current?.setOver?.(false);
      current = next;
      current?.setOver?.(true);
    }
    return current;
  };
  return {
    over: (x, y) => set(pathSinkAt(x, y)),
    clear: () => void set(null),
  };
}

/** Test seam: the registry is module state, and a test that adds must remove. */
export function resetPathSinks(): void {
  sinks.clear();
}

/**
 * One path as an argument a shell (or an agent's prompt parser) reads as a
 * single word. Only whitespace forces quoting — a bare path is what someone
 * pasting into a chat prompt wants to see, and single quotes around every one
 * of them is noise. Inside quotes a single quote is closed, escaped and
 * reopened, the one form `'…'` has no escape of its own for.
 */
export function quoteArg(path: string): string {
  return /\s/.test(path) ? `'${path.replace(/'/g, `'\\''`)}'` : path;
}

/**
 * What a drop types at the prompt: the paths, space-separated, with a trailing
 * space so whatever the user types next doesn't run into the last one.
 */
export function pathsToInput(paths: string[]): string {
  return paths.map(quoteArg).join(" ") + " ";
}
