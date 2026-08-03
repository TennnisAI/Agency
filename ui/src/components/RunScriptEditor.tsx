import { useState } from "react";
import { RunScript, RunScriptConfig } from "../api";
import Toggle from "./Toggle";

// Add or edit one of the project's run scripts. Doubles as the Run tab's empty
// state: with nothing configured yet it is the first thing the tab shows, which
// is why it carries the explanation of what a run script is at all.
export default function RunScriptEditor({
  config,
  where,
  existing,
  takenNames,
  onSave,
  onDelete,
  onCancel,
}: {
  config: RunScriptConfig;
  where: string;
  // The script being edited, or null when adding a new one.
  existing: RunScript | null;
  // Names already in the list (excluding the one being edited). A name keys the
  // script's session, so two scripts sharing one would fight over the same
  // process.
  takenNames: string[];
  onSave: (script: RunScript, thenRun: boolean) => Promise<void>;
  // Absent when adding — there is nothing to remove yet.
  onDelete?: () => Promise<void>;
  // Backs out: to the run panel when the project has scripts, otherwise to
  // whichever tab the Run tab was opened from. Absent when there is nowhere to
  // back out to — the project-level tab showing its own empty state — so the
  // button isn't offered as a no-op.
  onCancel?: () => void;
}) {
  const first = !existing && config.scripts.length === 0;
  // Prefill a new script from the best guess, so the common case is one click
  // rather than typing.
  const suggestion = existing ? null : config.suggestions[0] ?? null;
  const [name, setName] = useState(existing?.name ?? suggestion?.name ?? "");
  const [command, setCommand] = useState(existing?.command ?? suggestion?.command ?? "");
  const [web, setWeb] = useState(existing?.web ?? suggestion?.web ?? false);
  const [nonconcurrent, setNonconcurrent] = useState(existing?.nonconcurrent ?? false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const trimmedName = name.trim();
  const clash = takenNames.includes(trimmedName);
  const valid = !!trimmedName && !!command.trim() && !clash;

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const save = (thenRun: boolean) =>
    run(() =>
      onSave({ name: trimmedName, command: command.trim(), web, nonconcurrent }, thenRun),
    );

  const port = config.port;
  return (
    <div className="run-setup">
      <div className="run-setup-card">
        <div className="run-setup-title">
          {existing ? `Edit "${existing.name}"` : first ? "Set up a run script" : "Add a run script"}
        </div>
        {first && (
          <p className="run-setup-lead">
            A run script is a command for starting or building this project. Keep as many as you
            need, a dev server and a release build side by side, and pick one from the Run tab
            whenever you want it. Every agent shares the list and runs each script in its own
            workspace, <code>$AGENCY_WORKSPACE_PATH</code>, so trying an agent's changes never
            disturbs your checkout.
          </p>
        )}
        <div className="run-setup-hint">
          Runs in {where}: <code>{config.workspace}</code>
          {port != null && <> on port <code>{port}</code></>}.
        </div>

        {!existing && config.suggestions.length > 0 && (
          <div className="run-setup-field">
            <div className="run-setup-label">Found in your project</div>
            <div className="run-setup-chips">
              {config.suggestions.map((s) => (
                <button
                  key={s.command}
                  className={`run-setup-chip ${command === s.command ? "on" : ""}`}
                  onClick={() => { setName(s.name); setCommand(s.command); setWeb(s.web); }}
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
          <div className="run-setup-label">Name</div>
          <input
            className="run-setup-input run-setup-name"
            autoFocus={!!existing}
            spellCheck={false}
            placeholder="dev"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Escape") onCancel?.(); }}
          />
          <div className="run-setup-hint">
            {clash
              ? <span className="run-setup-clash">This project already has a script called "{trimmedName}".</span>
              : "What the Run tab lists it as."}
          </div>
        </div>

        <div className="run-setup-field">
          <div className="run-setup-label">Command</div>
          <input
            className="run-setup-input"
            autoFocus={!existing}
            spellCheck={false}
            placeholder={port != null ? "pnpm dev --port $AGENCY_PORT" : "pnpm dev"}
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && valid && !busy) save(true);
              if (e.key === "Escape") onCancel?.();
            }}
          />
          <div className="run-setup-hint">
            Runs through <code>sh -lc</code>, so pipes, env vars and <code>&amp;&amp;</code> all work.
          </div>
        </div>

        <div className="run-setup-row">
          <div className="run-setup-row-text">
            <div className="run-setup-label">Opens a web app</div>
            <div className="run-setup-hint">
              {port != null ? (
                <>
                  This one serves a site. It gets its own port in <code>$AGENCY_PORT</code>, and the
                  Run tab shows the app beside the logs. Leave it off for a build or a test run,
                  which have nothing to point a browser at.
                </>
              ) : (
                <>
                  This workspace has no port block, so only the logs are shown either way.
                </>
              )}
            </div>
          </div>
          <Toggle checked={web} onChange={setWeb} />
        </div>

        <div className="run-setup-row">
          <div className="run-setup-row-text">
            <div className="run-setup-label">One app at a time</div>
            <div className="run-setup-hint">
              Starting this stops every other run script in the project. Turn it on when the
              command binds a fixed port instead of <code>AGENCY_PORT</code>.
            </div>
          </div>
          <Toggle checked={nonconcurrent} onChange={setNonconcurrent} />
        </div>

        {error && <div className="run-setup-error">{error}</div>}

        <div className="run-setup-actions">
          <button className="settings-save" disabled={busy || !valid} onClick={() => save(true)}>
            Save and run
          </button>
          <button className="tile-act" disabled={busy || !valid} onClick={() => save(false)}>
            Save only
          </button>
          {onCancel && (
            <button className="tile-act" disabled={busy} onClick={onCancel}>Cancel</button>
          )}
          {onDelete && (
            <>
              <span className="spacer" />
              <button className="tile-act danger" disabled={busy} onClick={() => run(onDelete)}>
                Delete
              </button>
            </>
          )}
        </div>

        <div className="run-setup-foot">
          {config.shared ? (
            <>
              The current scripts come from <code>.agency/agency.toml</code>, shared with your
              team. Saving here stores your version in{" "}
              <code>.agency/agency.local.toml</code> (this machine only), which takes precedence.
            </>
          ) : (
            <>
              Saved to <code>.agency/agency.local.toml</code>, this machine only. Move the{" "}
              <code>[[scripts.runs]]</code> entries to <code>.agency/agency.toml</code> to share
              them with your team.
            </>
          )}
        </div>
      </div>
    </div>
  );
}
