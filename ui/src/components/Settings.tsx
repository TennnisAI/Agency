import { useEffect, useState } from "react";
import {
  AgentProfile,
  ProviderSettings,
  NotifSettings,
  deleteProfile,
  getSettings,
  getNotifSettings,
  listProfiles,
  saveProfile,
  saveSettings,
  saveNotifSettings,
} from "../api";
import Toggle from "./Toggle";
import { THEMES, ThemeId, applyTheme, getStoredTheme } from "../lib/themes";

export default function Settings({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<ProviderSettings>({
    anthropicApiKey: "",
    lmStudioBaseUrl: "",
  });
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [draft, setDraft] = useState({ name: "", command: "", args: "", env: "" });
  const [error, setError] = useState("");
  const [notif, setNotif] = useState<NotifSettings>({
    agentFinished: true,
    agentIdle: true,
    runCrashed: true,
    mergeAttention: true,
    onlyWhenUnfocused: true,
    idleSecs: 30,
  });
  const [themeId, setThemeId] = useState<ThemeId>(getStoredTheme());

  function pickTheme(id: ThemeId) {
    setThemeId(id);
    applyTheme(id);
  }

  async function refresh() {
    try {
      setSettings(await getSettings());
      setProfiles(await listProfiles());
      setNotif(await getNotifSettings());
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

  async function persistNotif(next: NotifSettings) {
    setNotif(next);
    try {
      await saveNotifSettings(next);
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
          <button className="icon-btn settings-close" onClick={onClose}>✕</button>
        </div>
        {error && <div className="git-error">{error}</div>}

        <section className="settings-section">
          <div className="settings-section-label">Appearance</div>
          <div className="settings-theme-grid">
            {THEMES.map((t) => (
              <button
                key={t.id}
                type="button"
                className="settings-theme-card"
                data-active={t.id === themeId}
                onClick={() => pickTheme(t.id)}
              >
                <span className="settings-theme-name">{t.label}</span>
                <span className="settings-theme-swatches">
                  {([t.vars.base, t.vars.s1, t.vars.text, t.vars.green, t.vars.yellow, t.vars.red] as const).map(
                    (c, i) => (
                      <span key={i} className="settings-theme-dot" style={{ background: c }} />
                    ),
                  )}
                </span>
              </button>
            ))}
          </div>
        </section>

        <section className="settings-section">
          <div className="settings-section-label">Agent profiles</div>
          <div className="settings-card-list">
            {profiles.map((p) => (
              <div key={p.name} className="settings-profile-card">
                <div className="settings-profile-head">
                  <span className="settings-profile-name">{p.name}</span>
                  <span className="spacer" />
                  <button className="settings-ghost-btn" onClick={() => editProfile(p)}>Edit</button>
                  <button className="settings-ghost-btn settings-del-btn" onClick={() => deleteProfile(p.name).then(refresh)}>Delete</button>
                </div>
                <div className="settings-profile-meta">
                  <span className="settings-meta-key">command</span>
                  <code className="settings-meta-val">{p.command}</code>
                  {p.args.length > 0 && (
                    <>
                      <span className="settings-meta-key">args</span>
                      <code className="settings-meta-val">{p.args.join(" ")}</code>
                    </>
                  )}
                  {p.env.length > 0 && (
                    <>
                      <span className="settings-meta-key">env</span>
                      <code className="settings-meta-val">{p.env.map(([k]) => k).join(", ")}</code>
                    </>
                  )}
                </div>
              </div>
            ))}
          </div>
          <div className="profile-form">
            <div className="settings-form-label">Add / edit profile</div>
            <input
              className="settings-input"
              placeholder="name"
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
            <input
              className="settings-input"
              placeholder="command (e.g. claude)"
              value={draft.command}
              onChange={(e) => setDraft({ ...draft, command: e.target.value })}
            />
            <input
              className="settings-input"
              placeholder="args (space-separated, use {{prompt}})"
              value={draft.args}
              onChange={(e) => setDraft({ ...draft, args: e.target.value })}
            />
            <textarea
              className="settings-input"
              placeholder="env, one KEY=VALUE per line"
              value={draft.env}
              onChange={(e) => setDraft({ ...draft, env: e.target.value })}
            />
            <button onClick={addProfile}>Save profile</button>
          </div>
        </section>

        <section className="settings-section">
          <div className="settings-section-label">Model providers</div>
          <div className="settings-providers">
            <div className="settings-provider-card">
              <div className="settings-provider-title">
                Anthropic <span className="settings-provider-sub">· Claude</span>
              </div>
              <div className="settings-provider-field">
                <label className="settings-field-key">API key</label>
                <input
                  className="settings-field-input"
                  type="password"
                  value={settings.anthropicApiKey}
                  onChange={(e) => setSettings({ ...settings, anthropicApiKey: e.target.value })}
                />
              </div>
            </div>
            <div className="settings-provider-card">
              <div className="settings-provider-title">
                LM Studio <span className="settings-provider-sub">· local, OpenAI-compatible</span>
              </div>
              <div className="settings-provider-field">
                <label className="settings-field-key">base URL</label>
                <input
                  className="settings-field-input"
                  value={settings.lmStudioBaseUrl}
                  onChange={(e) => setSettings({ ...settings, lmStudioBaseUrl: e.target.value })}
                />
              </div>
            </div>
          </div>
          <button onClick={persistSettings}>Save providers</button>
        </section>

        <section className="settings-section">
          <div className="settings-section-label">Notifications</div>
          <div className="settings-notif">
            {([
              ["agentFinished", "Agent finished"],
              ["agentIdle", "Agent needs input / idle"],
              ["runCrashed", "Run script crashed"],
              ["mergeAttention", "Merge needs attention"],
              ["onlyWhenUnfocused", "Only when app is not focused"],
            ] as [keyof NotifSettings, string][]).map(([key, label]) => (
              <div key={key} className="settings-notif-row">
                <span className="settings-notif-label">{label}</span>
                <Toggle
                  checked={notif[key] as boolean}
                  onChange={(next) => persistNotif({ ...notif, [key]: next })}
                />
              </div>
            ))}
            <div className="settings-notif-row">
              <span className="settings-notif-label">Idle after (seconds)</span>
              <input
                className="settings-input settings-notif-secs"
                type="number"
                min={5}
                value={notif.idleSecs}
                onChange={(e) => persistNotif({ ...notif, idleSecs: Number(e.target.value) || 30 })}
              />
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}
