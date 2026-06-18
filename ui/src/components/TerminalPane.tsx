import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { sendInput, startTask, taskStatus, TaskInfo } from "../api";

interface Props {
  projectId: string;
  prompt: string;
  onStatus: (label: string) => void;
}

export default function TerminalPane({ projectId, prompt, onStatus }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const term = new Terminal({ convertEol: true, fontSize: 13 });
    const fit = new FitAddon();
    term.loadAddon(fit);
    if (containerRef.current) {
      term.open(containerRef.current);
      fit.fit();
    }

    let info: TaskInfo | null = null;
    let statusTimer: number | undefined;
    let disposed = false;

    startTask(projectId, prompt, "shell", (bytes) => term.write(bytes)).then((i) => {
      if (disposed) return;
      info = i;
      term.onData((data) => {
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
      term.dispose();
      void info; // session teardown handled by App via stopTask
    };
  }, [projectId, prompt, onStatus]);

  return <div className="terminal" ref={containerRef} />;
}
