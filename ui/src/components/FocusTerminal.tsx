import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { attachRun, detachRun, runInput, runPreview } from "../api";
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
    const doFit = () => { try { fit.fit(); } catch { /* not laid out */ } };
    requestAnimationFrame(() => { doFit(); term.focus(); });
    const ro = new ResizeObserver(doFit);
    ro.observe(container);

    let disposed = false;
    let onData: { dispose(): void } | undefined;
    runPreview(runId, 200).then((seed) => { if (!disposed && seed) term.write(seed.endsWith("\n") ? seed : seed + "\n"); });
    attachRun(runId, (bytes) => term.write(bytes)).then(() => {
      if (disposed) return;
      onData = term.onData((d) => runInput(runId, d));
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
