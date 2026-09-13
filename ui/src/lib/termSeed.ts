/**
 * The placeholder frame a pane paints while it waits for its session's first
 * live one.
 *
 * `run_preview` answers with the daemon's `Emulator::capture`: plain text, rows
 * joined by a bare `\n`, no escapes. That is a *listing* of the grid, not a
 * stream a terminal drew — and a terminal reads a bare line feed as "down one
 * row, same column", which is what a terminal is supposed to do (AGE-204).
 *
 * The pane used to run xterm.js with `convertEol`, which returned the carriage
 * on every line feed and made the listing land as lines. Dropping that option
 * fixed the pane and left the seed painting a staircase: every row one column
 * further right than the last, walking off the screen. It shows for as long as
 * the attach takes, which is a session resume rather than a frame.
 *
 * So the seed says what it means. `\r\n` is the pane's own text, not the
 * child's, so nothing here has to agree with the daemon's reading of anything.
 */
export function seedFrame(preview: string): string {
  // The daemon pads its capture out to the pane height and we used to add a
  // newline of our own; both rendered as a block of blank rows on every open.
  const trimmed = preview.replace(/[\r\n]+$/, "");
  if (!trimmed) return "";
  return trimmed.split("\n").map((line) => line.replace(/\r$/, "")).join("\r\n");
}
