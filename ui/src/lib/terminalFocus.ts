/**
 * Whether keyboard focus sits inside a terminal. xterm parks focus in a helper
 * textarea inside the `.terminal` container, so an ancestor check finds it.
 *
 * View-level chords are bound with `metaKey || ctrlKey` (⌘ on macOS, Ctrl
 * elsewhere), and Ctrl+W / Ctrl+P are load-bearing shell bindings — delete-word
 * and previous-command. Now that the Docs and Files views can host an agent
 * terminal, those views have to stand down while it's focused, or typing in the
 * shell closes file tabs and opens palettes.
 */
export function inTerminal(el: Element | null): boolean {
  return !!el?.closest(".terminal");
}

export function terminalHasFocus(): boolean {
  return typeof document !== "undefined" && inTerminal(document.activeElement);
}

/**
 * The focus report a pane owes its child after replaying a reattach snapshot,
 * or `null` when the child is not asking for one.
 *
 * Focus reporting (`?1004h`) is state the child sets once and then trusts. An
 * agent uses it to decide whether to draw its own text cursor: cursor-agent
 * repaints its input line without the inverse-video block the moment it is told
 * focus went away, and puts it back when told focus returned. That makes the
 * two reports a matched pair, and losing the second one is a pane whose cursor
 * never comes back.
 *
 * Both halves of the pair go missing on a reattach, from opposite directions.
 * The blur that navigating away produces *is* delivered — xterm reports it off
 * its own textarea — so the child hides its cursor. Coming back builds a new
 * terminal that already holds focus, so no focus event ever fires in it, and
 * the only `ESC[I` in play is the one xterm answers the snapshot's own `?1004h`
 * with, which the replay deliberately mutes (see `termInput.suspend`, AGE-147).
 * The child is left believing it is unfocused for as long as the pane is open:
 * the cursor is gone, typing still lands (AGE-171).
 *
 * So the pane states its real focus, once, after the replay. Answering from
 * what the DOM actually says is what separates this from the muted reply, which
 * reports whatever the element happened to carry mid-repaint.
 */
export function focusReport(sendFocus: boolean, focused: boolean): string | null {
  if (!sendFocus) return null;
  return focused ? "\x1b[I" : "\x1b[O";
}
