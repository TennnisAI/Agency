import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { sendInput, startTask, taskStatus, TaskInfo } from "../api";
import { xtermTheme } from "../lib/xtermTheme";

interface Props {
  projectId: string;
  prompt: string;
  profile: string;
  onStatus: (label: string) => void;
  onStarted?: (taskId: string) => void;
}

export default function TerminalPane({ projectId, prompt, profile, onStatus, onStarted }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({ convertEol: true, fontSize: 13, cursorBlink: true, theme: xtermTheme });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(container);

    // Fit once the flex layout has actually sized the container, then keep
    // fitting on resize. Fitting synchronously here (before layout) can size
    // the terminal to ~0 rows/cols and render nothing.
    const doFit = () => {
      try {
        fit.fit();
      } catch {
        /* container not laid out yet */
      }
    };
    requestAnimationFrame(() => {
      doFit();
      term.focus();
    });
    const resizeObserver = new ResizeObserver(doFit);
    resizeObserver.observe(container);

    let info: TaskInfo | null = null;
    let statusTimer: number | undefined;
    let disposed = false;
    let onDataDisposable: { dispose(): void } | undefined;

    startTask(projectId, prompt, profile, (bytes) => term.write(bytes)).then((i) => {
      if (disposed) return;
      info = i;
      onStarted?.(i.taskId);
      onDataDisposable = term.onData((data) => {
        sendInput(i.taskId, data);
      });
      statusTimer = window.setInterval(async () => {
        try {
          const s = await taskStatus(i.taskId);
          onStatus(s.code != null ? `${s.state} (${s.code})` : s.state);
        } catch {
          /* task gone */
        }
      }, 1000);
    });

    return () => {
      disposed = true;
      if (statusTimer) window.clearInterval(statusTimer);
      resizeObserver.disconnect();
      onDataDisposable?.dispose();
      term.dispose();
      void info; // session teardown handled by App via stopTask
    };
  }, [projectId, prompt, profile, onStatus, onStarted]);

  return <div className="terminal" ref={containerRef} />;
}
