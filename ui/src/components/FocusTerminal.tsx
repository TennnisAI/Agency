import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { attachRun, detachRun, resizeRun, runInput, runPreview,
  attachRunScript, detachRunScript, resizeRunScript, runScriptInput, runScriptPreview } from "../api";
import { xtermTheme } from "../lib/xtermTheme";

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
  { runId, stream = agentStream }: { runId: string; stream?: TerminalStream },
) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = ref.current;
    if (!container) return;
    const term = new Terminal({ convertEol: true, fontSize: 13, cursorBlink: true, theme: xtermTheme });
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

    let disposed = false;
    let onData: { dispose(): void } | undefined;
    stream.preview(runId, 200).then((seed) => { if (!disposed && seed) term.write(seed.endsWith("\n") ? seed : seed + "\n"); });
    stream.attach(runId, (bytes) => term.write(bytes)).then(() => {
      if (disposed) return;
      onData = term.onData((d) => stream.input(runId, d));
      // Re-send the size now that the attach exists, so the first paint matches.
      doFit();
    });

    return () => {
      disposed = true;
      ro.disconnect();
      onData?.dispose();
      stream.detach(runId);
      term.dispose();
    };
  }, [runId, stream]);

  return <div className="terminal focus-term" ref={ref} />;
}
