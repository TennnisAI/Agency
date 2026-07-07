import { Fragment, useEffect, useRef, useState } from "react";
import {
  AgentProfile,
  KnowledgeConfig,
  McpServer,
  ProviderSettings,
  NotifSettings,
  deleteProfile,
  getKnowledgeConfig,
  getSettings,
  getNotifSettings,
  listMcpServers,
  listProfiles,
  saveKnowledgeConfig,
  saveMcpServers,
  saveProfile,
  saveSettings,
  saveNotifSettings,
} from "../api";
import Toggle from "./Toggle";
import { agentColor, agentLabel } from "../agents";
import { THEMES, ThemeId, applyTheme, getStoredTheme } from "../lib/themes";
import { getWordWrap, setWordWrap } from "../lib/editorPrefs";

// Full-view settings page (design handoff: settings takes over the main area,
// entered from the ⚙ button at the bottom of the Projects pane).
export default function Settings({
  onClose,
  projectId,
  projectName,
}: {
  onClose: () => void;
  projectId: string | null;
  projectName: string | null;
}) {
  const [settings, setSettings] = useState<ProviderSettings>({
    lmStudioBaseUrl: "",
  });
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const emptyDraft = { name: "", command: "", args: "", env: "", resume: "", loop: "" };
  const [draft, setDraft] = useState(emptyDraft);
  const [formOpen, setFormOpen] = useState(false);
  // Name of the profile being edited, or null when adding a new one. Drives
  // where the form renders: inline under the edited card, else at the bottom.
  const [editing, setEditing] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [notif, setNotif] = useState<NotifSettings>({
    agentFinished: true,
    agentIdle: true,
    runCrashed: true,
    mergeAttention: true,
    loopEvents: true,
    onlyWhenUnfocused: true,
    idleSecs: 30,
  });
  const [themeId, setThemeId] = useState<ThemeId>(getStoredTheme());
  const [wordWrap, setWrap] = useState<boolean>(getWordWrap());
  const [mcpServers, setMcpServers] = useState<McpServer[]>([]);
  const emptyMcpDraft = { name: "", command: "", args: "", env: "", url: "" };
  const [mcpDraft, setMcpDraft] = useState(emptyMcpDraft);
  const [mcpFormOpen, setMcpFormOpen] = useState(false);
  // Per-project knowledge-graph config. `null` until loaded (or when no project
  // is selected — the section then prompts to pick one). `kgDraft` holds the
  // editable command-override text so it survives re-renders between saves.
  const [kg, setKg] = useState<KnowledgeConfig | null>(null);
  const [kgDraft, setKgDraft] = useState({ serve: "", build: "" });

  function pickWordWrap(on: boolean) {
    setWrap(on);
    setWordWrap(on);
  }

  function pickTheme(id: ThemeId) {
    setThemeId(id);
    applyTheme(id);
  }

  // The local-model base URL is edited into a local form rather than saved on
  // each change. To honour "settings autosave", we flush it when Settings
  // unmounts (Back, or navigating to a project/run). `loadedRef` holds the
  // last-persisted baseline so the flush only writes when something actually
  // changed and only after the initial load.
  const settingsRef = useRef(settings);
  useEffect(() => { settingsRef.current = settings; }, [settings]);
  const loadedRef = useRef<ProviderSettings | null>(null);

  async function refresh() {
    try {
      const loaded = await getSettings();
      loadedRef.current = loaded;
      setSettings(loaded);
      setProfiles(await listProfiles());
      setNotif(await getNotifSettings());
      setMcpServers(await listMcpServers());
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }

  async function persistMcp(next: McpServer[]) {
    try {
      await saveMcpServers(next);
      setMcpServers(next);
      setError("");
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  }

  async function addMcpServer() {
    const name = mcpDraft.name.trim();
    if (!name) return;
    const url = mcpDraft.url.trim();
    const command = mcpDraft.command.trim();
    const env: Record<string, string> = {};
    for (const line of mcpDraft.env.split("\n").map((l) => l.trim()).filter(Boolean)) {
      const i = line.indexOf("=");
      if (i > 0) env[line.slice(0, i)] = line.slice(i + 1);
    }
    const server: McpServer = {
      name,
      command: command || null,
      args: mcpDraft.args.trim() ? mcpDraft.args.trim().split(/\s+/) : [],
      env,
      url: url || null,
    };
    const next = [...mcpServers.filter((s) => s.name !== name), server];
    if (await persistMcp(next)) {
      setMcpDraft(emptyMcpDraft);
      setMcpFormOpen(false);
    }
  }

  function editMcpServer(s: McpServer) {
    setMcpDraft({
      name: s.name,
      command: s.command ?? "",
      args: s.args.join(" "),
      env: Object.entries(s.env).map(([k, v]) => `${k}=${v}`).join("\n"),
      url: s.url ?? "",
    });
    setMcpFormOpen(true);
  }

  useEffect(() => {
    refresh();
  }, []);

  // Knowledge-graph config is per-project — (re)load whenever the selected
  // project changes; clear it when there is no project to configure.
  async function loadKnowledge(id: string) {
    try {
      const cfg = await getKnowledgeConfig(id);
      setKg(cfg);
      setKgDraft({ serve: cfg.serve_command ?? "", build: cfg.build_command ?? "" });
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    if (projectId) loadKnowledge(projectId);
    else setKg(null);
  }, [projectId]);

  // Persist the whole knowledge section at once (toggle and Save both route
  // here) so an in-progress command edit is never dropped by a toggle, then
  // reload to refresh the derived install-status flags.
  async function persistKnowledge(graph: boolean) {
    if (!projectId) return;
    try {
      await saveKnowledgeConfig(projectId, graph, kgDraft.serve.trim() || null, kgDraft.build.trim() || null);
      setError("");
      await loadKnowledge(projectId);
    } catch (e) {
      setError(String(e));
    }
  }

  // Flush unsaved provider edits on the way out, from whatever navigation.
  useEffect(() => () => {
    const loaded = loadedRef.current;
    const cur = settingsRef.current;
    if (loaded && cur.lmStudioBaseUrl !== loaded.lmStudioBaseUrl) {
      saveSettings(cur).catch(() => {});
    }
  }, []);

  async function persistSettings() {
    try {
      await saveSettings(settings);
      loadedRef.current = settings;
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
    // Empty resume field = no resume recipe: the agent always starts fresh.
    const resume = draft.resume.trim();
    const resume_args = resume ? resume.split(/\s+/) : null;
    // Empty loop field = no headless one-shot recipe: the agent can't loop.
    const loop = draft.loop.trim();
    const loop_args = loop ? loop.split(/\s+/) : null;
    try {
      await saveProfile({ name: draft.name.trim(), command: draft.command.trim(), args, env, resume_args, loop_args });
      closeForm();
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
      resume: (p.resume_args ?? []).join(" "),
      loop: (p.loop_args ?? []).join(" "),
    });
    setEditing(p.name);
    setFormOpen(true);
  }

  function openAddProfile() {
    setDraft(emptyDraft);
    setEditing(null);
    setFormOpen(true);
  }

  function closeForm() {
    setDraft(emptyDraft);
    setEditing(null);
    setFormOpen(false);
  }

  function renderProfileForm() {
    return (
      <div className="profile-form">
        <div className="settings-form-label">{editing ? "Edit profile" : "Add profile"}</div>
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
        <input
          className="settings-input"
          placeholder="resume args (e.g. --continue; empty = always start fresh)"
          value={draft.resume}
          onChange={(e) => setDraft({ ...draft, resume: e.target.value })}
        />
        <input
          className="settings-input"
          placeholder="loop args, headless one-shot (e.g. -p {{prompt}}; empty = can't loop)"
          value={draft.loop}
          onChange={(e) => setDraft({ ...draft, loop: e.target.value })}
        />
        <textarea
          className="settings-input"
          placeholder="env, one KEY=VALUE per line"
          value={draft.env}
          onChange={(e) => setDraft({ ...draft, env: e.target.value })}
        />
        <div className="row-actions">
          <button onClick={addProfile}>Save profile</button>
          <button className="ghost" onClick={closeForm}>Cancel</button>
        </div>
      </div>
    );
  }

  return (
    <main className="settings-page">
      <div className="settings-inner">
        <button className="settings-back" onClick={onClose}>← Back</button>
        <h1 className="settings-title">Settings</h1>
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
              <Fragment key={p.name}>
                <div className="settings-profile-card">
                  <div className="settings-profile-head">
                    <span className="agent-dot" style={{ background: agentColor(p.name) }} />
                    <span className="settings-profile-name">{agentLabel(p.name)}</span>
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
                    {(p.resume_args?.length ?? 0) > 0 && (
                      <>
                        <span className="settings-meta-key">resume</span>
                        <code className="settings-meta-val">{p.resume_args?.join(" ")}</code>
                      </>
                    )}
                    {(p.loop_args?.length ?? 0) > 0 && (
                      <>
                        <span className="settings-meta-key">loop</span>
                        <code className="settings-meta-val">{p.loop_args?.join(" ")}</code>
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
                {formOpen && editing === p.name && renderProfileForm()}
              </Fragment>
            ))}
          </div>
          {formOpen && editing === null
            ? renderProfileForm()
            : !formOpen && (
                <button className="settings-add-profile" onClick={openAddProfile}>+ Add agent profile</button>
              )}
        </section>

        <section className="settings-section">
          <div className="settings-section-label">MCP servers</div>
          <p className="settings-section-hint">
            Available to every agent workspace, in each agent's native config format (Claude, Cursor,
            OpenCode). Projects can add their own via <code>[mcp.servers]</code> in{" "}
            <code>.agency/agency.toml</code>; project entries win on name conflicts.
          </p>
          <div className="settings-card-list">
            {mcpServers.map((s) => (
              <div key={s.name} className="settings-profile-card">
                <div className="settings-profile-head">
                  <span className="settings-profile-name">{s.name}</span>
                  <span className="spacer" />
                  <button className="settings-ghost-btn" onClick={() => editMcpServer(s)}>Edit</button>
                  <button
                    className="settings-ghost-btn settings-del-btn"
                    onClick={() => persistMcp(mcpServers.filter((x) => x.name !== s.name))}
                  >
                    Delete
                  </button>
                </div>
                <div className="settings-profile-meta">
                  {s.url ? (
                    <>
                      <span className="settings-meta-key">url</span>
                      <code className="settings-meta-val">{s.url}</code>
                    </>
                  ) : (
                    <>
                      <span className="settings-meta-key">command</span>
                      <code className="settings-meta-val">{[s.command, ...s.args].filter(Boolean).join(" ")}</code>
                    </>
                  )}
                  {Object.keys(s.env).length > 0 && (
                    <>
                      <span className="settings-meta-key">env</span>
                      <code className="settings-meta-val">{Object.keys(s.env).join(", ")}</code>
                    </>
                  )}
                </div>
              </div>
            ))}
          </div>
          {mcpFormOpen ? (
            <div className="profile-form">
              <div className="settings-form-label">
                {mcpServers.some((s) => s.name === mcpDraft.name.trim()) ? "Edit MCP server" : "Add MCP server"}
              </div>
              <input
                className="settings-input"
                placeholder="name"
                value={mcpDraft.name}
                onChange={(e) => setMcpDraft({ ...mcpDraft, name: e.target.value })}
              />
              <input
                className="settings-input"
                placeholder="command (stdio server, e.g. npx)"
                value={mcpDraft.command}
                onChange={(e) => setMcpDraft({ ...mcpDraft, command: e.target.value })}
              />
              <input
                className="settings-input"
                placeholder="args (space-separated)"
                value={mcpDraft.args}
                onChange={(e) => setMcpDraft({ ...mcpDraft, args: e.target.value })}
              />
              <input
                className="settings-input"
                placeholder="url (remote server — leave command empty)"
                value={mcpDraft.url}
                onChange={(e) => setMcpDraft({ ...mcpDraft, url: e.target.value })}
              />
              <textarea
                className="settings-input"
                placeholder="env, one KEY=VALUE per line"
                value={mcpDraft.env}
                onChange={(e) => setMcpDraft({ ...mcpDraft, env: e.target.value })}
              />
              <div className="row-actions">
                <button onClick={addMcpServer}>Save server</button>
                <button className="ghost" onClick={() => { setMcpFormOpen(false); setMcpDraft(emptyMcpDraft); }}>Cancel</button>
              </div>
            </div>
          ) : (
            <button className="settings-add-profile" onClick={() => setMcpFormOpen(true)}>+ Add MCP server</button>
          )}
        </section>

        <section className="settings-section">
          <div className="settings-section-label">Knowledge graph</div>
          <p className="settings-section-hint">
            Builds a graphify code-knowledge graph for this project and exposes it to every agent as an
            MCP server, rebuilding after each clean merge. Saved to this machine only
            (<code>.agency/agency.local.toml</code>), not shared with the team. Needs the{" "}
            <code>graphify</code> / <code>uv</code> tooling on your <code>PATH</code>.
          </p>
          {!projectId ? (
            <div className="settings-group-card">
              <span className="settings-notif-label">Select a project to configure its knowledge graph.</span>
            </div>
          ) : kg ? (
            <div className="settings-group-card">
              <div className="settings-notif-row">
                <span className="settings-notif-label">
                  Enable knowledge graph{projectName ? ` for ${projectName}` : ""}
                </span>
                <Toggle checked={kg.graph} onChange={(next) => persistKnowledge(next)} />
              </div>
              {kg.graph && (
                <>
                  {(!kg.serve_installed || !kg.build_installed) && (
                    <div className="settings-kg-warn">
                      {!kg.serve_installed && !kg.build_installed
                        ? "The serve and build commands aren't on your PATH"
                        : !kg.serve_installed
                        ? "The serve command isn't on your PATH"
                        : "The build command isn't on your PATH"}
                      {" "}— the graph is enabled but will be skipped until the tooling is installed.
                    </div>
                  )}
                  <div className="settings-provider-field">
                    <label className="settings-field-key">serve</label>
                    <input
                      className="settings-field-input"
                      placeholder={kg.serve_default}
                      value={kgDraft.serve}
                      onChange={(e) => setKgDraft({ ...kgDraft, serve: e.target.value })}
                    />
                  </div>
                  <div className="settings-provider-field">
                    <label className="settings-field-key">build</label>
                    <input
                      className="settings-field-input"
                      placeholder={kg.build_default}
                      value={kgDraft.build}
                      onChange={(e) => setKgDraft({ ...kgDraft, build: e.target.value })}
                    />
                  </div>
                  <button className="settings-save" onClick={() => persistKnowledge(kg.graph)}>Save commands</button>
                </>
              )}
            </div>
          ) : null}
        </section>

        <section className="settings-section">
          <div className="settings-section-label">Local model (optional)</div>
          <p className="settings-section-hint">
            Points OpenAI-compatible agents at a local, OpenAI-protocol server via{" "}
            <code>OPENAI_BASE_URL</code>. Agents with their own login (Claude, Codex, …) ignore it.
            Leave blank to disable.
          </p>
          <div className="settings-providers">
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
          <button className="settings-save" onClick={persistSettings}>Save</button>
        </section>

        <section className="settings-section">
          <div className="settings-section-label">Editor</div>
          <div className="settings-group-card">
            <div className="settings-notif-row">
              <span className="settings-notif-label">Word wrap in file viewer</span>
              <Toggle checked={wordWrap} onChange={pickWordWrap} />
            </div>
          </div>
        </section>

        <section className="settings-section">
          <div className="settings-section-label">Notifications</div>
          <div className="settings-group-card">
            {([
              ["agentIdle", "Agent finished a turn"],
              ["agentFinished", "Agent exited"],
              ["runCrashed", "Run script crashed"],
              ["mergeAttention", "Merge needs attention"],
              ["loopEvents", "Loop complete or stalled"],
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
    </main>
  );
}
