import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { attachRun, detachRun, resizeRun, runInput, runPreview } from "../api";
import { xtermTheme } from "../lib/xtermTheme";

export default function FocusTerminal({ runId }: { runId: string }) {
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
        if (term.cols > 0 && term.rows > 0) resizeRun(runId, term.cols, term.rows).catch(() => {});
      } catch { /* not laid out */ }
    };
    requestAnimationFrame(() => { doFit(); term.focus(); });
    const ro = new ResizeObserver(doFit);
    ro.observe(container);

    let disposed = false;
    let onData: { dispose(): void } | undefined;
    runPreview(runId, 200).then((seed) => { if (!disposed && seed) term.write(seed.endsWith("\n") ? seed : seed + "\n"); });
    attachRun(runId, (bytes) => term.write(bytes)).then(() => {
      if (disposed) return;
      onData = term.onData((d) => runInput(runId, d));
      // Re-send the size now that the attach exists, so the first paint matches.
      doFit();
    });

    return () => {
      disposed = true;
      ro.disconnect();
      onData?.dispose();
      detachRun(runId);
      term.dispose();
    };
  }, [runId]);

  return <div className="terminal focus-term" ref={ref} />;
}
