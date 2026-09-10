// Which desktop the webview is running on, and the one thing most of the UI
// needs to know about it: what the command key is called.
//
// Observed 2026-09-10 on Linux: every shortcut hint in the app said ⌘ (the
// status bar, the search box, the onboarding button), on a machine with no
// such key. The chords themselves were already bound to `metaKey || ctrlKey`
// everywhere, so only the labels were wrong.

export type Platform = "mac" | "linux" | "windows";

export function detectPlatform(userAgent: string, platform: string): Platform {
  if (platform.startsWith("Mac") || /Macintosh/.test(userAgent)) return "mac";
  if (platform.startsWith("Win") || /Windows/.test(userAgent)) return "windows";
  return "linux";
}

export const PLATFORM: Platform =
  typeof navigator === "undefined" ? "mac" : detectPlatform(navigator.userAgent, navigator.platform);
export const IS_MAC = PLATFORM === "mac";
export const IS_LINUX = PLATFORM === "linux";

/**
 * A shortcut written the Mac way ("⌘⇧D", "⌥⌘F", "⌘↵"), rendered for
 * `platform`. Everything is authored in Mac glyphs because that is how the
 * native menu and the design record spell them; this is the one translation.
 */
export function shortcutLabel(macChord: string, platform: Platform = PLATFORM): string {
  if (platform === "mac") return macChord;
  const parts: string[] = [];
  let key = "";
  for (const ch of macChord) {
    if (ch === "⌘") parts.push("Ctrl");
    else if (ch === "⇧") parts.push("Shift");
    else if (ch === "⌥") parts.push("Alt");
    else if (ch === "⌃") parts.push("Ctrl");
    else key += ch;
  }
  // Modifier order is fixed regardless of how the chord was spelled: Ctrl,
  // Alt, Shift is what every Linux and Windows menu prints.
  const order = ["Ctrl", "Alt", "Shift"];
  parts.sort((a, b) => order.indexOf(a) - order.indexOf(b));
  const named: Record<string, string> = { "↵": "Enter", "⎋": "Esc", "⌫": "Backspace" };
  parts.push(named[key] ?? key);
  return parts.join("+");
}
