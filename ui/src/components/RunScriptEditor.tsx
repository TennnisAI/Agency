import { useState } from "react";
import { RunScript, RunScriptConfig, RunSuggestion } from "../api";
import Toggle from "./Toggle";
import { deriveScriptName, uniqueScriptName } from "../lib/runScripts";

// Add or edit one of the project's run scripts. Doubles as the Run tab's empty
// state: with nothing configured yet it is the first thing the tab shows, which
// is why it carries a line about what a run script is at all.
//
// The command is the whole point, so it is the only field anyone has to fill:
// the name follows it until someone types over it, and the detected commands
// under it fill the field in a click. Nothing here is a selection, so there is
// never a pick to undo.
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
  const [command, setCommand] = useState(existing?.command ?? "");
  const [name, setName] = useState(existing?.name ?? "");
  // Until the name field is touched it mirrors the command, so the usual script
  // costs one field instead of two. Editing an existing script starts pinned:
  // its name is already what the Run tab lists and renaming is deliberate.
  const [pinned, setPinned] = useState(!!existing);
  const [web, setWeb] = useState(existing?.web ?? false);
  const [nonconcurrent, setNonconcurrent] = useState(existing?.nonconcurrent ?? false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const shownName = pinned ? name : uniqueScriptName(deriveScriptName(command), takenNames);
  const finalName = shownName.trim();
  // Only a hand-typed name can collide: the derived one is uniquified already.
  const clash = takenNames.includes(finalName);
  const valid = !!finalName && !!command.trim() && !clash;

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
    run(() => onSave({ name: finalName, command: command.trim(), web, nonconcurrent }, thenRun));

  // A detected command fills the form. It brings its own name, which reads
  // better than anything derived from the command line ("runserver", not
  // "manage"), so from here the name is the user's to change.
  const apply = (s: RunSuggestion) => {
    setCommand(s.command);
    setWeb(s.web);
    setName(uniqueScriptName(s.name, takenNames));
    setPinned(true);
  };

  const port = config.port;
  return (
    <div className="run-setup">
      <div className="run-setup-card">
        <div className="run-setup-title">
          {existing ? `Edit "${existing.name}"` : first ? "Add your first run script" : "New run script"}
        </div>
        {first && (
          <p className="run-setup-lead">
            A run script is the command that starts or builds this project. Every agent shares the
            list and runs it in its own workspace, so trying an agent's work never disturbs your
            checkout.
          </p>
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
              if (e.key === "Enter" && valid && !busy) save(true);
              if (e.key === "Escape") onCancel?.();
            }}
          />
          {!existing && config.suggestions.length > 0 && (
            <div className="run-setup-picks">
              <span className="run-setup-picks-label">Found in your project</span>
              {config.suggestions.map((s) => (
                <button
                  key={s.command}
                  className="run-setup-pick"
                  onClick={() => apply(s)}
                  title={`${s.command} (${s.detail})`}
                >
                  {s.label}
                </button>
              ))}
            </div>
          )}
          <div className="run-setup-hint">
            Runs through <code>sh -lc</code>, so pipes, env vars and <code>&amp;&amp;</code> all work.
          </div>
        </div>

        <div className="run-setup-field">
          <div className="run-setup-label">Name</div>
          <input
            className="run-setup-input run-setup-name"
            spellCheck={false}
            placeholder="dev"
            value={shownName}
            onChange={(e) => { setPinned(true); setName(e.target.value); }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && valid && !busy) save(true);
              if (e.key === "Escape") onCancel?.();
            }}
          />
          {clash && (
            <div className="run-setup-hint run-setup-clash">
              This project already has a script called "{finalName}".
            </div>
          )}
        </div>

        <div className="run-setup-row">
          <div className="run-setup-row-text">
            <div className="run-setup-row-label">Serves a web app</div>
            <div className="run-setup-hint">
              {port != null ? (
                <>
                  Gets its own port in <code>$AGENCY_PORT</code>, and the app opens beside the logs.
                  {config.previewToolsEnabled &&
                    " Dispatched agents can see and drive that preview over MCP."}
                </>
              ) : (
                <>This workspace has no port, so only the logs are shown.</>
              )}
            </div>
          </div>
          <Toggle checked={web} onChange={setWeb} />
        </div>

        <div className="run-setup-row">
          <div className="run-setup-row-text">
            <div className="run-setup-row-label">One at a time</div>
            <div className="run-setup-hint">
              Starting it stops the project's other scripts. For commands that bind a fixed port.
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
            Save
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
          <div>
            Runs in {where}: <code>{config.workspace}</code>
            {port != null && <> on port <code>{port}</code></>}.
          </div>
          <div>
            {config.shared ? (
              <>
                The list comes from <code>.agency/agency.toml</code>, shared with your team. Saving
                here keeps your version in <code>.agency/agency.local.toml</code>, on this machine
                only, and that one wins.
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
    </div>
  );
}
