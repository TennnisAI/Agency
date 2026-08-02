import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  RunInfo,
  RunScriptConfig,
  SessionStatus,
  runScriptConfig,
  runScriptStatus,
  saveRunScript,
  startRunScript,
  stopRunScript,
} from "../api";
import FocusTerminal, { runStream } from "./FocusTerminal";
import RunScriptSetup from "./RunScriptSetup";

export default function RunPanel({ run }: { run: RunInfo }) {
  const [status, setStatus] = useState<SessionStatus>({ state: "gone" });
  const [config, setConfig] = useState<RunScriptConfig | null>(null);
  // Setup is forced open while unconfigured, and opened on demand by "Edit"
  // once a command exists.
  const [editing, setEditing] = useState(false);
  const [started, setStarted] = useState(false);
  const [previewKey, setPreviewKey] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const url = run.port != null ? `http://localhost:${run.port}` : null;
  const running = status.state === "running";

  const loadConfig = useCallback(
    () => runScriptConfig(run.id).then(setConfig).catch((e) => setError(String(e))),
    [run.id],
  );

  useEffect(() => {
    setStarted(false);
    setEditing(false);
    setError(null);
    setConfig(null);
    loadConfig();
  }, [run.id, loadConfig]);

  useEffect(() => {
    let alive = true;
    const tick = () => runScriptStatus(run.id).then((s) => { if (alive) setStatus(s); }).catch(() => {});
    tick();
    const t = setInterval(tick, 1500);
    return () => { alive = false; clearInterval(t); };
  }, [run.id]);

  const start = async () => {
    setError(null);
    try {
      await startRunScript(run.id);
      setStarted(true);
    } catch (e) {
      setError(String(e));
    }
  };
  const stop = async () => {
    setError(null);
    try {
      await stopRunScript(run.id);
      setStarted(false);
    } catch (e) {
      // Stop failed: the script is still running, so `started` must stay true.
      setError(String(e));
    }
  };

  // Save from the setup card, then optionally start straight away — the point
  // of the card is that clicking Run leads somewhere, not to a dead end.
  const save = async (command: string, nonconcurrent: boolean, thenRun: boolean) => {
    await saveRunScript(run.id, command.trim() || null, nonconcurrent);
    await loadConfig();
    setEditing(false);
    if (thenRun && command.trim()) await start();
  };

  // Config hasn't arrived yet: render nothing rather than flashing the setup
  // card at someone who already has a run script.
  if (!config) return <div className="run-panel" />;

  if (!config.command || editing) {
    return (
      <div className="run-panel">
        <RunScriptSetup
          config={config}
          onSave={save}
          onCancel={config.command ? () => setEditing(false) : undefined}
        />
      </div>
    );
  }

  return (
    <div className="run-panel">
      <div className="run-controls">
        <span className={`dot ${running ? "running" : "exited"}`} />
        {running ? (
          <button className="tile-act" onClick={stop}>■ Stop</button>
        ) : (
          <button className="tile-act" onClick={start}>▶ Run</button>
        )}
        <code className="run-cmd" title={config.command}>{config.command}</code>
        <button className="tile-act" onClick={() => setEditing(true)}>Edit</button>
        {url && (
          <>
            <code className="run-url">{url}</code>
            <button className="tile-act" onClick={() => setPreviewKey((k) => k + 1)}>⟳ Refresh</button>
            <button className="tile-act" onClick={() => openUrl(url)}>Open in browser ↗</button>
          </>
        )}
        {error && <span className="run-error">{error}</span>}
      </div>
      <div className="run-body">
        {(running || started) && (
          <div className="run-term">
            <FocusTerminal key={`run-${run.id}`} runId={run.id} stream={runStream} />
          </div>
        )}
        {url && running && (
          <iframe
            key={previewKey}
            className="run-preview"
            src={url}
            title="agent app preview"
          />
        )}
      </div>
    </div>
  );
}
