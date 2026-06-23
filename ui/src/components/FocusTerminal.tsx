import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { attachRun, detachRun, resizeRun, runInput, runPreview,
  attachRunScript, detachRunScript, resizeRunScript, runScriptInput, runScriptPreview } from "../api";
import { currentXtermTheme } from "../lib/themes";
import { initialCapture, feed } from "../lib/firstPrompt";

export interface TerminalStream {
  attach(id: string, onBytes: (b: Uint8Array) => void): Promise<void>;
  detach(id: string): void;
  resize(id: string, cols: number, rows: number): Promise<void>;
  input(id: string, data: string): Promise<void>;
  preview(id: string, lines: number): Promise<string>;
}

export const agentStream: TerminalStream = {
  attach: attachRun, detach: detachRun, resize: resizeRun, input: runInput, preview: runPreview,
};

export const runStream: TerminalStream = {
  attach: attachRunScript, detach: detachRunScript, resize: resizeRunScript,
  input: runScriptInput, preview: runScriptPreview,
};

export default function FocusTerminal(
  { runId, stream = agentStream, onFirstPrompt }: { runId: string; stream?: TerminalStream; onFirstPrompt?: (line: string) => void },
) {
  const ref = useRef<HTMLDivElement>(null);
  const onFirstPromptRef = useRef(onFirstPrompt);
  onFirstPromptRef.current = onFirstPrompt;
  const captureRef = useRef(initialCapture());

  useEffect(() => {
    const container = ref.current;
    if (!container) return;
    const term = new Terminal({ convertEol: true, fontSize: 13, cursorBlink: true, theme: currentXtermTheme() });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(container);
    // Fit xterm to its container, then push the new size to the backend so the
    // PTY (and thus tmux) reflows to match. resize_run is a no-op until the
    // attach lands, so it's safe to call before/while attaching.
    const doFit = () => {
      try {
        fit.fit();
        if (term.cols > 0 && term.rows > 0) stream.resize(runId, term.cols, term.rows).catch(() => {});
      } catch { /* not laid out */ }
    };
    requestAnimationFrame(() => { doFit(); term.focus(); });
    const ro = new ResizeObserver(doFit);
    ro.observe(container);

    const onThemeChange = () => { term.options.theme = currentXtermTheme(); };
    window.addEventListener("themechange", onThemeChange);

    let disposed = false;
    let onData: { dispose(): void } | undefined;
    stream.preview(runId, 200).then((seed) => {
      if (disposed || !seed) return;
      // tmux capture-pane pads the snapshot with blank lines up to the pane height;
      // we also used to force a trailing newline. Both rendered as a block of empty
      // lines on every open. Trim trailing blank lines — the live attach
      // stream is authoritative for the current screen.
      const trimmed = seed.replace(/[\r\n]+$/, "");
      if (trimmed) term.write(trimmed);
    });
    stream.attach(runId, (bytes) => term.write(bytes)).then(() => {
      if (disposed) return;
      onData = term.onData((d) => {
        stream.input(runId, d);
        const cb = onFirstPromptRef.current;
        if (cb && !captureRef.current.done) {
          const r = feed(captureRef.current, d);
          captureRef.current = r.state;
          if (r.line !== null) cb(r.line);
        }
      });
      // Re-send the size now that the attach exists, so the first paint matches.
      doFit();
    });

    return () => {
      disposed = true;
      ro.disconnect();
      window.removeEventListener("themechange", onThemeChange);
      onData?.dispose();
      stream.detach(runId);
      term.dispose();
      captureRef.current = initialCapture();
    };
  }, [runId, stream]);

  return <div className="terminal focus-term" ref={ref} />;
}
