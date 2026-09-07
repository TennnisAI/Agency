/**
 * The options an agent pane's terminal is built with.
 *
 * Out here rather than inline in `FocusTerminal` because one of them is not a
 * preference: it is half of an agreement with the daemon, and the half that has
 * no other place to be written down. `term/vt_pin.rs` says it plainly — two
 * implementations parse the escape sequences in Agency, alacritty in the daemon
 * and xterm.js here, and where they read the same byte differently the pane and
 * the reattach snapshot show different screens. The daemon's side is pinned by
 * the reattach tests in `term/emulator.rs`; this file is where the pane's side
 * can be fed the same bytes and checked against them.
 *
 * The option in question is `convertEol`, which is off, which is the xterm.js
 * default, and which must stay off. See `termOptions.test.ts` and AGE-204.
 */
import type { ITerminalOptions } from "@xterm/xterm";
import { currentXtermTheme, minContrastRatio, TERMINAL_FONT_FAMILY } from "./themes";

export function paneOptions(): ITerminalOptions {
  return {
    fontSize: 13,
    fontFamily: TERMINAL_FONT_FAMILY,
    cursorBlink: true,
    theme: currentXtermTheme(),
    minimumContrastRatio: minContrastRatio(),
    // Large scrollback: this is the live scroll depth (xterm accumulates the
    // streamed output into its own buffer), so a small cap is what makes long
    // agent conversations "stop" scrolling well before their start. xterm
    // allocates lines lazily, so the ceiling only costs memory once it's filled.
    scrollback: 50000,
  };
}
