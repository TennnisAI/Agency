type EscState = "none" | "esc" | "csi" | "osc" | "oscEsc";

export interface CaptureState {
  buf: string;
  done: boolean;
  esc: EscState;
}

export const initialCapture = (): CaptureState => ({ buf: "", done: false, esc: "none" });

// Feed a raw terminal-input chunk (xterm.js onData — typed keys AND automatic
// replies to terminal queries). Returns the (possibly updated) state and, when
// the first non-empty line is submitted, that trimmed line. Whole escape
// sequences (CSI / OSC / other) are skipped, not captured, so a terminal's
// query-replies never pollute the captured prompt.
export function feed(
  state: CaptureState,
  chunk: string,
): { state: CaptureState; line: string | null } {
  if (state.done) return { state, line: null };
  let buf = state.buf;
  let esc = state.esc;

  for (const ch of chunk) {
    const code = ch.codePointAt(0)!;

    // Inside an escape sequence: consume until its terminator.
    if (esc !== "none") {
      if (esc === "esc") {
        esc = ch === "[" ? "csi" : ch === "]" ? "osc" : "none"; // other ESC x = 2-byte
      } else if (esc === "csi") {
        if (code >= 0x40 && code <= 0x7e) esc = "none"; // CSI final byte
      } else if (esc === "osc") {
        if (code === 0x07) esc = "none"; // BEL terminator
        else if (ch === "\x1b") esc = "oscEsc"; // maybe ST (ESC \)
      } else if (esc === "oscEsc") {
        esc = ch === "\\" ? "none" : "esc"; // ESC \ ends OSC; else new ESC
      }
      continue;
    }

    if (ch === "\x1b") {
      esc = "esc";
    } else if (ch === "\r" || ch === "\n") {
      const line = buf.trim();
      if (line.length > 0) {
        return { state: { buf: "", done: true, esc: "none" }, line };
      }
      buf = ""; // empty submission — keep waiting
    } else if (ch === "\x7f" || ch === "\b") {
      buf = buf.slice(0, -1);
    } else if (code >= 0x20 && code !== 0x7f) {
      buf += ch;
    }
    // other control bytes are dropped
  }

  return { state: { buf, done: false, esc }, line: null };
}
