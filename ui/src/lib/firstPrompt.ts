export interface CaptureState {
  buf: string;
  done: boolean;
}

export const initialCapture = (): CaptureState => ({ buf: "", done: false });

// Feed a raw terminal-input chunk. Returns the (possibly updated) state and,
// when the first non-empty line is submitted, that trimmed line.
export function feed(
  state: CaptureState,
  chunk: string,
): { state: CaptureState; line: string | null } {
  if (state.done) return { state, line: null };
  let buf = state.buf;

  for (const ch of chunk) {
    const code = ch.codePointAt(0)!;
    if (ch === "\r" || ch === "\n") {
      const line = buf.trim();
      if (line.length > 0) {
        return { state: { buf: "", done: true }, line };
      }
      buf = ""; // empty submission — keep waiting
    } else if (ch === "\x7f" || ch === "\b") {
      buf = buf.slice(0, -1);
    } else if (code >= 0x20 && code !== 0x7f) {
      buf += ch;
    }
    // other control bytes (e.g. ESC sequences) are dropped
  }

  return { state: { buf, done: false }, line: null };
}
