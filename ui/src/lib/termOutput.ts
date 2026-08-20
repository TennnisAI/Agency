/**
 * Coalescing writer for everything a terminal pane receives from its session.
 *
 * The mirror of `termInput`, and for the same reason: a pane should hand xterm
 * a frame's worth of bytes at a time, not a read's worth.
 *
 * xterm renders once per animation frame, from whatever its buffer holds at the
 * time, so a repaint that reaches it as several `write` calls is painted half
 * done if a frame boundary lands between them. Whether that shows depends on
 * how the agent repaints. Claude Code overwrites the cells it changes on the
 * alternate screen, so half a repaint looks like the whole one. Cursor Agent
 * draws on the normal screen the way an Ink app does — blank the old frame with
 * `\x1b[2K\x1b[1A` per line, walking the cursor back up, then write the new one
 * — so half a repaint is a band of blank lines where the output was. That is
 * AGE-151: the Cursor pane flickering on every line it wrote.
 *
 * What tore the repaints up is the kernel, and `term/pty.rs` stitches them back
 * together there, where the pieces can be told apart. This is the second half
 * of the same guarantee, for the pieces no daemon can rejoin: a repaint the
 * agent genuinely writes in two calls, or one long enough to run past the
 * daemon's ceilings. Joining what a frame delivers makes an erase and its
 * redraw one parse and therefore one paint.
 *
 * A terminal stream has no framing of its own, so concatenating consecutive
 * chunks is the same stream; only the timing changes, by at most a frame.
 */
import { nextFrame, type Schedule } from "./termInput";

export interface OutputWriter {
  /** Queue `bytes`; they reach the terminal with the rest of this frame's. */
  write(bytes: Uint8Array): void;
  /** Drop anything still queued and stop scheduling. */
  dispose(): void;
}

export function createOutputWriter(
  send: (bytes: Uint8Array) => void,
  schedule: Schedule = nextFrame,
): OutputWriter {
  let pending: Uint8Array[] = [];
  let cancel: (() => void) | null = null;

  const flush = () => {
    cancel = null;
    const chunks = pending;
    pending = [];
    if (!chunks.length) return;
    if (chunks.length === 1) {
      send(chunks[0]);
      return;
    }
    let total = 0;
    for (const c of chunks) total += c.length;
    const joined = new Uint8Array(total);
    let at = 0;
    for (const c of chunks) {
      joined.set(c, at);
      at += c.length;
    }
    send(joined);
  };

  return {
    write(bytes: Uint8Array) {
      if (!bytes.length) return;
      pending.push(bytes);
      if (!cancel) cancel = schedule(flush);
    },
    // Unlike a keystroke, output that missed the pane's last frame has nowhere
    // to land: the pane is unmounting and its terminal goes with it.
    dispose() {
      cancel?.();
      cancel = null;
      pending = [];
    },
  };
}
