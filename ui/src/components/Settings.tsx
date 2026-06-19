import { useEffect, useState } from "react";
import {
  AgentProfile,
  ProviderSettings,
  deleteProfile,
  getSettings,
  listProfiles,
  saveProfile,
  saveSettings,
} from "../api";

export default function Settings({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<ProviderSettings>({
    anthropicApiKey: "",
    lmStudioBaseUrl: "",
  });
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [draft, setDraft] = useState({ name: "", command: "", args: "", env: "" });
  const [error, setError] = useState("");

  async function refresh() {
    try {
      setSettings(await getSettings());
      setProfiles(await listProfiles());
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  async function persistSettings() {
    try {
      await saveSettings(settings);
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }

  async function addProfile() {
    if (!draft.name.trim() || !draft.command.trim()) return;
    const args = draft.args.trim() ? draft.args.trim().split(/\s+/) : [];
    const env: [string, string][] = draft.env
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean)
      .map((l) => {
        const i = l.indexOf("=");
        return [l.slice(0, i), l.slice(i + 1)] as [string, string];
      })
      .filter(([k]) => k);
    try {
      await saveProfile({ name: draft.name.trim(), command: draft.command.trim(), args, env });
      setDraft({ name: "", command: "", args: "", env: "" });
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  function editProfile(p: AgentProfile) {
    setDraft({
      name: p.name,
      command: p.command,
      args: p.args.join(" "),
      env: p.env.map(([k, v]) => `${k}=${v}`).join("\n"),
    });
  }

  return (
    <div className="settings-overlay">
      <div className="settings">
        <div className="settings-head">
          <h2>Settings</h2>
          <button onClick={onClose}>Close</button>
        </div>
        {error && <div className="git-error">{error}</div>}

        <section>
          <h3>Providers</h3>
          <label>Anthropic API key</label>
          <input
            type="password"
            value={settings.anthropicApiKey}
            onChange={(e) => setSettings({ ...settings, anthropicApiKey: e.target.value })}
          />
          <label>LM Studio base URL</label>
          <input
            value={settings.lmStudioBaseUrl}
            onChange={(e) => setSettings({ ...settings, lmStudioBaseUrl: e.target.value })}
          />
          <button onClick={persistSettings}>Save providers</button>
        </section>

        <section>
          <h3>Agent profiles</h3>
          <ul className="profile-list">
            {profiles.map((p) => (
              <li key={p.name}>
                <span className="profile-name" onClick={() => editProfile(p)}>
                  {p.name}
                </span>
                <code>{p.command}</code>
                <button onClick={() => deleteProfile(p.name).then(refresh)}>Delete</button>
              </li>
            ))}
          </ul>
          <div className="profile-form">
            <input
              placeholder="name"
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
            <input
              placeholder="command (e.g. claude)"
              value={draft.command}
              onChange={(e) => setDraft({ ...draft, command: e.target.value })}
            />
            <input
              placeholder="args (space-separated, use {{prompt}})"
              value={draft.args}
              onChange={(e) => setDraft({ ...draft, args: e.target.value })}
            />
            <textarea
              placeholder="env, one KEY=VALUE per line"
              value={draft.env}
              onChange={(e) => setDraft({ ...draft, env: e.target.value })}
            />
            <button onClick={addProfile}>Save profile</button>
          </div>
        </section>
      </div>
    </div>
  );
}
