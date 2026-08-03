import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  RunScript,
  RunScriptConfig,
  SessionStatus,
  runScriptConfig,
  runScriptKey,
  runScriptsStatus,
  saveRunScripts,
  startRunScript,
  stopRunScript,
} from "../api";
import FocusTerminal, { runStream } from "./FocusTerminal";
import RunScriptEditor from "./RunScriptEditor";
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";

type Editing = { mode: "new" } | { mode: "edit"; name: string } | null;

// The Run tab: the project's run scripts, and one workspace to run them in.
//
// `target` is that workspace — an agent's run id, or `project:<id>` for the
// project's own checkout, so the same panel serves both the agent's Run tab and
// the project-level one. The scripts themselves are project-wide however they
// were reached: editing them from an agent edits them for everyone.
//
// `onClose` leaves the Run tab entirely. The setup card needs it: a project
// with no scripts has no panel to fall back to, so without it the card is a
// dead end with no way out but picking another tab. The project-level tab is
// its own destination and passes nothing.
export default function RunPanel({
  target,
  where,
  onClose,
}: {
  target: string;
  // How to describe the workspace in the setup copy, e.g. "this agent's
  // workspace" or "the project checkout".
  where: string;
  onClose?: () => void;
}) {
  const [config, setConfig] = useState<RunScriptConfig | null>(null);
  const [statuses, setStatuses] = useState<Record<string, SessionStatus>>({});
  const [selected, setSelected] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing>(null);
  // How many times each script has been started in this panel. Starting kills
  // the previous session and spawns a new one under the same name, so a pane
  // already on screen is left holding a subscription to a session that no
  // longer exists: it shows nothing further and reads as a run that hung. The
  // count rides in the terminal's key, so a start remounts the pane onto the
  // session it actually spawned.
  const [runCount, setRunCount] = useState<Record<string, number>>({});
  const [previewKey, setPreviewKey] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // The preview is the point of a web script, so it gets the larger half by
  // default and the split is remembered across sessions.
  const previewPane = usePaneWidth("run-preview", 560, 280, 1400);

  const loadConfig = useCallback(
    () => runScriptConfig(target).then(setConfig).catch((e) => setError(String(e))),
    [target],
  );

  useEffect(() => {
    setConfig(null);
    setStatuses({});
    setSelected(null);
    setEditing(null);
    setRunCount({});
    setError(null);
    loadConfig();
  }, [target, loadConfig]);

  // Starting and stopping change the answer there and then, so both refresh
  // rather than wait out the poll. `seq` keeps the newest request the one that
  // wins: a reply already in flight when the click landed describes the state
  // before it, and would otherwise put the stale answer back for a poll.
  const seq = useRef(0);
  const refresh = useCallback(async () => {
    const mine = ++seq.current;
    try {
      const list = await runScriptsStatus(target);
      if (mine === seq.current) {
        setStatuses(Object.fromEntries(list.map((s) => [s.name, s.status])));
      }
    } catch { /* transient; the next poll asks again */ }
  }, [target]);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 1500);
    // Bumping the seq drops whatever is in flight: on a target change it
    // describes the workspace we just left, and on unmount nobody wants it.
    return () => { seq.current++; clearInterval(t); };
  }, [refresh]);

  const scripts = useMemo(() => config?.scripts ?? [], [config]);

  // Keep the selection on a script that still exists — a rename or a delete
  // otherwise leaves the panel pointed at nothing.
  useEffect(() => {
    if (!scripts.length) return;
    setSelected((cur) => (cur && scripts.some((s) => s.name === cur) ? cur : scripts[0].name));
  }, [scripts]);

  const script = scripts.find((s) => s.name === selected) ?? null;
  const status = script ? statuses[script.name] : undefined;
  const running = status?.state === "running";
  const url = config?.port != null ? `http://localhost:${config.port}` : null;

  const start = async (name: string) => {
    setError(null);
    try {
      await startRunScript(target, name);
      setRunCount((c) => ({ ...c, [name]: (c[name] ?? 0) + 1 }));
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  // Stop a running script, and clear a finished one: both are the same call,
  // which takes the session down and with it everything the panel shows for
  // that script. A finished script leaves its session behind precisely so its
  // output survives; this is how you say you are done reading it.
  const stop = async (name: string) => {
    setError(null);
    try {
      await stopRunScript(target, name);
      await refresh();
    } catch (e) {
      // Stop failed: the script is still running, so leave the pane as it is.
      setError(String(e));
    }
  };

  // Insert or replace one script in the project's list, keeping the order the
  // user set. `replacing` is the name the entry had before the edit, so a
  // rename lands in place instead of appending a second copy.
  const saveScript = async (next: RunScript, replacing: string | null, thenRun: boolean) => {
    const list = replacing
      ? scripts.map((s) => (s.name === replacing ? next : s))
      : [...scripts, next];
    await saveRunScripts(target, list);
    // A rename leaves the old session running under the old name; nothing can
    // reach it any more, so take it down with the edit.
    if (replacing && replacing !== next.name) await stopRunScript(target, replacing).catch(() => {});
    await loadConfig();
    setSelected(next.name);
    setEditing(null);
    if (thenRun) await start(next.name);
  };

  const deleteScript = async (name: string) => {
    await stopRunScript(target, name).catch(() => {});
    await saveRunScripts(target, scripts.filter((s) => s.name !== name));
    await loadConfig();
    setSelected(null);
    setEditing(null);
  };

  // Config hasn't arrived yet: render nothing rather than flashing the setup
  // card at someone who already has run scripts.
  if (!config) return <div className="run-panel" />;

  const editorFor = (e: Exclude<Editing, null>) => (
    <RunScriptEditor
      config={config}
      where={where}
      existing={e.mode === "edit" ? scripts.find((s) => s.name === e.name) ?? null : null}
      takenNames={scripts
        .map((s) => s.name)
        .filter((n) => !(e.mode === "edit" && n === e.name))}
      onSave={(next, thenRun) => saveScript(next, e.mode === "edit" ? e.name : null, thenRun)}
      onDelete={e.mode === "edit" ? () => deleteScript(e.name) : undefined}
      // Nothing to return to: the project's Run tab with no scripts yet is
      // itself the destination, so the editor drops Cancel rather than
      // offering a button that goes nowhere.
      onCancel={
        scripts.length ? () => setEditing(null) : onClose ? () => onClose() : undefined
      }
    />
  );

  if (!scripts.length) {
    return <div className="run-panel">{editorFor({ mode: "new" })}</div>;
  }
  if (editing) {
    return <div className="run-panel">{editorFor(editing)}</div>;
  }

  // Whether this script has anything to show: it is running, or it ran and left
  // its output behind. Anything else — never started here, or cleared — has no
  // logs to fill the pane with, and shows the script's start card instead.
  const hasLogs = !!script && (running || status?.state === "exited");
  const showPreview = !!script?.web && !!url && running;

  return (
    <div className="run-panel">
      <div className="run-scripts">
        {scripts.map((s) => {
          const st = statuses[s.name];
          return (
            <button
              key={s.name}
              className={`run-script-tab ${s.name === selected ? "on" : ""}`}
              title={s.command}
              onClick={() => setSelected(s.name)}
            >
              <span className={`dot ${st?.state === "running" ? "running" : "exited"}`} />
              {s.name}
            </button>
          );
        })}
        <button
          className="run-script-add"
          title="Add another run script to this project"
          onClick={() => setEditing({ mode: "new" })}
        >+</button>
        <span className="spacer" />
        <span className="run-where" title={config.workspace}>{where}</span>
      </div>

      {error && <div className="run-error">{error}</div>}

      {/* The toolbar drives a script that is already on screen. A script with
          nothing to show carries its own start button in the body, so Start is
          never offered twice at once. */}
      {script && hasLogs && (
        <div className="run-controls">
          {running ? (
            <button className="tile-act" onClick={() => stop(script.name)}>■ Stop</button>
          ) : (
            <button className="tile-act" onClick={() => start(script.name)}>▶ Start</button>
          )}
          <code className="run-cmd" title={script.command}>{script.command}</code>
          {status?.state === "exited" && (
            <>
              <span className={`run-exit ${status.code === 0 ? "" : "bad"}`}>
                {status.code === 0 ? "finished" : `exited ${status.code}`}
              </span>
              <button
                className="tile-act"
                title="Clear this finished run and its output"
                onClick={() => stop(script.name)}
              >✕ Clear</button>
            </>
          )}
          {script.web && url && (
            <>
              <code className="run-url">{url}</code>
              <button
                className="tile-act"
                title="Reload the preview"
                onClick={() => setPreviewKey((k) => k + 1)}
              >⟳ Reload</button>
              <button className="tile-act" title={`Open ${url} in your browser`} onClick={() => openUrl(url)}>
                Open ↗
              </button>
            </>
          )}
          <button className="tile-act" onClick={() => setEditing({ mode: "edit", name: script.name })}>
            Edit
          </button>
        </div>
      )}

      <div className="run-body">
        {script && hasLogs ? (
          <div className="run-term">
            <FocusTerminal
              key={`${runScriptKey(target, script.name)}:${runCount[script.name] ?? 0}`}
              runId={runScriptKey(target, script.name)}
              stream={runStream}
            />
          </div>
        ) : script ? (
          <div className="run-idle">
            <button className="run-start" onClick={() => start(script.name)}>▶ Start</button>
            <code className="run-idle-cmd" title={script.command}>{script.command}</code>
            {script.web && url && (
              <div className="run-idle-note">Serves a web app; the preview opens beside the logs.</div>
            )}
            <button
              className="run-idle-edit"
              onClick={() => setEditing({ mode: "edit", name: script.name })}
            >Edit this script</button>
          </div>
        ) : null}
        {showPreview && (
          <>
            <Resizer
              size={previewPane.width}
              min={280}
              max={1400}
              side="right"
              onChange={previewPane.setWidth}
            />
            <iframe
              key={previewKey}
              className="run-preview"
              style={{ width: previewPane.width, flex: "0 0 auto" }}
              src={url!}
              title={`${script!.name} preview`}
            />
          </>
        )}
      </div>
    </div>
  );
}
