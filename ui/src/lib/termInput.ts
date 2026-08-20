/**
 * Coalescing writer for everything a terminal pane sends to its session.
 *
 * Agent TUIs turn on any-event mouse tracking (Copilot CLI and Claude Code both
 * ask for `?1003h ?1006h`), which makes the pane a firehose: xterm reports every
 * wheel notch and every pointer move as its own escape sequence, and each one
 * used to be an IPC call of its own — webview → app → daemon → a `write` to the
 * pty. The agent then answers each report with a full repaint that has to come
 * all the way back. That round trip per notch is why scrolling a Copilot pane
 * lags behind the wheel (AGE-67).
 *
 * Batching a frame's worth of input into one write shortens both halves. The
 * obvious half is ours: one IPC call instead of dozens. The larger half is the
 * agent's — reports that arrive in a single read are handled in one pass and
 * repaint once, where the same reports spread over several reads repaint once
 * each. Nothing is dropped, so a gesture still travels exactly as far as it did.
 *
 * A terminal's input is a byte stream with no framing of its own, so joining
 * consecutive writes is the same stream; only the timing changes, by at most a
 * frame.
 */

/** Backstop flush, for when rAF is paused because the window is hidden. */
const FLUSH_MS = 16;

export interface InputWriter {
  /** Queue `data`; it goes out with everything else queued this frame. */
  write(data: string): void;
  /**
   * Drop everything written until the returned resume is called.
   *
   * For replaying the reattach snapshot into the pane. The repaint is history,
   * but the terminal it is replayed into is live: xterm answers some of what it
   * parses, and by the time an answer reaches the child it is indistinguishable
   * from a keystroke. The snapshot re-asserts focus reporting with `?1004h`, and
   * xterm 5.5 fires `_reportFocus()` straight out of `setModePrivate` case 1004
   * (`InputHandler.ts`), so every reattach sends the agent an `ESC[I` or `ESC[O`
   * that nobody generated — and which of the two depends on whether the pane
   * element happened to carry the `focus` class mid-repaint, not on anything the
   * user did. Real focus changes still reach the child: xterm reports those from
   * its own textarea focus and blur handlers, which this does not touch.
   *
   * The window is one repaint, before the pane can take focus, and a keystroke
   * that lands inside it is dropped rather than queued: the reply we are here to
   * swallow is an escape sequence and so is an arrow key, and guessing which is
   * which would be worse than losing a character that almost never exists.
   * Resume is idempotent.
   */
  suspend(): () => void;
  /** Send anything still queued and stop scheduling. */
  dispose(): void;
}

/** Runs `fn` on the next frame; returns a cancel. */
export type Schedule = (fn: () => void) => () => void;

const nextFrame: Schedule = (fn) => {
  let done = false;
  const run = () => {
    if (done) return;
    done = true;
    cancelAnimationFrame(frame);
    clearTimeout(timer);
    fn();
  };
  const frame = requestAnimationFrame(run);
  const timer = window.setTimeout(run, FLUSH_MS);
  return () => {
    done = true;
    cancelAnimationFrame(frame);
    clearTimeout(timer);
  };
};

export function createInputWriter(
  send: (data: string) => void,
  schedule: Schedule = nextFrame,
): InputWriter {
  let pending = "";
  let cancel: (() => void) | null = null;
  let suspended = 0;

  const flush = () => {
    cancel = null;
    const out = pending;
    pending = "";
    if (out) send(out);
  };

  return {
    write(data: string) {
      if (!data || suspended) return;
      pending += data;
      if (!cancel) cancel = schedule(flush);
    },
    suspend() {
      suspended += 1;
      let released = false;
      return () => {
        if (released) return;
        released = true;
        suspended -= 1;
      };
    },
    dispose() {
      // Teardown is not a reason to swallow a keystroke: a pane can unmount on
      // the same frame as the Enter that was typed into it.
      cancel?.();
      flush();
    },
  };
}
