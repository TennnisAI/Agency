import { describe, it, expect } from "vitest";
import type { Terminal } from "@xterm/xterm";
import { createPaneTerminal, paneOptions } from "./termOptions";

const bytes = (s: string) => new TextEncoder().encode(s);

/** The pane's own terminal, headless, after parsing `data`. */
const pane = async (data: string) => {
  const term = createPaneTerminal({ cols: 20, rows: 4 });
  await new Promise<void>((done) => term.write(bytes(data), done));
  return term;
};

const row = (term: Terminal, y: number) =>
  term.buffer.active.getLine(y)!.translateToString(true);

describe("the agent pane's terminal", () => {
  it("holds the column a TUI drew at across a bare line feed (AGE-204)", async () => {
    // What crush emits to blank the area behind its command palette: erase a
    // run of cells, step down with a bare LF holding the column, erase the same
    // run again. With `convertEol` on, every step after the first landed at
    // column 0 — the palette lost the left-hand label off each of its rows, the
    // sidebar behind it came apart into fragments down the left edge of the
    // pane, and the rows below drifted. A real terminal does not return the
    // carriage here, and neither does the daemon's emulator: the same bytes,
    // the same columns, in `term/emulator.rs::a_bare_line_feed_holds_the_column`.
    const term = await pane("\x1b[1;6Habcd\x1b[6X\nefgh\x1b[6X\nij");
    expect([row(term, 0), row(term, 1), row(term, 2)]).toEqual([
      "     abcd", "         efgh", "             ij",
    ]);
  });

  it("leaves convertEol off, which is what makes that true", () => {
    // Named on its own because the assertion above passes either way if xterm
    // ever changes what the option does; this is the setting itself.
    expect(paneOptions().convertEol).toBeFalsy();
  });

  it("still starts a line at column 0 when the child returns the carriage", async () => {
    // The other half: a cooked-mode child's `\n` reaches the pane as `\r\n`,
    // because the pty is opened with the default termios and the kernel's
    // ONLCR has already done the conversion. Nothing about dropping the option
    // changes those.
    const term = await pane("\x1b[1;6Habcd\r\nefgh");
    expect([row(term, 0), row(term, 1)]).toEqual(["     abcd", "efgh"]);
  });

  it("repaints after an emoji at the column the agent counted (AGE-234)", async () => {
    // Claude Code draws "❌ Not yet", then rewrites the `t` of "Not" by moving
    // to the column it counted: emoji in 1-2, space in 3, so `t` in 6. Under
    // xterm's default Unicode 6 table the emoji took one column, so column 6 was
    // the space and the row read "❌ Nottyet". The same bytes, the same columns,
    // in `term/emulator.rs::a_wide_emoji_takes_the_columns_the_pane_gives_it`.
    const term = await pane("❌ Not yet\x1b[1;6Ht\r\n🟡 Not yet\x1b[2;6Ht");
    expect([row(term, 0), row(term, 1)]).toEqual(["❌ Not yet", "🟡 Not yet"]);
  });

  it("measures the glyphs agents draw with the daemon's widths", async () => {
    // Each case is written at column 0 and read back as where the cursor ended
    // up. The expected columns are alacritty's, from the daemon test named above.
    const cases: [string, number][] = [
      ["❌", 2], ["🟡", 2], ["✅", 2], ["🧪", 2], ["中", 2],
      ["⚠️", 1], ["⏺", 1], ["⎿", 1], ["●", 1], ["✻", 1],
    ];
    const got: [string, number][] = [];
    for (const [glyph] of cases) {
      const term = await pane(glyph);
      got.push([glyph, term.buffer.active.cursorX]);
    }
    expect(got).toEqual(cases);
  });
});
