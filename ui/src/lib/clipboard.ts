/**
 * Reading a paste, without trusting the webview's copy of the clipboard.
 *
 * A macOS pasteboard can hold several items at once, and apps use that for a
 * multi-row selection: copying a block of build errors out of Xcode writes one
 * item per row. WebKit fills a paste event's `clipboardData` from the *first*
 * item only, so `getData("text/plain")` hands the page one row and drops the
 * rest — the whole clipboard is there, the webview just never asks for it. A
 * clipboard that also carries file URLs is worse: the event arrives typed as
 * "Files" with no text at all. Pasting into a native text field takes a
 * different path inside WebKit and gets everything, which is why a bare
 * terminal, or a round trip through another app (which collapses the
 * pasteboard to one item), pastes the full text. That is AGE-41.
 *
 * xterm.js pastes what `clipboardData` gives it, so a terminal pane asks the
 * backend for the real pasteboard instead (`read_clipboard_text` reads every
 * item) and falls back to the webview's text if that read comes up empty.
 */

import { readClipboardText } from "../api";

/**
 * The full clipboard text for a paste. `fromWebview` is the paste event's own
 * `text/plain`, used when the backend has nothing better — it is the truncated
 * copy, but on a single-item clipboard the two agree, and it is all there is
 * if the native read fails.
 */
export async function fullClipboardText(
  fromWebview: string,
  read: () => Promise<string | null> = readClipboardText,
): Promise<string> {
  try {
    const native = await read();
    return native || fromWebview;
  } catch {
    return fromWebview;
  }
}

/**
 * The clipboard as text for a paste the UI performs itself (a menu item, say)
 * rather than one the webview hands over. Same order: the real pasteboard,
 * then the webview's own reader — which needs a user gesture and can be
 * refused outright, so its rejection is left to propagate to the caller.
 */
export async function clipboardText(
  read: () => Promise<string | null> = readClipboardText,
): Promise<string> {
  const native = await read().catch(() => null);
  return native || (await navigator.clipboard.readText());
}
