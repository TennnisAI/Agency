import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { RunInfo, SessionStatus, runScriptStatus, runScriptConfigured, startRunScript, stopRunScript } from "../api";
import FocusTerminal, { runStream } from "./FocusTerminal";

export default function RunPanel({ run }: { run: RunInfo }) {
  const [status, setStatus] = useState<SessionStatus>({ state: "gone" });
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [started, setStarted] = useState(false);
  const [previewKey, setPreviewKey] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const url = run.port != null ? `http://localhost:${run.port}` : null;
  const running = status.state === "running";

  useEffect(() => {
    setStarted(false);
    setError(null);
    runScriptConfigured(run.id).then(setConfigured).catch(() => setConfigured(false));
  }, [run.id]);

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
    await stopRunScript(run.id);
    setStarted(false);
  };

  if (configured === false) {
    return (
      <div className="run-panel empty">
        No run script configured. Add a <code>[scripts]</code> <code>run</code> line to
        <code> .agency/agency.toml</code> to launch this workspace's app.
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
            title="workspace preview"
          />
        )}
      </div>
    </div>
  );
}
