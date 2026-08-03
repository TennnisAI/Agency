import { useState } from "react";
import { RunScriptConfig } from "../api";
import Toggle from "./Toggle";

// The Run tab's setup card: what a run script is, what the project looks like
// it needs, and a field to save one. Shown whenever no run command is
// configured, and on demand behind "Edit" once there is one.
export default function RunScriptSetup({
  config,
  onSave,
  onCancel,
}: {
  config: RunScriptConfig;
  onSave: (command: string, nonconcurrent: boolean, thenRun: boolean) => Promise<void>;
  // Backs out of the card: to the run panel when a command already exists,
  // otherwise to whichever tab the Run tab was opened from.
  onCancel: () => void;
}) {
  // Prefill with the best guess so the common case is one click, not typing.
  const [command, setCommand] = useState(config.command ?? config.suggestions[0]?.command ?? "");
  const [nonconcurrent, setNonconcurrent] = useState(config.nonconcurrent);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async (thenRun: boolean) => {
    setSaving(true);
    setError(null);
    try {
      await onSave(command, nonconcurrent, thenRun);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const port = config.port;
  return (
    <div className="run-setup">
      <div className="run-setup-card">
        <div className="run-setup-title">
          {config.command ? "Edit the project's run script" : "Set up the project's run script"}
        </div>
        <p className="run-setup-lead">
          One command for the whole project: every agent here uses it to start your app. Each agent
          runs it in its own workspace, <code>$AGENCY_WORKSPACE_PATH</code>, so trying an agent's
          changes never disturbs your checkout.
          {port != null ? (
            <>
              {" "}Each one also gets its own port in <code>$AGENCY_PORT</code>. Use that variable
              in the command and the preview pane opens the app beside the logs.
            </>
          ) : (
            <> This agent has no port block, so only the logs are shown.</>
          )}
        </p>
        <div className="run-setup-hint">
          For this agent that is <code>{config.workspace}</code>
          {port != null && <> on port <code>{port}</code></>}.
        </div>

        {config.suggestions.length > 0 && (
          <div className="run-setup-field">
            <div className="run-setup-label">Found in your project</div>
            <div className="run-setup-chips">
              {config.suggestions.map((s) => (
                <button
                  key={s.command}
                  className={`run-setup-chip ${command === s.command ? "on" : ""}`}
                  onClick={() => setCommand(s.command)}
                  title={s.command}
                >
                  <span className="run-setup-chip-label">{s.label}</span>
                  <span className="run-setup-chip-detail">{s.detail}</span>
                </button>
              ))}
            </div>
          </div>
        )}

        <div className="run-setup-field">
          <div className="run-setup-label">Command</div>
          <input
            className="run-setup-input"
            autoFocus
            spellCheck={false}
            placeholder={port != null ? "pnpm dev --port $AGENCY_PORT" : "pnpm dev"}
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && command.trim() && !saving) save(true);
              if (e.key === "Escape") onCancel();
            }}
          />
          <div className="run-setup-hint">
            Runs through <code>sh -lc</code>, so pipes, env vars and <code>&amp;&amp;</code> all work.
          </div>
        </div>

        <div className="run-setup-row">
          <div className="run-setup-row-text">
            <div className="run-setup-label">One app at a time</div>
            <div className="run-setup-hint">
              Starting an app stops every other agent's. Turn this on when your app binds a fixed
              port instead of <code>AGENCY_PORT</code>.
            </div>
          </div>
          <Toggle checked={nonconcurrent} onChange={setNonconcurrent} />
        </div>

        {error && <div className="run-setup-error">{error}</div>}

        <div className="run-setup-actions">
          <button className="settings-save" disabled={saving || !command.trim()} onClick={() => save(true)}>
            Save and run
          </button>
          <button className="tile-act" disabled={saving} onClick={() => save(false)}>
            Save only
          </button>
          <button className="tile-act" disabled={saving} onClick={onCancel}>Cancel</button>
        </div>

        <div className="run-setup-foot">
          {config.shared ? (
            <>
              The current command comes from <code>.agency/agency.toml</code>, shared with your
              team. Saving here stores your version in{" "}
              <code>.agency/agency.local.toml</code> (this machine only), which takes precedence.
            </>
          ) : (
            <>
              Saved to <code>.agency/agency.local.toml</code>, this machine only. Move the{" "}
              <code>[scripts]</code> <code>run</code> line to <code>.agency/agency.toml</code> to
              share it with your team.
            </>
          )}
        </div>
      </div>
    </div>
  );
}
