/**
 * The terminal an agent pane is built with.
 *
 * Out here rather than inline in `FocusTerminal` because two of its settings are
 * not preferences: they are half of an agreement with the daemon, and the half
 * that has no other place to be written down. `term/vt_pin.rs` says it plainly —
 * two implementations parse the escape sequences in Agency, alacritty in the
 * daemon and xterm.js here, and where they read the same byte differently the
 * pane and the reattach snapshot show different screens. The daemon's side is
 * pinned by the reattach tests in `term/emulator.rs`; this file is where the
 * pane's side can be fed the same bytes and checked against them.
 *
 * The first is `convertEol`, which is off, which is the xterm.js default, and
 * which must stay off (AGE-204). The second is the width table a glyph is
 * measured with, which is Unicode 11 and not the default (AGE-234). See
 * `termOptions.test.ts` for both.
 */
import { Terminal, type ITerminalInitOnlyOptions, type ITerminalOptions } from "@xterm/xterm";
import { Unicode11Addon } from "@xterm/addon-unicode11";
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
    // `term.unicode`, which the width table is switched through, is still
    // proposed API in xterm 5.5; without this the addon throws on load.
    allowProposedApi: true,
  };
}

/** A pane's terminal: `paneOptions` (plus `overrides`) and the width table. */
export function createPaneTerminal(
  overrides: ITerminalOptions & ITerminalInitOnlyOptions = {},
): Terminal {
  const term = new Terminal({ ...paneOptions(), ...overrides });
  // AGE-234: xterm's default table is Unicode 6, which gives ❌, 🟡 and ✅ one
  // column. Claude Code counts them as two, and so does the daemon's alacritty.
  // Claude Code repaints a line by moving the cursor to a column it counted
  // itself, so every write after an emoji landed one column left of where it was
  // aimed: "❌ Noxt yet", "🟡 Notd yet", "tex ycolors". A scroll repainted the
  // screen and moved the stray letters somewhere else, which is why they looked
  // like the agent's bug and not the pane's. Unicode 11 matches alacritty on
  // every glyph the tests below check.
  term.loadAddon(new Unicode11Addon());
  term.unicode.activeVersion = "11";
  return term;
}
