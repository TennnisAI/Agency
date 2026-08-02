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
 * arrow keys · use PgUp/PgDn to scroll". A pager or editor, on the other hand,
 * scrolls correctly from those arrows, which is why case 3 is left alone for the
 * shell/run panes and only replaced where arrows mean history.
 *
 * What replaces it: the page keys the agent itself asks for. They are what a TUI
 * binds its scrollback to, and unlike arrows they are not history keys, so the
 * worst case in an agent that ignores them is that nothing happens. This is the
 * path taken when the daemon serving this pane predates the snapshot fix that
 * restores mouse reporting, and for agents that never track the mouse at all.
 */

/** Wheel distance that makes one page key, in CSS pixels (~2 notches). */
export const PAGE_PX = 240;

/** Normalize a wheel delta to pixels; some mice report lines or pages. */
export function wheelPixels(deltaY: number, deltaMode: number): number {
  if (deltaMode === 1) return deltaY * 16; // lines
  if (deltaMode === 2) return deltaY * 400; // pages
  return deltaY;
}

/**
 * Accumulates wheel deltas and emits PgUp (`CSI 5~`) / PgDn (`CSI 6~`) once a
 * page's worth has gone by. Stateful because a trackpad delivers a page as a
 * long run of small deltas; a reversal drops the leftovers so a flick back does
 * not need to pay off the previous direction first.
 */
export function createPageScroller(pagePx = PAGE_PX) {
  let acc = 0;
  return (deltaY: number, deltaMode = 0): string => {
    const px = wheelPixels(deltaY, deltaMode);
    if (px === 0) return "";
    if (Math.sign(px) !== Math.sign(acc)) acc = 0;
    acc += px;
    let out = "";
    while (acc <= -pagePx) {
      out += "\x1b[5~";
      acc += pagePx;
    }
    while (acc >= pagePx) {
      out += "\x1b[6~";
      acc -= pagePx;
    }
    return out;
  };
}

/**
 * True when this pane must not let xterm turn the wheel into arrow keys, i.e.
 * an agent pane showing an alternate-screen app that isn't tracking the mouse.
 */
export function shouldSwallowWheel(bufferType: string, altScrollArrows: boolean): boolean {
  return !altScrollArrows && bufferType === "alternate";
}
