import { useEffect, useRef, useState } from "react";
import {
  CatalogEntry,
  completeAgentOnboarding,
  listAgentCatalog,
  listProfiles,
  saveProfile,
} from "../api";
import { INSTALL_COMMANDS, agentColor, agentLabel } from "../agents";
import { toastError } from "../lib/toast";

// First-run agent picker: choose which catalog profiles to enable, optionally
// add a custom one, and copy install commands for missing CLIs. Completing
// writes the selection and marks onboarding done.
export default function AgentOnboarding({ onDone }: { onDone: () => void }) {
  const [catalog, setCatalog] = useState<CatalogEntry[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [customSaved, setCustomSaved] = useState(false);
  const [showCustom, setShowCustom] = useState(false);
  const [draft, setDraft] = useState({ name: "", command: "", args: "" });
  const [installFor, setInstallFor] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | undefined>(undefined);

  // Clear any pending copy-reset timer on unmount so it can't fire setCopied
  // after the component is gone.
  useEffect(() => () => window.clearTimeout(copyTimer.current), []);

  async function load() {
    try {
      const entries = await listAgentCatalog();
      setCatalog(entries);
      // Pre-select agents already on PATH so a machine with CLIs installed
      // lands on a sensible default set.
      setSelected(new Set(entries.filter((e) => e.installed).map((e) => e.id)));
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    load();
  }, []);

  function toggle(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function saveCustom() {
    const name = draft.name.trim();
    const command = draft.command.trim();
    if (!name || !command) return;
    const args = draft.args.trim() ? draft.args.trim().split(/\s+/) : [];
    try {
      await saveProfile({
        name,
        command,
        args,
        env: [],
        resume_args: null,
        loop_args: null,
      });
      setCustomSaved(true);
      setShowCustom(false);
      setDraft({ name: "", command: "", args: "" });
    } catch (e) {
      toastError(e, "Couldn't save custom profile");
    }
  }

  async function continueOnboarding() {
    if (busy) return;
    const ids = [...selected];
    if (ids.length === 0 && !customSaved) {
      // Custom may have been saved before this render — re-check profiles.
      try {
        const profiles = await listProfiles();
        if (profiles.length === 0) {
          setError("Select at least one agent, or add a custom profile.");
          return;
        }
      } catch {
        setError("Select at least one agent, or add a custom profile.");
        return;
      }
    }
    setBusy(true);
    setError("");
    try {
      await completeAgentOnboarding(ids);
      onDone();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  async function copyInstall(id: string) {
    const cmd = INSTALL_COMMANDS[id];
    if (!cmd) return;
    try {
      await navigator.clipboard.writeText(cmd);
      setCopied(true);
      window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      toastError(e, "Couldn't copy to clipboard");
    }
  }

  if (!catalog) {
    return (
      <div className="onboarding">
        <div className="onboarding-card">
          <p className="onboarding-lede">Loading agents…</p>
          {error && <div className="git-error">{error}</div>}
        </div>
      </div>
    );
  }

  const canContinue = selected.size > 0 || customSaved;
  const installCmd = installFor ? INSTALL_COMMANDS[installFor] : null;

  return (
    <div className="onboarding">
      <div className="onboarding-card">
        <div className="onboarding-mark">▦</div>
        <h1>Choose your agents</h1>
        <p className="onboarding-lede">
          Pick which coding agents to enable. You can add more later in Settings,
          including any you skip now.
        </p>

        <ul className="onboarding-list">
          {catalog.map((entry) => {
            const checked = selected.has(entry.id);
            const install = INSTALL_COMMANDS[entry.id];
            return (
              <li key={entry.id} className={`onboarding-row${checked ? " is-selected" : ""}`}>
                <label className="onboarding-check">
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={() => toggle(entry.id)}
                  />
                  <span className="agent-dot" style={{ background: agentColor(entry.id) }} />
                  <span className="onboarding-name">{agentLabel(entry.id)}</span>
                  <code className="onboarding-cmd">{entry.command}</code>
                </label>
                <span className="spacer" />
                {entry.installed ? (
                  <span className="onboarding-status is-ok">Installed</span>
                ) : install ? (
                  <button
                    type="button"
                    className="settings-ghost-btn"
                    onClick={() => setInstallFor(entry.id)}
                  >
                    Install
                  </button>
                ) : (
                  <span className="onboarding-status">Not on PATH</span>
                )}
              </li>
            );
          })}
        </ul>

        {showCustom ? (
          <div className="onboarding-custom">
            <div className="settings-form-label">Custom agent</div>
            <input
              className="settings-input"
              placeholder="name"
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
            <input
              className="settings-input"
              placeholder="command"
              value={draft.command}
              onChange={(e) => setDraft({ ...draft, command: e.target.value })}
            />
            <input
              className="settings-input"
              placeholder="args (optional, use {{prompt}})"
              value={draft.args}
              onChange={(e) => setDraft({ ...draft, args: e.target.value })}
            />
            <div className="row-actions">
              <button type="button" className="btn-primary" onClick={saveCustom}>
                Save custom
              </button>
              <button type="button" className="btn-secondary" onClick={() => setShowCustom(false)}>
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <button
            type="button"
            className="settings-add-profile"
            onClick={() => setShowCustom(true)}
          >
            + Add custom agent
          </button>
        )}

        {customSaved && (
          <p className="onboarding-custom-ok">Custom profile saved. It will appear in your agent list.</p>
        )}

        {error && <div className="git-error">{error}</div>}

        <div className="onboarding-actions">
          <button
            type="button"
            className="btn-primary"
            disabled={!canContinue || busy}
            onClick={continueOnboarding}
          >
            {busy ? "Saving…" : "Continue"}
          </button>
        </div>
      </div>

      {installFor && (
        <div
          className="modal-backdrop"
          onClick={() => setInstallFor(null)}
        >
          <div
            className="modal confirm"
            role="dialog"
            aria-modal="true"
            aria-label={`Install ${agentLabel(installFor)}`}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-head">
              <h3>Install {agentLabel(installFor)}</h3>
              <button className="modal-x" onClick={() => setInstallFor(null)}>✕</button>
            </div>
            <div className="modal-body">
              <p className="install-lede">
                Run this in a terminal to install <code>{catalog.find((e) => e.id === installFor)?.command}</code>,
                then come back and continue. After you add a project, Agency can also
                run installs for you when spawning an agent.
              </p>
              {installCmd ? (
                <pre className="install-cmd">{installCmd}</pre>
              ) : (
                <p>Install it manually and point the profile at the right command in Settings.</p>
              )}
            </div>
            <div className="modal-foot">
              <button className="btn-secondary" onClick={() => setInstallFor(null)}>Close</button>
              {installCmd && (
                <button
                  className="btn-primary"
                  onClick={() => copyInstall(installFor)}
                >
                  {copied ? "Copied" : "Copy command"}
                </button>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
