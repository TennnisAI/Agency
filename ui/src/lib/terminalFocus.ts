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
