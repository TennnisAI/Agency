import { Fragment, useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { appLogDir } from "@tauri-apps/api/path";
import { revealItemInDir, openUrl } from "@tauri-apps/plugin-opener";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  AgentProfile,
  CatalogEntry,
  FilesConfig,
  KnowledgeConfig,
  McpServer,
  Project,
  ProviderSettings,
  NotifSettings,
  UpdateCheck,
  authenticateMcpServer,
  checkForUpdate,
  closeProject,
  createWorkspace,
  deauthenticateMcpServer,
  defaultWorkspaceLocation,
  deleteProfile,
  enableAgentProfiles,
  getFilesConfig,
  getKnowledgeConfig,
  getSettings,
  getNotifSettings,
  getUpdateCheckEnabled,
  getWorkspace,
  importMcpJson,
  inspectRepo,
  listAgentCatalog,
  listMcpServers,
  listProfiles,
  moveWorkspace,
  saveFilesConfig,
  saveKnowledgeConfig,
  saveMcpServers,
  saveProfile,
  saveSettings,
  saveNotifSettings,
  setUpdateCheckEnabled,
} from "../api";
import Toggle from "./Toggle";
import { toastError, toastSuccess } from "../lib/toast";
import { agentColor, agentLabel } from "../agents";
import { THEMES, ThemeId, applyTheme, getStoredTheme } from "../lib/themes";
import { getWordWrap, setWordWrap } from "../lib/editorPrefs";
import { setWorkspaceHidden, workspaceHidden } from "../lib/workspacePref";

// Full-view settings page (design handoff: settings takes over the main area,
// entered from the ⚙ button at the bottom of the Projects pane).
export default function Settings({
  onClose,
  onOpenTerminal,
  projectId,
  projectName,
}: {
  onClose: () => void;
  onOpenTerminal?: (runId: string) => void;
  projectId: string | null;
  projectName: string | null;
}) {
  const [settings, setSettings] = useState<ProviderSettings>({
    lmStudioBaseUrl: "",
    defaultAgent: null,
  });
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  // Whether the "add built-in agent" dropdown is open.
  const [catalogOpen, setCatalogOpen] = useState(false);
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
  const emptyMcpDraft = { name: "", command: "", args: "", env: "", url: "", transport: "stdio", headers: "" };
  const [mcpDraft, setMcpDraft] = useState(emptyMcpDraft);
  const [mcpFormOpen, setMcpFormOpen] = useState(false);
  // Original name of the MCP server being edited (null when adding). A rename
  // must drop the old-named entry, not just upsert the new name — otherwise the
  // list keeps both and the server is duplicated.
  const [mcpEditing, setMcpEditing] = useState<string | null>(null);
  // Per-project knowledge-graph config. `null` until loaded (or when no project
  // is selected — the section then prompts to pick one). `kgDraft` holds the
  // editable command-override text so it survives re-renders between saves.
  const [kg, setKg] = useState<KnowledgeConfig | null>(null);
  const [kgDraft, setKgDraft] = useState({ serve: "", build: "" });
  // Per-project list of files copied into every new worktree. `files` holds the
  // loaded config (incl. auto-detected .env files); `filesDraft` is the editable
  // newline-separated text of the explicit copy list.
  const [files, setFiles] = useState<FilesConfig | null>(null);
  const [filesDraft, setFilesDraft] = useState("");
  // App version for the Diagnostics section; empty until the Tauri call lands.
  const [version, setVersion] = useState("");
  // Result of the last update check, or null before one has run in this view.
  const [update, setUpdate] = useState<UpdateCheck | null>(null);
  const [checking, setChecking] = useState(false);
  const [autoCheck, setAutoCheck] = useState(true);

  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
    getUpdateCheckEnabled().then(setAutoCheck).catch(() => {});
  }, []);

  async function runUpdateCheck() {
    setChecking(true);
    try {
      setUpdate(await checkForUpdate());
    } catch (e) {
      toastError(e, "Couldn't check for updates");
    } finally {
      setChecking(false);
    }
  }

  async function pickAutoCheck(on: boolean) {
    setAutoCheck(on);
    try {
      await setUpdateCheckEnabled(on);
    } catch (e) {
      setAutoCheck(!on);
      toastError(e, "Couldn't save the update setting");
    }
  }

  async function openLogs() {
    try {
      const dir = await appLogDir();
      await revealItemInDir(dir);
    } catch (e) {
      toastError(e, "Couldn't open the log folder");
    }
  }

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
      setCatalog(await listAgentCatalog());
      setNotif(await getNotifSettings());
      setMcpServers(await listMcpServers());
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }

  async function addFromCatalog(id: string) {
    try {
      await enableAgentProfiles([id]);
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't add agent");
    }
  }

  async function persistMcp(next: McpServer[]) {
    try {
      await saveMcpServers(next);
      setMcpServers(next);
      return true;
    } catch (e) {
      toastError(e, "Couldn't save MCP servers");
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
    // Headers accept "Key: value" or "Key=value"; only meaningful for remote servers.
    const headers: Record<string, string> = {};
    for (const line of mcpDraft.headers.split("\n").map((l) => l.trim()).filter(Boolean)) {
      const i = line.search(/[:=]/);
      if (i > 0) headers[line.slice(0, i).trim()] = line.slice(i + 1).trim();
    }
    // Only remote servers carry a transport; stdio is inferred from `command`.
    const transport: McpServer["transport"] = url
      ? (mcpDraft.transport === "sse" ? "sse" : "http")
      : null;
    const server: McpServer = {
      name,
      command: command || null,
      args: mcpDraft.args.trim() ? mcpDraft.args.trim().split(/\s+/) : [],
      env,
      url: url || null,
      transport,
      headers,
      // Preserve the authenticated flag across edits of the same server.
      userScope: mcpServers.find((s) => s.name === mcpEditing)?.userScope ?? false,
    };
    // Drop both the new name and the original (on a rename they differ) so an
    // edit replaces the entry instead of leaving a stale duplicate behind.
    const next = [
      ...mcpServers.filter((s) => s.name !== name && s.name !== mcpEditing),
      server,
    ];
    if (await persistMcp(next)) {
      setMcpDraft(emptyMcpDraft);
      setMcpEditing(null);
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
      transport: s.transport ?? (s.url ? "http" : "stdio"),
      headers: Object.entries(s.headers).map(([k, v]) => `${k}: ${v}`).join("\n"),
    });
    setMcpEditing(s.name);
    setMcpFormOpen(true);
  }

  // Import servers from a standard / VS Code mcp.json. Read client-side (avoids
  // a filesystem plugin) and merge into the global list backend-side.
  async function importMcp(file: File) {
    try {
      const { servers, imported } = await importMcpJson(await file.text());
      setMcpServers(servers);
      toastSuccess(`Imported ${imported} MCP server${imported === 1 ? "" : "s"}`);
    } catch (e) {
      toastError(e, "Couldn't import mcp.json");
    }
  }

  // Register the server with the Claude CLI at user scope (synchronously, so a
  // failure surfaces here instead of silently flagging it), then open a terminal
  // to complete the interactive /mcp OAuth sign-in.
  async function authenticateMcp(name: string) {
    if (!projectId) return;
    try {
      const run = await authenticateMcpServer(projectId, "claude", name);
      setMcpServers(await listMcpServers());
      // Jump straight into the terminal so the user can run /mcp and sign in.
      if (onOpenTerminal) onOpenTerminal(run.id);
      else onClose();
    } catch (e) {
      toastError(e, "Couldn't start MCP authentication");
    }
  }

  // Clear the user-scope flag so Agency emits the server per-worktree again — the
  // recovery path if registration failed or the user wants Agency to manage it.
  async function deauthenticateMcp(name: string) {
    try {
      await deauthenticateMcpServer(name);
      setMcpServers(await listMcpServers());
    } catch (e) {
      toastError(e, "Couldn't un-authenticate server");
    }
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

  // Worktree copy-list is per-project — same lifecycle as the knowledge config.
  async function loadFiles(id: string) {
    try {
      const cfg = await getFilesConfig(id);
      setFiles(cfg);
      setFilesDraft(cfg.copy.join("\n"));
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    if (projectId) loadFiles(projectId);
    else setFiles(null);
  }, [projectId]);

  async function persistFiles() {
    if (!projectId) return;
    const copy = filesDraft.split("\n").map((l) => l.trim()).filter(Boolean);
    try {
      await saveFilesConfig(projectId, copy);
      await loadFiles(projectId);
    } catch (e) {
      toastError(e, "Couldn't save worktree files");
    }
  }

  // Persist the whole knowledge section at once (toggle and Save both route
  // here) so an in-progress command edit is never dropped by a toggle, then
  // reload to refresh the derived install-status flags.
  async function persistKnowledge(graph: boolean) {
    if (!projectId) return;
    // Optimistic: reflect the toggle immediately so it doesn't lag the save
    // round-trip; revert if the write fails.
    const prev = kg;
    setKg((k) => (k ? { ...k, graph } : k));
    try {
      await saveKnowledgeConfig(projectId, graph, kgDraft.serve.trim() || null, kgDraft.build.trim() || null);
      await loadKnowledge(projectId);
    } catch (e) {
      setKg(prev);
      toastError(e, "Couldn't save knowledge settings");
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
    } catch (e) {
      toastError(e, "Couldn't save settings");
    }
  }

  // The default-agent dropdown saves immediately (unlike the text fields, which
  // flush on blur/exit). Empty value = "auto", persisted as null.
  function pickDefaultAgent(name: string) {
    const next = { ...settings, defaultAgent: name || null };
    setSettings(next);
    saveSettings(next)
      .then(() => { loadedRef.current = next; })
      .catch((e) => toastError(e, "Couldn't save settings"));
  }

  async function persistNotif(next: NotifSettings) {
    setNotif(next);
    try {
      await saveNotifSettings(next);
    } catch (e) {
      toastError(e, "Couldn't save notification settings");
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
    const name = draft.name.trim();
    try {
      await saveProfile({ name, command: draft.command.trim(), args, env, resume_args, loop_args });
      // A rename upserts under the new name; delete the old-named profile so the
      // list doesn't keep both.
      if (editing && editing !== name) {
        await deleteProfile(editing);
      }
      closeForm();
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't save profile");
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

  // ── workspace (pinned notes/journal project) ──────────────────────────
  const [workspace, setWorkspace] = useState<Project | null>(null);
  const [wsDefault, setWsDefault] = useState("");
  const [wsGitless, setWsGitless] = useState(false);
  const refreshWorkspace = () => {
    getWorkspace()
      .then(async (ws) => {
        setWorkspace(ws);
        if (ws) {
          const r = await inspectRepo(ws.repo_path).catch(() => null);
          setWsGitless(r?.state === "notARepo");
        }
      })
      .catch(() => {});
  };
  useEffect(() => {
    refreshWorkspace();
    defaultWorkspaceLocation().then(setWsDefault).catch(() => {});
  }, []);

  // Move = pick a destination parent; the folder keeps its name. The backend
  // renames on disk and repoints the project row.
  async function doMoveWorkspace() {
    if (!workspace) return;
    const sel = await openDialog({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    const name = workspace.repo_path.split("/").filter(Boolean).pop() ?? "Agency";
    const dest = `${sel.replace(/\/+$/, "")}/${name}`;
    if (dest === workspace.repo_path) return;
    try {
      setWorkspace(await moveWorkspace(dest));
      toastSuccess("Workspace moved");
    } catch (e) {
      toastError(e, "Couldn't move workspace");
    }
  }

  // Hide the workspace entirely for people who don't want it: the pinned ◈
  // row (and ⌘⇧D / the palette entry) go away. A created workspace is also
  // closed — sessions stop, records and files stay — and re-enabling revives
  // it via the idempotent create_workspace.
  const [wsOff, setWsOff] = useState(workspaceHidden());
  async function toggleWorkspaceVisible(show: boolean) {
    // Flip the pref (and this Toggle) immediately — the pinned row is gated on
    // the pref alone, so hiding is instant even while close_project is still
    // killing sessions.
    setWorkspaceHidden(!show);
    setWsOff(!show);
    try {
      const ws = await getWorkspace();
      if (ws) {
        if (show) await createWorkspace(ws.repo_path, false);
        else await closeProject(ws.id);
      }
      // Re-fire the pref event now that the row's closed state has settled.
      // The first event races the backend call by design (instant hide); this
      // one makes ProjectTree refetch a list that finally includes the revived
      // workspace — without it, re-enabling looked like it did nothing until
      // the next app launch.
      setWorkspaceHidden(!show);
    } catch (e) {
      // Backend didn't follow — put the pref (and Toggle) back.
      setWorkspaceHidden(show);
      setWsOff(show);
      toastError(e, show ? "Couldn't restore the workspace" : "Couldn't hide the workspace");
    }
    refreshWorkspace();
  }

  // Late opt-in to git for a workspace created without it (create_workspace is
  // idempotent: it just inits + makes the initial commit).
  async function enableWorkspaceGit() {
    if (!workspace) return;
    try {
      await createWorkspace(workspace.repo_path, true);
      refreshWorkspace();
      toastSuccess("Workspace repository initialized");
    } catch (e) {
      toastError(e, "Couldn't initialize git");
    }
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
          <div className="settings-section-label">Workspace</div>
          <p className="settings-section-hint">
            Your home for journaling, planning, and cross-project notes — plain
            markdown files on disk. <kbd>⌘⇧D</kbd> opens today's journal note.
          </p>
          <div className="settings-group-card">
            <div className="settings-notif-row">
              <span className="settings-notif-label">
                Show the workspace
                {wsOff && " — currently hidden; nothing on disk was deleted"}
              </span>
              <Toggle checked={!wsOff} onChange={(on) => { void toggleWorkspaceVisible(on); }} />
            </div>
            {!wsOff && (workspace ? (
              <>
                <div className="settings-notif-row">
                  <span className="settings-notif-label">
                    Location: <code className="settings-meta-val">{workspace.repo_path}</code>
                  </span>
                  <button className="settings-save" onClick={doMoveWorkspace}>Move…</button>
                </div>
                {wsGitless && (
                  <div className="settings-notif-row">
                    <span className="settings-notif-label">
                      Git is off, so agents can't be dispatched on notes. Initialize a
                      repository to enable them — nothing is ever pushed anywhere.
                    </span>
                    <button className="settings-save" onClick={enableWorkspaceGit}>Enable git</button>
                  </div>
                )}
              </>
            ) : (
              <div className="settings-notif-row">
                <span className="settings-notif-label">
                  Not created yet — by default it will live at{" "}
                  <code className="settings-meta-val">{wsDefault || "~/Agency"}</code>.
                </span>
                <button
                  className="settings-save"
                  onClick={() => {
                    onClose();
                    window.dispatchEvent(new CustomEvent("agency:create-workspace"));
                  }}
                >Create…</button>
              </div>
            ))}
          </div>
        </section>

        <section className="settings-section">
          <div className="settings-section-label">Agent profiles</div>
          <div className="settings-group-card">
            <div className="settings-notif-row">
              <span className="settings-notif-label">Default agent for new tasks</span>
              <select
                className="settings-input"
                style={{ maxWidth: 220 }}
                value={settings.defaultAgent ?? ""}
                onChange={(e) => pickDefaultAgent(e.target.value)}
              >
                {/* Auto = fall back to the project's last-used agent (prior behavior). */}
                <option value="">Auto (last used in project)</option>
                {profiles.map((p) => (
                  <option key={p.name} value={p.name}>{agentLabel(p.name)}</option>
                ))}
              </select>
            </div>
          </div>
          <div className="settings-card-list">
            {profiles.map((p) => (
              <Fragment key={p.name}>
                <div className="settings-profile-card">
                  <div className="settings-profile-head">
                    <span className="agent-dot" style={{ background: agentColor(p.name) }} />
                    <span className="settings-profile-name">{agentLabel(p.name)}</span>
                    <span className="spacer" />
                    <button className="settings-ghost-btn" onClick={() => editProfile(p)}>Edit</button>
                    <button className="settings-ghost-btn settings-del-btn" onClick={() => deleteProfile(p.name).then(refresh).catch((e) => toastError(e, "Couldn't delete profile"))}>Delete</button>
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
          {formOpen && editing === null ? (
            renderProfileForm()
          ) : !formOpen ? (
            <div className="settings-add-row">
              {catalog.some((e) => !e.enabled) && (
                <div className="settings-add-dropdown">
                  <button className="settings-add-profile" onClick={() => setCatalogOpen((o) => !o)}>
                    + Add agent ▾
                  </button>
                  {catalogOpen && (
                    <>
                      <div className="settings-menu-backdrop" onClick={() => setCatalogOpen(false)} />
                      <div className="settings-menu">
                        {catalog.filter((e) => !e.enabled).map((e) => (
                          <button key={e.id} onClick={() => { setCatalogOpen(false); addFromCatalog(e.id); }}>
                            <span className="agent-dot" style={{ background: agentColor(e.id) }} />
                            <span className="settings-menu-name">{agentLabel(e.id)}</span>
                            <code className="settings-menu-cmd">{e.command}</code>
                          </button>
                        ))}
                      </div>
                    </>
                  )}
                </div>
              )}
              <button className="settings-add-profile" onClick={openAddProfile}>+ Custom agent</button>
            </div>
          ) : null}
        </section>

        <section className="settings-section">
          <div className="settings-section-label">MCP servers</div>
          <p className="settings-section-hint">
            Emitted into each agent workspace, in the agent's native config format (
            {catalog.filter((e) => e.supportsMcp).map((e) => agentLabel(e.id)).join(", ") ||
              "Claude Code, Copilot CLI, Cursor, OpenCode"}
            ). Copilot loads workspace servers after you approve folder trust on first launch.
            Remote servers support <code>http</code>/<code>sse</code> transports and auth
            headers; OAuth servers (e.g. Atlassian) use <b>Authenticate</b> to sign in via the Claude
            CLI at user scope. Projects can add their own via <code>[mcp.servers]</code> in{" "}
            <code>.agency/agency.toml</code>; project entries win on name conflicts.
          </p>
          {catalog.some((e) => e.enabled && !e.supportsMcp) && (
            <p className="settings-section-hint">
              {catalog.filter((e) => e.enabled && !e.supportsMcp).map((e) => agentLabel(e.id)).join(", ")}{" "}
              manage MCP servers in their own global config and won't see the servers below.
            </p>
          )}
          <div className="settings-card-list">
            {mcpServers.map((s) => (
              <div key={s.name} className="settings-profile-card">
                <div className="settings-profile-head">
                  <span className="settings-profile-name">{s.name}</span>
                  {s.userScope && <span className="settings-meta-key">user scope · not emitted per-worktree</span>}
                  <span className="spacer" />
                  {s.url && !s.userScope && (
                    <button
                      className="settings-ghost-btn"
                      disabled={!projectId}
                      title={projectId ? "Register & sign in with the Claude CLI" : "Select a project first"}
                      onClick={() => authenticateMcp(s.name)}
                    >
                      Authenticate
                    </button>
                  )}
                  {s.url && s.userScope && (
                    <button
                      className="settings-ghost-btn"
                      title="Stop treating this as user-scope; Agency will emit it into worktrees again"
                      onClick={() => deauthenticateMcp(s.name)}
                    >
                      Un-authenticate
                    </button>
                  )}
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
                      <span className="settings-meta-key">{s.transport ?? "http"}</span>
                      <code className="settings-meta-val">{s.url}</code>
                    </>
                  ) : (
                    <>
                      <span className="settings-meta-key">command</span>
                      <code className="settings-meta-val">{[s.command, ...s.args].filter(Boolean).join(" ")}</code>
                    </>
                  )}
                  {Object.keys(s.headers).length > 0 && (
                    <>
                      <span className="settings-meta-key">headers</span>
                      <code className="settings-meta-val">{Object.keys(s.headers).join(", ")}</code>
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
              {mcpDraft.url.trim() && (
                <>
                  <select
                    className="settings-input"
                    value={mcpDraft.transport === "sse" ? "sse" : "http"}
                    onChange={(e) => setMcpDraft({ ...mcpDraft, transport: e.target.value })}
                  >
                    <option value="http">transport: http (streamable)</option>
                    <option value="sse">transport: sse</option>
                  </select>
                  <textarea
                    className="settings-input"
                    placeholder="headers (remote auth), one 'Authorization: Bearer …' per line"
                    value={mcpDraft.headers}
                    onChange={(e) => setMcpDraft({ ...mcpDraft, headers: e.target.value })}
                  />
                </>
              )}
              <textarea
                className="settings-input"
                placeholder="env, one KEY=VALUE per line"
                value={mcpDraft.env}
                onChange={(e) => setMcpDraft({ ...mcpDraft, env: e.target.value })}
              />
              <div className="row-actions">
                <button onClick={addMcpServer}>Save server</button>
                <button className="ghost" onClick={() => { setMcpFormOpen(false); setMcpDraft(emptyMcpDraft); setMcpEditing(null); }}>Cancel</button>
              </div>
            </div>
          ) : (
            <div className="row-actions">
              <button className="settings-add-profile" onClick={() => setMcpFormOpen(true)}>+ Add MCP server</button>
              <label className="settings-add-profile" style={{ cursor: "pointer" }}>
                Import mcp.json
                <input
                  type="file"
                  accept=".json,application/json"
                  style={{ display: "none" }}
                  onChange={(e) => {
                    const f = e.target.files?.[0];
                    if (f) importMcp(f);
                    e.target.value = "";
                  }}
                />
              </label>
            </div>
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
                  <div className="settings-card-foot">
                    <button className="settings-save" onClick={() => persistKnowledge(kg.graph)}>Save commands</button>
                  </div>
                </>
              )}
            </div>
          ) : null}
        </section>

        <section className="settings-section">
          <div className="settings-section-label">Worktree files</div>
          <p className="settings-section-hint">
            Files copied into every new agent worktree. Git worktrees only contain
            committed files, so untracked essentials (local certs, service-account
            keys) must be listed here to reach agents. Untracked{" "}
            <code>.env</code> / <code>.env.*</code> files in the repo root are copied
            automatically. Saved to this machine only (<code>.agency/agency.local.toml</code>).
          </p>
          {!projectId ? (
            <div className="settings-group-card">
              <span className="settings-notif-label">Select a project to configure its worktree files.</span>
            </div>
          ) : files ? (
            <div className="settings-group-card">
              {files.detectedEnv.length > 0 && (
                <div className="settings-notif-row">
                  <span className="settings-notif-label">
                    Auto-copied env files:{" "}
                    {files.detectedEnv.map((f) => <code key={f} className="settings-meta-val">{f}</code>)}
                  </span>
                </div>
              )}
              <div className="settings-field-stack">
                <label className="settings-field-key">Files to copy</label>
                <textarea
                  className="settings-input"
                  placeholder={"One repo-relative path per line, e.g.\nconfig/service-account.json\ncerts/dev.pem"}
                  value={filesDraft}
                  onChange={(e) => setFilesDraft(e.target.value)}
                />
              </div>
              <div className="settings-card-foot">
                <button className="settings-save" onClick={persistFiles}>Save file list</button>
              </div>
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
              <div className="settings-card-foot">
                <button className="settings-save" onClick={persistSettings}>Save</button>
              </div>
            </div>
          </div>
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

        <section className="settings-section">
          <div className="settings-section-label">Diagnostics</div>
          <div className="settings-group-card">
            <div className="settings-notif-row">
              <span className="settings-notif-label">Version</span>
              <span className="settings-notif-label">{version ? `Agency ${version}` : "Agency"}</span>
            </div>
            <div className="settings-notif-row">
              <span className="settings-notif-label">
                {update?.updateAvailable
                  ? `Agency ${update.latest} is available`
                  : update?.error
                    ? "Couldn't reach the releases feed"
                    : update
                      ? "Up to date"
                      : "Updates"}
              </span>
              {update?.updateAvailable ? (
                <button
                  className="settings-ghost-btn"
                  onClick={() => { openUrl(update.url).catch(() => {}); }}
                >Download</button>
              ) : (
                <button className="settings-ghost-btn" disabled={checking} onClick={runUpdateCheck}>
                  {checking ? "Checking…" : "Check now"}
                </button>
              )}
            </div>
            <div className="settings-notif-row">
              <span className="settings-notif-label">Check for updates on launch</span>
              <Toggle checked={autoCheck} onChange={pickAutoCheck} />
            </div>
            <div className="settings-notif-row">
              <span className="settings-notif-label">Log files</span>
              <button className="settings-ghost-btn" onClick={openLogs}>Open logs</button>
            </div>
            <div className="settings-notif-row">
              <span className="settings-notif-label">Report an issue</span>
              <button
                className="settings-ghost-btn"
                onClick={() => { openUrl("https://github.com/nic123/Agency/issues/new").catch(() => {}); }}
              >Open GitHub issues</button>
            </div>
          </div>
        </section>
      </div>
    </main>
  );
}
