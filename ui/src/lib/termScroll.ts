/**
 * Wheel policy for the embedded terminals.
 *
 * xterm.js has three wheel behaviours, picked in this order:
 *   1. the app asked for mouse reporting -> real wheel events are sent to it
 *      (our custom handler is never consulted in this case);
 *   2. the buffer has scrollback (normal screen) -> the viewport scrolls;
 *   3. otherwise (alternate screen, no mouse reporting) -> "alternate scroll":
 *      every wheel notch is translated into an Up/Down arrow key.
 *
 * Case 3 is the bug behind AGE-15. Agent TUIs live on the alternate screen and
 * read Up/Down as prompt-history navigation, so scrolling silently rewrites the
 * prompt; Claude Code even detects the burst and warns "Scroll wheel is sending
 * arrow keys". A pager or editor, on the other hand, scrolls correctly from
 * those arrows, which is why the fallback is kept for the shell/run panes and
 * only suppressed where arrows mean history.
 *
 * This is also what lets the daemon-side fix (snapshots that restore mouse
 * reporting) ship without forcing a daemon replacement: against a daemon that
 * predates it, an agent pane simply doesn't scroll instead of typing arrows.
 */
export function shouldSwallowWheel(bufferType: string, altScrollArrows: boolean): boolean {
  return !altScrollArrows && bufferType === "alternate";
}
