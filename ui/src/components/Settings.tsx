import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { appLogDir } from "@tauri-apps/api/path";
import { revealItemInDir, openUrl } from "@tauri-apps/plugin-opener";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  AgentCliInfo,
  AgentProfile,
  CatalogEntry,
  FilesConfig,
  IssueKeyPreview,
  IssueSyncConfig,
  KnowledgeConfig,
  McpServer,
  McpTransport,
  Project,
  ProviderSettings,
  NotifSettings,
  UpdateCheck,
  agentCliInfo,
  authenticateMcpServer,
  checkForUpdate,
  closeProject,
  createWorkspace,
  deauthenticateMcpServer,
  defaultWorkspaceLocation,
  deleteProfile,
  enableAgentProfiles,
  getFilesConfig,
  getIssueSyncConfig,
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
  relocateProject,
  saveFilesConfig,
  saveIssueSyncConfig,
  previewIssueKey,
  setProjectIssueKey,
  saveKnowledgeConfig,
  setKnowledgeBackend,
  buildKnowledgeGraph,
  stopKnowledgeBuild,
  installKnowledgeTooling,
  saveMcpServers,
  saveProfile,
  saveSettings,
  saveNotifSettings,
  setAgentModel,
  setUpdateCheckEnabled,
} from "../api";
import ModelSelect from "./ModelSelect";
import PillSelect from "./PillSelect";
import Toggle from "./Toggle";
import ConfirmDialog from "./ConfirmDialog";
import { notifyProjectsChanged } from "../lib/projectEvents";
import FormDialog, { Field } from "./FormDialog";
import NoticesDialog from "./NoticesDialog";
import { toastError, toastSuccess } from "../lib/toast";
import { fmtDur } from "../lib/runstate";
import { agentColor, agentLabel, updateCommand } from "../agents";
import { THEMES, ThemeId, applyTheme, getStoredTheme } from "../lib/themes";
import { getWordWrap, setWordWrap } from "../lib/editorPrefs";
import { useAgentModels } from "../hooks/useAgentModels";
import { resendOpenFile } from "../hooks/useOpenFile";
import { setWorkspaceHidden, workspaceHidden } from "../lib/workspacePref";
import { HUSHABLE, HushId, isHushed, setHushed } from "../lib/hushed";
import { FindRank, registerFindTarget } from "../lib/findBus";
import { shortcutLabel } from "../lib/platform";
import {
  ExtraTerms,
  GROUPS,
  SECTIONS,
  SECTION_BY_ID,
  SectionId,
  matchSections,
  scopeOf,
} from "../lib/settingsSections";

// One editable row of an MCP server's headers or environment. Kept as an
// ordered pair list rather than a Record while editing so a half-typed row
// (blank key, or two rows briefly sharing a key) survives the next keystroke.
type KeyValue = { key: string; value: string };

const recordToPairs = (r: Record<string, string>): KeyValue[] =>
  Object.entries(r).map(([key, value]) => ({ key, value }));

// Blank-keyed rows are dropped; later rows win on a duplicate key.
const pairsToRecord = (pairs: KeyValue[]): Record<string, string> =>
  Object.fromEntries(
    pairs.map(({ key, value }) => [key.trim(), value.trim()]).filter(([k]) => k),
  );

// " after 4m" for a finished graph build, empty when there is no timing to
// report (the app was restarted since, so the build it watched is not one it
// has a duration for).
const buildTook = (kg: KnowledgeConfig): string =>
  kg.last_build_secs === null ? "" : ` after ${fmtDur(kg.last_build_secs * 1000)}`;

// Which section Settings reopens on. Kept in the module rather than in storage:
// coming straight back to where you were is worth having within a session,
// remembering it a week later is not.
let lastSection: SectionId = "appearance";

// The heading of one section, with the tag that says who it applies to. The
// project-scoped tag carries the project's own name rather than the bare word
// "project", so the heading answers "which one" in the same glance.
function SectionHead({ id, projectName }: { id: SectionId; projectName: string | null }) {
  const scope = scopeOf(id);
  return (
    <div className="settings-section-head">
      <span className="settings-section-label">{SECTION_BY_ID[id].label}</span>
      <span className="settings-scope-tag" data-scope={scope}>
        {scope === "global" ? "Global" : projectName ? `Project · ${projectName}` : "No project"}
      </span>
    </div>
  );
}

// The three shapes an MCP server can take, as offered by the transport picker.
// "Local" spawns a command over stdio; the other two are remote endpoints.
const MCP_TRANSPORTS: { id: McpTransport; label: string }[] = [
  { id: "stdio", label: "Local" },
  { id: "http", label: "HTTP" },
  { id: "sse", label: "SSE" },
];

// Editable key/value list for MCP headers and environment variables. Beats a
// free-text box: no separator to get wrong, and a secret with a colon or an "="
// in it round-trips intact. Rows are addressed by index so editing a key does
// not re-key the row and steal focus mid-keystroke.
function KeyValueRows({
  pairs,
  onChange,
  keyPlaceholder,
  valuePlaceholder,
  addLabel,
}: {
  pairs: KeyValue[];
  onChange: (pairs: KeyValue[]) => void;
  keyPlaceholder: string;
  valuePlaceholder: string;
  addLabel: string;
}) {
  const edit = (i: number, patch: Partial<KeyValue>) =>
    onChange(pairs.map((p, n) => (n === i ? { ...p, ...patch } : p)));
  return (
    <div className="kv-rows">
      {pairs.map((p, i) => (
        <div className="kv-row" key={i}>
          <input
            className="settings-input mono"
            placeholder={keyPlaceholder}
            value={p.key}
            onChange={(e) => edit(i, { key: e.target.value })}
          />
          <input
            className="settings-input mono"
            placeholder={valuePlaceholder}
            value={p.value}
            onChange={(e) => edit(i, { value: e.target.value })}
          />
          <button
            type="button"
            className="settings-ghost-btn kv-del"
            aria-label="Remove"
            onClick={() => onChange(pairs.filter((_, n) => n !== i))}
          >
            ✕
          </button>
        </div>
      ))}
      <button
        type="button"
        className="kv-add"
        onClick={() => onChange([...pairs, { key: "", value: "" }])}
      >
        + {addLabel}
      </button>
    </div>
  );
}

// Full-view settings page (design handoff: settings takes over the main area,
// entered from the ⚙ button at the bottom of the Projects pane).
export default function Settings({
  onClose,
  onOpenTerminal,
  projectId,
  projectName,
  section,
}: {
  onClose: () => void;
  onOpenTerminal?: (runId: string) => void;
  projectId: string | null;
  projectName: string | null;
  /**
   * Section to open on, for a caller that is answering "where do I turn this
   * on" rather than opening Settings for its own sake. null keeps
   * `lastSection`, which is what every other caller wants.
   */
  section?: SectionId | null;
}) {
  const [settings, setSettings] = useState<ProviderSettings>({
    lmStudioBaseUrl: "",
    defaultAgent: null,
    defaultWorktree: true,
    shareOpenFile: false,
  });
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const { models, reload: reloadModels } = useAgentModels();
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
    onlyWhenWatching: true,
    idleSecs: 30,
  });
  const [themeId, setThemeId] = useState<ThemeId>(getStoredTheme());
  const [wordWrap, setWrap] = useState<boolean>(getWordWrap());
  // Which explanations the user has switched off, mirrored into state so the
  // toggles move. The rows read "show this message", so a ticked row is one
  // that is *not* hushed.
  const [hushed, setHushedState] = useState<HushId[]>(
    () => HUSHABLE.filter((h) => isHushed(h.id)).map((h) => h.id),
  );
  const [mcpServers, setMcpServers] = useState<McpServer[]>([]);
  // The draft mirrors the form: `transport` is the single source of truth for
  // whether this is a local (stdio) or remote server, so the form shows one
  // coherent set of fields instead of asking for a command *and* a URL and
  // inferring which the user meant. Headers and env are ordered pairs so blank
  // rows can exist while typing (a Record would collapse them).
  const emptyMcpDraft = {
    name: "",
    transport: "stdio" as McpTransport,
    command: "",
    args: "",
    url: "",
    headers: [] as KeyValue[],
    env: [] as KeyValue[],
  };
  const [mcpDraft, setMcpDraft] = useState(emptyMcpDraft);
  const [mcpFormOpen, setMcpFormOpen] = useState(false);
  // Name of the server whose "Authenticate with…" agent menu is open.
  const [mcpAuthMenu, setMcpAuthMenu] = useState<string | null>(null);
  // Original name of the MCP server being edited (null when adding). A rename
  // must drop the old-named entry, not just upsert the new name — otherwise the
  // list keeps both and the server is duplicated.
  const [mcpEditing, setMcpEditing] = useState<string | null>(null);
  // Per-project knowledge-graph config. `null` until loaded (or when no project
  // is selected — the section then prompts to pick one). `kgDraft` holds the
  // editable command-override text so it survives re-renders between saves.
  const [backlog, setBacklog] = useState<IssueSyncConfig | null>(null);
  // Editable remote text, kept out of `backlog` so it survives re-renders
  // between saves, the same way `kgDraft` does.
  const [backlogRemote, setBacklogRemote] = useState("");
  // The issue key input's own text, for the same reason the remote has one: it
  // is deliberately mid-edit between the first keystroke and the blur that
  // commits it, and nothing saved must read from it.
  const [backlogKey, setBacklogKey] = useState("");
  // The rename the user is being asked to confirm, and whether it is running.
  const [keyPreview, setKeyPreview] = useState<IssueKeyPreview | null>(null);
  const [keyBusy, setKeyBusy] = useState(false);
  const [kg, setKg] = useState<KnowledgeConfig | null>(null);
  const [kgDraft, setKgDraft] = useState({ serve: "", build: "" });
  // The model name typed under the backend picker, saved on blur (a keystroke
  // is not a choice). Mirrors what the effective build command already names.
  const [kgModel, setKgModel] = useState("");
  // The build-output pane, and whether it is currently scrolled to the newest
  // line. Followed only while it is: pulling the pane back down every 1.5s
  // while the user is reading further up is the scroll-latch bug again.
  const kgLogRef = useRef<HTMLPreElement | null>(null);
  const kgLogFollow = useRef(true);
  // Why a requested build never started (tooling missing, one already running).
  // Cleared on the next attempt; build failures come back on the config itself.
  const [kgBuildError, setKgBuildError] = useState<string | null>(null);
  // Per-project list of files copied into every new worktree. `files` holds the
  // loaded config (incl. auto-detected .env files); `filesDraft` is the editable
  // newline-separated text of the explicit copy list.
  const [files, setFiles] = useState<FilesConfig | null>(null);
  const [filesDraft, setFilesDraft] = useState("");
  // App version for the Diagnostics section; empty until the Tauri call lands.
  const [version, setVersion] = useState("");
  // Per-agent CLI facts (path, version, install method) for the Diagnostics
  // section. Loaded in its own effect because the backend runs each CLI's
  // `--version`, which takes a second or two; rows appear when it answers.
  const [cliInfo, setCliInfo] = useState<AgentCliInfo[]>([]);
  // Result of the last update check, or null before one has run in this view.
  const [update, setUpdate] = useState<UpdateCheck | null>(null);
  const [checking, setChecking] = useState(false);
  const [autoCheck, setAutoCheck] = useState(true);
  // The third-party licence notices, shown on demand: the file is 650 KB, so
  // the dialog fetches it rather than the bundle carrying it at startup.
  const [noticesOpen, setNoticesOpen] = useState(false);

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

  function pickMessage(id: HushId, show: boolean) {
    setHushedState((prev) => (show ? prev.filter((h) => h !== id) : [...prev, id]));
    setHushed(id, !show);
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
    const remote = mcpDraft.transport !== "stdio";
    const url = mcpDraft.url.trim();
    const command = mcpDraft.command.trim();
    // Only the fields the chosen transport actually uses are persisted, so
    // switching Local → HTTP mid-edit can't leave a stale command behind that
    // would make the entry fail validation (a server is command *or* url).
    const server: McpServer = {
      name,
      command: remote ? null : command || null,
      args: remote || !mcpDraft.args.trim() ? [] : mcpDraft.args.trim().split(/\s+/),
      env: remote ? {} : pairsToRecord(mcpDraft.env),
      url: remote ? url || null : null,
      transport: remote ? mcpDraft.transport : null,
      headers: remote ? pairsToRecord(mcpDraft.headers) : {},
      // Preserve which agents this server is already registered with across edits.
      userScopeAgents: mcpServers.find((s) => s.name === mcpEditing)?.userScopeAgents ?? [],
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
      env: recordToPairs(s.env),
      url: s.url ?? "",
      transport: s.transport ?? (s.url ? "http" : "stdio"),
      headers: recordToPairs(s.headers),
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

  // Register the server with `agent`'s own CLI at that CLI's user scope
  // (synchronously, so a failure surfaces here instead of silently recording
  // it), then open a terminal to complete the interactive /mcp OAuth sign-in.
  // Agency keeps emitting the server into every *other* agent's workspace.
  async function authenticateMcp(agent: string, name: string) {
    if (!projectId) return;
    setMcpAuthMenu(null);
    try {
      const run = await authenticateMcpServer(projectId, agent, name);
      setMcpServers(await listMcpServers());
      // Jump straight into the terminal so the user can run /mcp and sign in.
      if (onOpenTerminal) onOpenTerminal(run.id);
      else onClose();
    } catch (e) {
      toastError(e, "Couldn't start MCP authentication");
    }
  }

  // Drop one agent's user-scope registration so Agency emits the server into
  // that agent's worktrees again — the recovery path if registration failed or
  // the user wants Agency to manage it.
  async function deauthenticateMcp(agent: string, name: string) {
    try {
      await deauthenticateMcpServer(agent, name);
      setMcpServers(await listMcpServers());
    } catch (e) {
      toastError(e, "Couldn't un-authenticate server");
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  useEffect(() => {
    agentCliInfo().then(setCliInfo).catch(() => {});
  }, []);

  // The project the three per-project loads below belong to. Settings now
  // stays open across a project switch (AGE-187), so those loads race where
  // they never used to: the heading renders the new project's name from its
  // prop immediately, while a reply for the project you just left is still in
  // flight. knowledge_config probes PATH for claude and ollama on every call,
  // so the window is wide enough to click in, and toggling "Share the backlog"
  // inside it wrote the old project's remote into the new project's
  // .agency/agency.local.toml. Assigned during render so an in-flight loader
  // always compares against the project actually on screen.
  const shownProject = useRef<string | null>(projectId);
  shownProject.current = projectId;

  /**
   * True while `id` is still the project on screen. Every per-project load and
   * save calls this after its await, before touching state: the reply belongs
   * to whichever project was selected when the call went out, and by the time
   * it lands that may not be the one the headings are naming.
   */
  const stillShowing = (id: string | null) => id === shownProject.current;

  // Knowledge-graph config is per-project — clear and reload whenever the
  // selected project changes.
  async function loadKnowledge(id: string) {
    try {
      const cfg = await getKnowledgeConfig(id);
      if (!stillShowing(id)) return;
      setKg(cfg);
      setKgDraft({ serve: cfg.serve_command ?? "", build: cfg.build_command ?? "" });
      setKgModel(cfg.build_model);
    } catch (e) {
      if (!stillShowing(id)) return;
      setError(String(e));
    }
  }

  // useLayoutEffect, not useEffect, for this and the two below: clearing first
  // is the point, and a passive effect runs after the commit that already
  // rendered the new project's name beside the old project's values. A layout
  // effect lands the clear in the same paint as the name, so that frame never
  // exists.
  useLayoutEffect(() => {
    // Clear first: the section renders nothing without a config, which is the
    // honest state while the new project's load is out, and is what a fresh
    // mount shows anyway. Leaving the old values up put the wrong project's
    // settings under the right project's name.
    setKg(null);
    setKgDraft({ serve: "", build: "" });
    setKgModel("");
    if (projectId) loadKnowledge(projectId);
  }, [projectId]);

  // Backlog sharing is per-project too, and loads on the same schedule.
  async function loadBacklog(id: string) {
    try {
      const cfg = await getIssueSyncConfig(id);
      if (!stillShowing(id)) return;
      setBacklog(cfg);
      setBacklogRemote(cfg.remote);
      setBacklogKey(cfg.issueKey);
    } catch (e) {
      if (!stillShowing(id)) return;
      setError(String(e));
    }
  }

  useLayoutEffect(() => {
    setBacklog(null);
    setBacklogRemote("");
    setBacklogKey("");
    if (projectId) loadBacklog(projectId);
  }, [projectId]);

  // One write for all three fields: the remote is meaningless with sync off,
  // and saving them separately would let a half-applied state reach disk.
  // Patch-shaped rather than positional for the same reason, now that there are
  // three: a caller changing one would otherwise have to restate the other two,
  // and the day it restated a stale one it would silently turn sharing off.
  async function persistBacklog(patch: { sync?: boolean; auto?: boolean; remote?: string }) {
    const id = projectId;
    if (!id || !backlog) return;
    const prev = backlog;
    const sync = patch.sync ?? backlog.sync;
    const auto = patch.auto ?? backlog.auto;
    // The saved remote and not `backlogRemote`, which is the input's own state
    // and is deliberately empty between choosing "Another remote…" and typing
    // one. Falling back to it meant flipping either toggle in that window wrote
    // an empty remote, which the backend reads as `origin`: the select snapped
    // back and the backlog was pointed somewhere the user had not chosen. The
    // select and the input's blur are the only writers of this field.
    const remote = patch.remote ?? backlog.remote;
    setBacklog((b) => (b ? { ...b, sync, auto, remote } : b));
    try {
      await saveIssueSyncConfig(id, sync, auto, remote);
      await loadBacklog(id);
    } catch (e) {
      // The revert is guarded for the same reason the loaders are, and it is
      // the worse half of the race: it puts a whole config object back, so
      // switching project during a failing save restored the project you left
      // under the name of the one on screen, and the next toggle wrote that
      // remote into this project's .agency/agency.local.toml. The toast is not
      // guarded — the write really did fail, whatever is on screen now.
      if (stillShowing(id)) setBacklog(prev);
      toastError(e, "Couldn't save backlog settings");
    }
  }

  // The key the input holds, as the backend would store it, when it differs
  // from the saved one. What enables Rename.
  const pendingKey = (() => {
    const next = backlogKey.trim().toUpperCase();
    return backlog && next && next !== backlog.issueKey ? next : null;
  })();

  // Renaming the key moves every issue file, so nothing commits until the
  // user has read what will move and said so. This used to commit on blur,
  // which renamed the whole backlog to `AG` on the way to typing `AGE` and
  // then renamed it all again, and showed the "another project uses this key"
  // warning only for the key already saved, which is to say after the fact.
  async function previewKeyRename() {
    const id = projectId;
    if (!id || !pendingKey) return;
    try {
      const preview = await previewIssueKey(id, pendingKey);
      if (stillShowing(id)) setKeyPreview(preview);
    } catch (e) {
      toastError(e, "Couldn't change the issue key");
    }
  }

  async function doKeyRename(preview: IssueKeyPreview) {
    const id = projectId;
    if (!id) return;
    setKeyBusy(true);
    try {
      const plan = await setProjectIssueKey(id, preview.key);
      const n = plan.moves.length;
      const renumbered = plan.renumbered.map((m) => `${m.from} is now ${m.to}`).join(", ");
      toastSuccess(
        n === 0
          ? `Issue key set to ${preview.key}`
          : `${n} issue file${n === 1 ? "" : "s"} renamed to ${preview.key}` +
              (renumbered ? `. ${renumbered}.` : ""),
      );
      setKeyPreview(null);
      // The project row every open view holds now names the old key.
      notifyProjectsChanged();
      await loadBacklog(id);
    } catch (e) {
      toastError(e, "Couldn't change the issue key");
      if (stillShowing(id)) {
        setKeyPreview(null);
        await loadBacklog(id);
      }
    } finally {
      setKeyBusy(false);
    }
  }

  useEffect(() => {
    const el = kgLogRef.current;
    if (el && kgLogFollow.current) el.scrollTop = el.scrollHeight;
  }, [kg?.build_log]);

  // A graph build runs on its own thread with no event of its own, so poll the
  // config while one is in flight to pick up its output, the finish, and any
  // failure.
  useEffect(() => {
    if (!projectId || !kg?.building) return;
    const t = setInterval(() => loadKnowledge(projectId), 1500);
    return () => clearInterval(t);
  }, [projectId, kg?.building]);

  // What the picker offers, and which of it the saved build command names.
  // "Custom" is only in the list when that command is one the picker didn't
  // write, so the pill has something to show instead of rendering blank.
  const backendOptions = [
    ...(kg?.backends ?? []).map((b) => ({ value: b.id, label: b.label })),
    ...(kg?.build_backend === "custom" ? [{ value: "custom", label: "Custom" }] : []),
  ];
  const pickedBackend = kg?.backends.find((b) => b.id === kg.build_backend) ?? null;
  // Claude Code's own model list, when that is the backend the build runs on.
  // Undefined for every other backend, and while the list is still loading,
  // which falls the control back to a typed model id.
  const claudeCliModels =
    pickedBackend?.id === "claude-cli" && models.claude?.supported ? models.claude : undefined;

  async function buildGraph() {
    const id = projectId;
    if (!id) return;
    setKgBuildError(null);
    try {
      await buildKnowledgeGraph(id);
    } catch (e) {
      // Guarded: the error renders inside the knowledge section, under the
      // selected project's name, so the project it is about has to be the one
      // being named.
      if (stillShowing(id)) setKgBuildError(String(e));
    }
    await loadKnowledge(id);
  }

  // The way out of a build that is going nowhere. Without it a wedged build
  // holds the slot until the app restarts, and every later build (including
  // every post-merge rebuild) is refused as already running.
  async function stopBuild() {
    const id = projectId;
    if (!id) return;
    setKgBuildError(null);
    try {
      await stopKnowledgeBuild(id);
    } catch (e) {
      if (stillShowing(id)) setKgBuildError(String(e));
    }
    await loadKnowledge(id);
  }

  // Choosing a model writes the build command and stops there. The build is a
  // separate, deliberate press: this panel exists so that what a build costs
  // and where it sends the project's docs is known before that.
  async function chooseBackend(backend: string, model: string) {
    const id = projectId;
    if (!id) return;
    setKgBuildError(null);
    try {
      await setKnowledgeBackend(id, backend, model);
    } catch (e) {
      toastError(e, "Couldn't save that model");
    }
    await loadKnowledge(id);
  }

  // Install the tooling the way a missing agent CLI is installed: in a visible
  // terminal the user is dropped into, so they see what runs on their machine
  // and can answer anything it asks.
  async function installKgTooling() {
    if (!projectId) return;
    setKgBuildError(null);
    try {
      const run = await installKnowledgeTooling(projectId);
      if (onOpenTerminal) onOpenTerminal(run.id);
      else onClose();
    } catch (e) {
      toastError(e, "Couldn't start the install");
    }
  }

  // Worktree copy-list is per-project — same lifecycle as the knowledge config.
  async function loadFiles(id: string) {
    try {
      const cfg = await getFilesConfig(id);
      if (!stillShowing(id)) return;
      setFiles(cfg);
      setFilesDraft(cfg.copy.join("\n"));
    } catch (e) {
      if (!stillShowing(id)) return;
      setError(String(e));
    }
  }

  useLayoutEffect(() => {
    setFiles(null);
    setFilesDraft("");
    if (projectId) loadFiles(projectId);
  }, [projectId]);

  async function persistFiles() {
    const id = projectId;
    if (!id) return;
    const copy = filesDraft.split("\n").map((l) => l.trim()).filter(Boolean);
    try {
      await saveFilesConfig(id, copy);
      await loadFiles(id);
    } catch (e) {
      toastError(e, "Couldn't save worktree files");
    }
  }

  // Persist the whole knowledge section at once (toggle and Save both route
  // here) so an in-progress command edit is never dropped by a toggle, then
  // reload to refresh the derived install-status flags.
  async function persistKnowledge(patch: Partial<Pick<KnowledgeConfig, "graph" | "rebuild_on_merge">>) {
    const id = projectId;
    if (!id || !kg) return;
    // Optimistic: reflect the toggle immediately so it doesn't lag the save
    // round-trip; revert if the write fails.
    const prev = kg;
    const next = { ...kg, ...patch };
    setKg(next);
    try {
      await saveKnowledgeConfig(
        id,
        next.graph,
        next.rebuild_on_merge,
        kgDraft.serve.trim() || null,
        kgDraft.build.trim() || null,
      );
      await loadKnowledge(id);
    } catch (e) {
      // Guarded like persistBacklog's revert, and for the same reason.
      if (stillShowing(id)) setKg(prev);
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

  // Save what is in state now: the text fields' explicit Save button.
  async function persistSettings() {
    try {
      await saveSettings(settings);
      loadedRef.current = settings;
    } catch (e) {
      toastError(e, "Couldn't save settings");
    }
  }

  // Save-on-change for the controls with no blur to flush on (the default-agent
  // select, the worktree toggle), keeping `loadedRef` in step so the exit flush
  // sees nothing left to do.
  function persistSettingsNow(next: ProviderSettings): Promise<void> {
    setSettings(next);
    return saveSettings(next)
      .then(() => { loadedRef.current = next; })
      .catch((e) => toastError(e, "Couldn't save settings"));
  }

  // Empty value = "auto", persisted as null.
  function pickDefaultAgent(name: string) {
    persistSettingsNow({ ...settings, defaultAgent: name || null });
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
    // Empty resume field = no generic resume recipe. The agent then starts
    // fresh unless Agency named its conversation itself, which is a resume
    // recipe of its own and does not come from the profile (see
    // `state.rs::should_resume`).
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

  function renderProfileDialog() {
    return (
      <FormDialog
        title={editing ? `Edit ${agentLabel(editing)}` : "New agent profile"}
        subtitle="How Agency launches this coding agent in a worktree."
        submitLabel={editing ? "Save changes" : "Create profile"}
        submitDisabled={!draft.name.trim() || !draft.command.trim()}
        onSubmit={addProfile}
        onCancel={closeForm}
      >
        <Field
          label="Name"
          hint="What this agent is called in Agency's menus. Also its id, so keep it short and unique."
        >
          <input
            className="settings-input"
            autoFocus
            placeholder="claude"
            value={draft.name}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
          />
        </Field>
        <Field
          label="Command"
          hint={<>The executable Agency runs. It has to be on your <code>PATH</code>.</>}
        >
          <input
            className="settings-input mono"
            placeholder="claude"
            value={draft.command}
            onChange={(e) => setDraft({ ...draft, command: e.target.value })}
          />
        </Field>
        <Field
          label="Arguments"
          optional
          hint={<>Passed every time this agent starts. <code>{"{{prompt}}"}</code> is replaced with the task text.</>}
        >
          <input
            className="settings-input mono"
            placeholder="{{prompt}}"
            value={draft.args}
            onChange={(e) => setDraft({ ...draft, args: e.target.value })}
          />
        </Field>
        <Field
          label="Resume arguments"
          optional
          hint="Added after the arguments above when reopening an existing session, in place of the prompt. Leave empty if this CLI has no resume flag. Agents whose conversations Agency can name come back to their own either way, so emptying this won't start them fresh. To take that over, name the conversation yourself in the arguments above (--session-id, --resume, --continue) and Agency stands down."
        >
          <input
            className="settings-input mono"
            placeholder="--continue"
            value={draft.resume}
            onChange={(e) => setDraft({ ...draft, resume: e.target.value })}
          />
        </Field>
        <Field
          label="Loop arguments"
          optional
          hint="The headless, one-shot invocation used by Loop agent, where the agent runs unattended and exits. Leave empty if this agent can't do that."
        >
          <input
            className="settings-input mono"
            placeholder="-p {{prompt}} --permission-mode acceptEdits"
            value={draft.loop}
            onChange={(e) => setDraft({ ...draft, loop: e.target.value })}
          />
        </Field>
        <Field
          label="Environment variables"
          optional
          hint={<>Added to the agent's environment, one <code>KEY=VALUE</code> per line.</>}
        >
          <textarea
            className="settings-input mono"
            placeholder={"ANTHROPIC_MODEL=claude-opus-5\nMY_FLAG=1"}
            value={draft.env}
            onChange={(e) => setDraft({ ...draft, env: e.target.value })}
          />
        </Field>
      </FormDialog>
    );
  }

  // ── workspace (pinned notes/journal project) ──────────────────────────
  const [workspace, setWorkspace] = useState<Project | null>(null);
  const [wsDefault, setWsDefault] = useState("");
  const [wsGitless, setWsGitless] = useState(false);
  // Whether the folder itself is gone, kept apart from `wsGitless`: the two
  // agree about what Switch should do with git and disagree about what to say
  // to the user, since "Git is off" is not what is wrong with a workspace on a
  // disk that is unplugged.
  const [wsMissing, setWsMissing] = useState(false);
  const refreshWorkspace = () => {
    getWorkspace()
      .then(async (ws) => {
        setWorkspace(ws);
        if (ws) {
          const r = await inspectRepo(ws.repo_path).catch(() => null);
          // "missing" too, and deliberately: it used to be folded into
          // `notARepo`, and reading it as "this workspace has git" makes
          // Switch pass `git: true` for a workspace whose folder is only
          // unmounted, so the folder the user then picks gets a `git init` and
          // a first commit of everything already in it. Unknown has to mean
          // gitless here: that way round the mistake costs nothing.
          setWsGitless(r?.state === "notARepo" || r?.state === "missing");
          setWsMissing(r?.state === "missing");
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

  // Reconnect a workspace whose folder has moved. `relocate_project` keeps the
  // project id, so the journal, notes and issues come back with it, and it
  // repairs the agents' worktree links, which record absolute paths and so
  // travelled with the folder and broke. Neither Move nor Switch can do this:
  // Move renames a folder that is not there, and Switch repoints the row
  // without repairing anything.
  async function locateWorkspace() {
    if (!workspace) return;
    const sel = await openDialog({
      directory: true,
      multiple: false,
      title: "Locate the workspace folder",
    });
    if (typeof sel !== "string") return;
    try {
      setWorkspace(await relocateProject(workspace.id, sel));
      refreshWorkspace();
      // Same-value re-fire: makes ProjectTree refetch the repointed row.
      setWorkspaceHidden(wsOff);
      // And the app's own copy of the row, which that event does not reach: it
      // still held the old repo_path, so closing Settings remounted the view
      // against the folder that is not there and put the missing-folder screen
      // back up over the workspace just reconnected.
      notifyProjectsChanged();
      toastSuccess("Workspace reconnected");
    } catch (e) {
      toastError(e, "Couldn't reconnect the workspace");
    }
  }

  // Switch = point the workspace at a different folder (picked directly,
  // unlike Move which picks a parent and renames on disk). The old folder
  // stays untouched, so switching back is choosing it again; the idempotent
  // create_workspace repoints the row and seeds a fresh folder's guide.
  const [wsSwitch, setWsSwitch] = useState<string | null>(null);
  async function pickSwitchWorkspace() {
    if (!workspace) return;
    const sel = await openDialog({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    const dest = sel.replace(/\/+$/, "");
    if (dest === workspace.repo_path) return;
    setWsSwitch(dest);
  }
  async function doSwitchWorkspace(dest: string) {
    try {
      // A workspace whose folder is missing has usually moved, and the folder
      // being picked here is where it went, so relocate first: that is what
      // runs `WorktreeManager::repair()` over the agents' worktrees, whose
      // absolute git links travelled with the folder and are now stale.
      // `create_workspace` on its own repoints the row and repairs nothing, so
      // reconnecting this way left every agent's worktree broken. Against a
      // genuinely fresh folder it finds no worktrees and does nothing.
      if (wsMissing && workspace) await relocateProject(workspace.id, dest);
      // Keep the current git preference; a gitless workspace stays gitless
      // (Enable git stays one click away in this section).
      await createWorkspace(dest, !wsGitless);
      refreshWorkspace();
      // Same-value re-fire: makes ProjectTree refetch the repointed row.
      setWorkspaceHidden(wsOff);
      // The app's copy of the row too; see locateWorkspace above.
      notifyProjectsChanged();
      toastSuccess("Workspace switched");
    } catch (e) {
      toastError(e, "Couldn't switch workspace");
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

  // ── nav and search ─────────────────────────────────────────────────────
  const [query, setQuery] = useState("");
  const [active, setActive] = useState<SectionId>(section ?? lastSection);
  const pageRef = useRef<HTMLElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  // What the sections hold that the static `terms` can't: the agents and MCP
  // servers this machine actually has, and the project's name. Typing "linear"
  // or the project's own name is how people look for a server or a
  // project-scoped setting, and neither word exists until runtime.
  const extraTerms = useMemo<ExtraTerms>(
    () => ({
      messages: HUSHABLE.map((h) => h.label).join(" "),
      agents: profiles.map((p) => agentLabel(p.name)).join(" "),
      mcp: mcpServers.map((s) => s.name).join(" "),
      backlog: projectName ?? "",
      knowledge: projectName ?? "",
      // Diagnostics is a per-agent list of CLI versions and update commands, so
      // "claude version" is a question it answers and used to find nothing: the
      // section's own label says neither word, and the agents' names are not
      // static (this is whatever is installed on this machine).
      diagnostics: cliInfo.map((c) => agentLabel(c.agent)).join(" "),
      files: projectName ?? "",
    }),
    [profiles, mcpServers, projectName, cliInfo],
  );

  // null while the box is empty: no search is running, so the nav shows every
  // section and the page shows the selected one.
  const matched = useMemo(() => matchSections(query, extraTerms), [query, extraTerms]);

  // A search puts every match on screen at once rather than listing links to
  // them: the setting you were hunting for is then already there to change.
  const visible = (id: SectionId) => (matched ? matched.has(id) : active === id);

  function pickSection(id: SectionId) {
    lastSection = id;
    setActive(id);
    setQuery("");
    scrollRef.current?.scrollTo({ top: 0 });
  }

  // A deep link also has to record itself as `lastSection`, or closing Settings
  // and reopening it from the menu jumps back to wherever you were before the
  // link. Settings mounts fresh on every open, so this normally runs once.
  useEffect(() => {
    if (section) pickSection(section);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [section]);

  // ⌘F belongs to this box while Settings is on screen. Settings takes over
  // the whole content area, so nothing else searchable is visible behind it.
  useEffect(
    () =>
      registerFindTarget({
        host: () => pageRef.current,
        open: () => {
          searchRef.current?.focus();
          searchRef.current?.select();
        },
        canReplace: false,
        rank: FindRank.list,
      }),
    [],
  );

  // The agent the default-model row is for, or null when there is none to
  // name: no default agent chosen, its models not loaded yet, or an agent
  // whose CLI takes no model flag at all.
  const defaultModelAgent =
    settings.defaultAgent && models[settings.defaultAgent]?.supported
      ? settings.defaultAgent
      : null;

  return (
    <main className="settings-page" ref={pageRef}>
      {/* The section rail. It stands where the project tree does one pane over,
          and for the same reason: thirteen sections on one scroll is a page you
          read top to bottom looking for something, not one you navigate. */}
      <div className="settings-rail">
        <button className="settings-back" onClick={onClose}>← Back</button>
        <h1 className="settings-title">Settings</h1>
        <input
          ref={searchRef}
          className="settings-search-input"
          type="text"
          aria-label="Search settings"
          placeholder="Search settings"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Escape") setQuery(""); }}
        />
        <nav className="settings-nav" aria-label="Settings sections">
          {GROUPS.map((g) => {
            const items = SECTIONS.filter((s) => s.group === g.id && (!matched || matched.has(s.id)));
            if (items.length === 0) return null;
            return (
              <div className="settings-nav-group" key={g.id}>
                <div className="settings-nav-group-head">
                  <span className="settings-nav-group-label">{g.label}</span>
                  {/* The whole point of the grouping: the project group is
                      labelled with the project it is about, so a setting that
                      only applies to one checkout never reads as a preference. */}
                  <span className="settings-scope-tag" data-scope={g.scope}>
                    {g.scope === "global" ? "Global" : projectName ?? "No project"}
                  </span>
                </div>
                {items.map((s) => (
                  <button
                    key={s.id}
                    type="button"
                    className="settings-nav-item"
                    data-active={!matched && s.id === active}
                    // data-active is the styling hook and says nothing to a
                    // screen reader; aria-current is what announces which of
                    // the thirteen is the one on screen.
                    aria-current={!matched && s.id === active ? "true" : undefined}
                    onClick={() => pickSection(s.id)}
                  >
                    {s.label}
                  </button>
                ))}
              </div>
            );
          })}
        </nav>
      </div>
      <div className="settings-scroll" ref={scrollRef}>
        <div className="settings-inner">
          {error && <div className="git-error">{error}</div>}
          {matched?.size === 0 && (
            <p className="settings-no-match">
              No settings match "{query.trim()}". Search by feature, by an agent's name, or by
              the name of an MCP server.
            </p>
          )}

          {visible("appearance") && (
            <section className="settings-section">
              <SectionHead id="appearance" projectName={projectName} />
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
          )}

          {visible("editor") && (
            <section className="settings-section">
              <SectionHead id="editor" projectName={projectName} />
              <div className="settings-group-card">
                <div className="settings-notif-row">
                  <span className="settings-notif-label">Word wrap in file viewer</span>
                  <Toggle checked={wordWrap} onChange={pickWordWrap} />
                </div>
                <div className="settings-notif-row">
                  <span className="settings-notif-label">Let agents see the file you have open</span>
                  <Toggle
                    checked={settings.shareOpenFile}
                    onChange={(next) => {
                      const saved = persistSettingsNow({ ...settings, shareOpenFile: next });
                      // The backend dropped every report made while this was
                      // off, so turning it on has to hand it the file the user
                      // already has open. Only after the save lands, or that
                      // report is dropped as well.
                      if (next) saved.then(resendOpenFile);
                    }}
                  />
                </div>
              </div>
              <p className="settings-section-hint">
                Off by default. On, an agent can ask Agency which file or note you
                have in focus, so "fix this file" means the one in front of you. It
                learns the path, never the contents, and only agents working in the
                same project as the file.
              </p>
              <p className="settings-section-hint">
                Agents pick the tool up when they start, and only agents with their
                own worktree: that is the one place Agency writes an agent's tool
                config. Switching this off stops it answering right away.
              </p>
            </section>
          )}

          {visible("messages") && (
            <section className="settings-section">
              <SectionHead id="messages" projectName={projectName} />
              <p className="settings-section-hint">
                Explanations you can switch off once you know the workflow, and switch back on here.
                Warnings about work that can't be recovered always show.
              </p>
              <div className="settings-group-card">
                {HUSHABLE.map((h) => (
                  <div key={h.id} className="settings-notif-row">
                    <span className="settings-notif-label" title={h.hint}>{h.label}</span>
                    <Toggle
                      checked={!hushed.includes(h.id)}
                      onChange={(next) => pickMessage(h.id, next)}
                    />
                  </div>
                ))}
              </div>
            </section>
          )}

          {visible("notifications") && (
            <section className="settings-section">
              <SectionHead id="notifications" projectName={projectName} />
              <div className="settings-group-card">
                {([
                  ["agentIdle", "Agent finished a turn"],
                  ["agentFinished", "Agent exited"],
                  ["runCrashed", "Run script crashed"],
                  ["mergeAttention", "Merge needs attention"],
                  ["loopEvents", "Loop complete or stalled"],
                  [
                    "onlyWhenWatching",
                    "Skip the agent you're watching",
                    "No notification for the agent open in front of you. Every other agent still notifies, even while you're using Agency.",
                  ],
                ] as [keyof NotifSettings, string, string?][]).map(([key, label, hint]) => (
                  <div key={key} className="settings-notif-row">
                    <span className="settings-notif-label" title={hint}>{label}</span>
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
          )}

          {visible("agents") && (
            <section className="settings-section">
              <SectionHead id="agents" projectName={projectName} />
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
                {/* A model id only means anything for one agent, so this row is
                    the default agent's. On Auto the agent isn't known until a
                    spawn picks one, and it renders nothing for a CLI that takes
                    no model flag, the same case ModelSelect itself sits out. */}
                {defaultModelAgent && (
                  <div className="settings-notif-row">
                    <span className="settings-notif-label">Default model for new tasks</span>
                    <ModelSelect
                      info={models[defaultModelAgent]}
                      projectId={projectId}
                      value={models[defaultModelAgent]?.selected ?? null}
                      onChange={(model) =>
                        setAgentModel(defaultModelAgent, model)
                          .then(reloadModels)
                          .catch((e) => toastError(e, "Couldn't set the default model"))
                      }
                    />
                  </div>
                )}
                <div className="settings-notif-row">
                  <span className="settings-notif-label">Give new agents their own worktree</span>
                  <Toggle
                    checked={settings.defaultWorktree}
                    onChange={(next) => persistSettingsNow({ ...settings, defaultWorktree: next })}
                  />
                </div>
              </div>
              <p className="settings-section-hint">
                A default model belongs to one agent, so that row appears once you
                have chosen a default agent above; on Auto each agent keeps the
                model it last ran on. Every menu that starts an agent still offers
                the full list, and starting one on another model moves that agent's
                default there.
              </p>
              <p className="settings-section-hint">
                Off means new agents work in the project checkout, on the branch you
                have open, with no branch of their own to merge. This sets how the
                add-agent menu starts; the Own worktree box there still decides each
                spawn. Races, loops and issue dispatch always take a worktree.
              </p>
              <div className="settings-card-list">
                {profiles.map((p) => (
                  <div key={p.name} className="settings-profile-card">
                    <div className="settings-profile-head">
                      <span className="agent-dot" style={{ background: agentColor(p.name) }} />
                      <span className="settings-profile-name">{agentLabel(p.name)}</span>
                      <code className="settings-profile-cmd">{p.command}</code>
                      <span className="spacer" />
                      <button className="settings-ghost-btn" onClick={() => editProfile(p)}>Edit</button>
                      <button className="settings-ghost-btn settings-del-btn" onClick={() => deleteProfile(p.name).then(refresh).catch((e) => toastError(e, "Couldn't delete profile"))}>Delete</button>
                    </div>
                    <div className="settings-profile-meta">
                      {([
                        ["Arguments", p.args.join(" ")],
                        ["Resume", (p.resume_args ?? []).join(" ")],
                        ["Loop", (p.loop_args ?? []).join(" ")],
                        ["Environment", p.env.map(([k]) => k).join(", ")],
                      ] as [string, string][])
                        .filter(([, val]) => val)
                        .map(([key, val]) => (
                          <Fragment key={key}>
                            <span className="settings-meta-key">{key}</span>
                            <code className="settings-meta-val" title={val}>{val}</code>
                          </Fragment>
                        ))}
                    </div>
                  </div>
                ))}
              </div>
              {catalog.some((e) => e.enabled && !e.acceptsPrompt) && (
                <p className="settings-section-hint">
                  {catalog.filter((e) => e.enabled && !e.acceptsPrompt).map((e) => agentLabel(e.id)).join(", ")}{" "}
                  take no opening prompt on the command line, so a dispatched issue or review starts
                  them promptless in the worktree and the ask has to go into their terminal by hand.
                </p>
              )}
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
            </section>
          )}

          {visible("mcp") && (
            <section className="settings-section">
              <SectionHead id="mcp" projectName={projectName} />
              <p className="settings-section-hint">
                Emitted into each agent workspace, in the agent's native config format (
                {catalog.filter((e) => e.supportsMcp).map((e) => agentLabel(e.id)).join(", ") ||
                  "Claude Code, Copilot CLI, Cursor, OpenCode"}
                ). Copilot is handed its file on the command line, since it waits for folder trust
                before reading workspace config on its own.
                Servers that need an OAuth sign-in can't live in workspace config; use{" "}
                <b>Authenticate</b> to register one with an agent's own CLI instead. That agent then
                reads it from its user config, and Agency keeps emitting it for the others. Projects
                can add their own via <code>[mcp.servers]</code> in <code>.agency/agency.toml</code>;
                project entries win on name conflicts.
              </p>
              {catalog.some((e) => e.enabled && !e.supportsMcp) && (
                <p className="settings-section-hint">
                  {catalog.filter((e) => e.enabled && !e.supportsMcp).map((e) => agentLabel(e.id)).join(", ")}{" "}
                  manage MCP servers in their own global config and won't see the servers below.
                </p>
              )}
              <div className="settings-card-list">
                {mcpServers.map((s) => {
                  // Agents that could still take this server at their own user scope:
                  // remote servers only, minus the ones already registered with it.
                  const authable = s.url
                    ? catalog.filter((e) => e.supportsMcpAuth && !s.userScopeAgents.includes(e.id))
                    : [];
                  return (
                  <div key={s.name} className="settings-profile-card">
                    <div className="settings-profile-head">
                      <span className="settings-profile-name">{s.name}</span>
                      <span className="mcp-transport-tag">{s.url ? (s.transport ?? "http") : "local"}</span>
                      <span className="spacer" />
                      {authable.length > 0 && (
                        <div className="settings-add-dropdown">
                          <button
                            className="settings-ghost-btn"
                            disabled={!projectId}
                            title={
                              projectId
                                ? "Register this server with an agent's own CLI and sign in"
                                : "Select a project first"
                            }
                            onClick={() => setMcpAuthMenu((n) => (n === s.name ? null : s.name))}
                          >
                            Authenticate ▾
                          </button>
                          {mcpAuthMenu === s.name && (
                            <>
                              <div className="settings-menu-backdrop" onClick={() => setMcpAuthMenu(null)} />
                              <div className="settings-menu">
                                {authable.map((e) => (
                                  <button
                                    key={e.id}
                                    // Registering shells out to the agent's own CLI, so
                                    // an uninstalled one can only fail — say so up front.
                                    disabled={!e.installed}
                                    title={e.installed ? undefined : `${e.command} is not on PATH`}
                                    onClick={() => authenticateMcp(e.id, s.name)}
                                  >
                                    <span className="agent-dot" style={{ background: agentColor(e.id) }} />
                                    <span className="settings-menu-name">{agentLabel(e.id)}</span>
                                    <code className="settings-menu-cmd">
                                      {e.installed ? `${e.command} mcp add` : "not installed"}
                                    </code>
                                  </button>
                                ))}
                              </div>
                            </>
                          )}
                        </div>
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
                          <span className="settings-meta-key">url</span>
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
                    {s.userScopeAgents.length > 0 && (
                      <div className="mcp-auth-row">
                        <span className="settings-meta-key">signed in via</span>
                        {s.userScopeAgents.map((id) => (
                          <button
                            key={id}
                            className="mcp-auth-chip"
                            title={`Registered with ${agentLabel(id)} at user scope, so Agency does not write it into ${agentLabel(id)} worktrees. Click to undo.`}
                            onClick={() => deauthenticateMcp(id, s.name)}
                          >
                            <span className="agent-dot" style={{ background: agentColor(id) }} />
                            {agentLabel(id)}
                            <span className="mcp-auth-chip-x">✕</span>
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                  );
                })}
              </div>
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
            </section>
          )}

          {visible("localModel") && (
            <section className="settings-section">
              <SectionHead id="localModel" projectName={projectName} />
              <p className="settings-section-hint">
                Sets <code>OPENAI_BASE_URL</code> and a placeholder <code>OPENAI_API_KEY</code> in
                every agent session, run script and terminal Agency opens, so OpenAI-protocol tools
                reach a server on this machine instead of OpenAI. Agents with their own login (Claude,
                Codex, …) ignore it. It also becomes a model the knowledge graph can build on. Blank is
                the default and sets neither variable; start the server before you rely on it.
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
                      placeholder="http://localhost:1234/v1"
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
          )}

          {visible("backlog") && (
            <section className="settings-section">
              <SectionHead id="backlog" projectName={projectName} />
              <p className="settings-section-hint">
                Shares this project's issues through its git remote, so the same backlog is on every
                machine you work from, and on your team's if you have one. Issues travel on a ref of
                their own that is never checked out, so nothing lands in a branch, in a diff or in a
                pull request. Status, priority and attachments all travel. Sync by hand from the
                issues view, or turn on automatic sync below.
              </p>
              {!projectId ? (
                <div className="settings-group-card">
                  <span className="settings-notif-label">Pick a project in the sidebar to configure its backlog.</span>
                </div>
              ) : backlog ? (
                <div className="settings-group-card">
                  <div className="settings-notif-row">
                    <span className="settings-notif-label">Issue key</span>
                    <input
                      className="settings-input"
                      style={{ maxWidth: 120, textTransform: "uppercase" }}
                      value={backlogKey}
                      maxLength={8}
                      placeholder="AGE"
                      onChange={(e) => setBacklogKey(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") void previewKeyRename();
                        else if (e.key === "Escape") setBacklogKey(backlog.issueKey);
                      }}
                    />
                    <button
                      className="settings-ghost-btn"
                      disabled={!pendingKey}
                      onClick={() => void previewKeyRename()}
                    >
                      Rename
                    </button>
                  </div>
                  {/* The whole reason this is editable. The key is derived from
                      the project name and shifted off any key already in use,
                      so a second checkout of one repo lands on a different key
                      than the backlog it is about to sync with, and then hides
                      every issue in it. */}
                  <p className="settings-section-hint">
                    Numbers issues in this project, as in <code>{backlogKey || "AGE"}-14</code>, and
                    names their files. A shared backlog has a key of its own: if this one does not
                    match it, the issues sync but stay off the board. Changing it renames this
                    project's issue files and the links between them. Links to them from other
                    projects and from notes keep the old key.
                  </p>
                  {backlog.keySharedWith.length > 0 && (
                    <p className="settings-section-hint">
                      {backlog.keySharedWith.join(", ")} also use{backlog.keySharedWith.length === 1 ? "s" : ""}{" "}
                      <code>{backlog.issueKey}</code>. That is what you want for two checkouts of one
                      repo sharing a backlog; otherwise a <code>[[{backlog.issueKey}-14]]</code> link
                      can only reach one of them.
                    </p>
                  )}
                  <div className="settings-notif-row">
                    <span className="settings-notif-label">
                      Share the backlog{projectName ? ` for ${projectName}` : ""}
                    </span>
                    <Toggle
                      checked={backlog.sync}
                      onChange={(next) => persistBacklog({ sync: next })}
                    />
                  </div>
                  {backlog.remotes.length === 0 ? (
                    <p className="settings-section-hint">
                      This project has no git remote, so there is nowhere to share a backlog to yet.
                    </p>
                  ) : (
                    backlog.sync && (
                      <>
                        <div className="settings-notif-row">
                          <span className="settings-notif-label">Remote</span>
                          <select
                            className="settings-input"
                            style={{ maxWidth: 220 }}
                            value={
                              backlog.remotes.includes(backlogRemote) ? backlogRemote : "__custom"
                            }
                            onChange={(e) => {
                              const next = e.target.value === "__custom" ? "" : e.target.value;
                              setBacklogRemote(next);
                              if (next) persistBacklog({ sync: true, remote: next });
                            }}
                          >
                            {backlog.remotes.map((r) => (
                              <option key={r} value={r}>
                                {r}
                              </option>
                            ))}
                            <option value="__custom">Another remote…</option>
                          </select>
                        </div>
                        {!backlog.remotes.includes(backlogRemote) && (
                          <input
                            className="settings-input"
                            value={backlogRemote}
                            placeholder="Remote name or URL"
                            onChange={(e) => setBacklogRemote(e.target.value)}
                            onBlur={() =>
                              backlogRemote.trim() && persistBacklog({ sync: true, remote: backlogRemote })
                            }
                          />
                        )}
                        {/* The one thing a reader can't check for themselves, and
                            the case this repo is in: public code, private backlog. */}
                        <p className="settings-section-hint">
                          Anyone who can read <code>{backlogRemote || "this remote"}</code> can read the
                          backlog. If the repository is public, point this at a private remote instead.
                        </p>
                        <div className="settings-notif-row">
                          <span className="settings-notif-label">Sync automatically</span>
                          <Toggle
                            checked={backlog.auto}
                            onChange={(next) => persistBacklog({ auto: next })}
                          />
                        </div>
                        {/* What it costs and when it interrupts, both stated: a
                            pass is two network round-trips, and the merge
                            decides a contested field by itself. Someone turning
                            this on is agreeing to both. */}
                        <p className="settings-section-hint">
                          Runs a pass when you open this backlog, then every couple of minutes while
                          it is on screen, so a teammate moving an issue to In progress turns up here
                          without your asking. It stays quiet unless the merge had to decide
                          something: when you have both changed the same field of the same issue, the
                          later edit wins, and you get told which issues that happened to.
                        </p>
                      </>
                    )
                  )}
                  {backlog.fromRepo && (
                    <p className="settings-section-hint">
                      This project's <code>agency.toml</code> asks for a shared backlog, so anyone who
                      clones it gets this on by default. Turning it off here only affects this machine.
                    </p>
                  )}
                  <p className="settings-section-hint">
                    Saved to this machine only (<code>.agency/agency.local.toml</code>). To share the
                    choice with the repository, commit <code>[issues] sync = true</code> into{" "}
                    <code>.agency/agency.toml</code>.
                  </p>
                </div>
              ) : null}
            </section>
          )}

          {visible("knowledge") && (
            <section className="settings-section">
              <SectionHead id="knowledge" projectName={projectName} />
              <p className="settings-section-hint">
                Builds a graphify code-knowledge graph for this project and exposes it to every agent as an
                MCP server, rebuilding after each clean merge. Agents only get the server once a graph
                exists, and the first one is built when you ask for it. Saved to this machine only
                (<code>.agency/agency.local.toml</code>), not shared with the team. Needs the{" "}
                <code>graphify</code> / <code>uv</code> tooling on your <code>PATH</code>. Code is
                indexed locally; your docs are read by whichever model you pick below, and nothing
                runs until you build. A first build of a large project takes minutes; its output
                appears here while it runs, and you can stop it.
              </p>
              {!projectId ? (
                <div className="settings-group-card">
                  <span className="settings-notif-label">Pick a project in the sidebar to configure its knowledge graph.</span>
                </div>
              ) : kg ? (
                <div className="settings-group-card">
                  <div className="settings-notif-row">
                    <span className="settings-notif-label">
                      Enable knowledge graph{projectName ? ` for ${projectName}` : ""}
                    </span>
                    <Toggle checked={kg.graph} onChange={(graph) => persistKnowledge({ graph })} />
                  </div>
                  {kg.graph && (
                    <>
                      {/* Also offered after a failed build, not just when the
                          commands are missing: an install made before the extras
                          were pinned leaves `graphify` right there on the PATH and
                          still fails every build on a package it does not have, and
                          there is otherwise nowhere left to repair that from. */}
                      {(!kg.serve_installed || !kg.build_installed || !!kg.last_build_error) && (
                        <div className="settings-kg-warn">
                          <div>
                            {!kg.serve_installed && !kg.build_installed
                              ? "The serve and build commands aren't on your PATH. The graph is enabled but will be skipped until the tooling is installed."
                              : !kg.serve_installed
                              ? "The serve command isn't on your PATH. The graph is enabled but will be skipped until the tooling is installed."
                              : !kg.build_installed
                              ? "The build command isn't on your PATH. The graph is enabled but will be skipped until the tooling is installed."
                              : "A build can also fail on a package an older install of the tooling is missing."}{" "}
                            Agency can install it for you in a terminal:
                          </div>
                          <pre className="install-cmd">{kg.install_command}</pre>
                          <div className="settings-kg-actions">
                            <button className="settings-secondary" onClick={installKgTooling}>
                              Install tooling
                            </button>
                          </div>
                        </div>
                      )}
                      {/* AGE-180: a build of this repository runs for ten minutes.
                          Saying only "Building the graph" for all of it left no way
                          to tell a working build from a wedged one, so the elapsed
                          time and the build's own output are both on screen. */}
                      {kg.build_installed && (
                        <div className={kg.graph_built && !kg.last_build_error ? "settings-kg-note" : "settings-kg-warn"}>
                          {kg.building
                            ? kg.graph_built
                              ? `Rebuilding the graph, ${fmtDur((kg.build_elapsed_secs ?? 0) * 1000)} so far. Agents keep using the graph that is already there until this one finishes.`
                              : `Building the graph, ${fmtDur((kg.build_elapsed_secs ?? 0) * 1000)} so far. Agents started after it finishes will get the knowledge-graph server.`
                            : kg.last_build_error
                            ? `The last graph build failed${buildTook(kg)}: ${kg.last_build_error}`
                            : kg.last_build_stopped
                            ? `You stopped the last build${buildTook(kg)}. Agents keep using whatever graph was built before it.`
                            : kg.graph_built
                            ? `Graph built at ${kg.graph_path}, and kept out of git.${
                                kg.last_build_secs === null
                                  ? ""
                                  : ` The last build took ${fmtDur(kg.last_build_secs * 1000)}.`
                              }`
                            : "No graph has been built yet, so agents get no knowledge-graph server. Build one to start using it."}
                        </div>
                      )}
                      {/* The build's own output, which is the only thing that says
                          what it is doing right now. Kept after it ends: a failure's
                          last lines are the context for the reason above. */}
                      {kg.build_log.length > 0 && (
                        <pre
                          className="settings-kg-log"
                          ref={kgLogRef}
                          onScroll={(e) => {
                            const el = e.currentTarget;
                            kgLogFollow.current =
                              el.scrollHeight - el.scrollTop - el.clientHeight < 8;
                          }}
                        >
                          {kg.build_log.join("\n")}
                        </pre>
                      )}
                      {kgBuildError && <div className="settings-kg-warn">{kgBuildError}</div>}
                      {/* The picker and its note are the "before it runs" half of
                          this feature: a build reads every doc in the project with
                          whichever model is named here, so what that costs and who
                          receives the docs is on screen before the Build button is
                          reachable. Choosing writes the build command below, which
                          stays editable for anything the picker doesn't cover. */}
                      <div className="settings-provider-field">
                        <label className="settings-field-key">model</label>
                        <PillSelect
                          value={kg.build_backend}
                          options={backendOptions}
                          onChange={(id) => {
                            // "Custom" is a readout of a hand-written build
                            // command, not something to switch to: composing
                            // `--backend custom` would just break the build.
                            if (id !== "custom") chooseBackend(id, "");
                          }}
                        />
                        {/* No name to give a build that runs no model at all. */}
                        {pickedBackend && pickedBackend.id !== "code-only" && (
                          // The claude CLI is the one backend whose models Agency
                          // already knows, so its model is picked from the same
                          // menu every agent uses rather than typed from memory.
                          // Picking "Agent's default" there writes this backend's
                          // own default (haiku) into the command, not an unnamed
                          // model: the build is a per-file `claude -p` loop, and an
                          // unnamed one answering on the plan's default spent a
                          // 5-hour usage window in 30 minutes. Every other backend
                          // takes a typed id, since what those can run is the
                          // vendor's business and not a list Agency holds.
                          claudeCliModels ? (
                            <ModelSelect
                              info={claudeCliModels}
                              projectId={projectId}
                              value={kg.build_model || null}
                              onChange={(model) => chooseBackend(pickedBackend.id, model ?? "")}
                            />
                          ) : (
                            <input
                              className="settings-field-input"
                              placeholder={pickedBackend.default_model || "name the model it serves"}
                              value={kgModel}
                              onChange={(e) => setKgModel(e.target.value)}
                              // On blur, not on keystroke: a half-typed model name is
                              // not a choice, and every save rewrites the command.
                              onBlur={() => {
                                if (kgModel.trim() !== kg.build_model) {
                                  chooseBackend(pickedBackend.id, kgModel.trim());
                                }
                              }}
                              onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                            />
                          )
                        )}
                      </div>
                      <div className="settings-kg-cost">
                        {pickedBackend
                          ? pickedBackend.note
                          : "This build command was written by hand, so the picker leaves it alone. Clear it to go back to a listed model."}
                      </div>
                      {/* What size of model this actually needs, which is the
                          question the picker raises and used to leave unanswered.
                          Stated once rather than per backend, because it is a fact
                          about the job and not about the vendor. */}
                      <div className="settings-kg-cost">
                        The model only reads docs, papers and images. Code is indexed locally by
                        tree-sitter, so this is a strict-format extraction job rather than a reasoning
                        one, and a light model is the right default. Reach for a heavier one when the
                        substance of the project is in its prose, or when you have edited the build
                        command to run deep mode.
                      </div>
                      {/* A command saved before the model was part of this choice.
                          It leaves the model to the CLI, which answered on the
                          plan's default and spent a 5-hour usage window on one
                          build, so it is called out rather than quietly rewritten:
                          picking a model above is what fixes it. */}
                      {pickedBackend?.id === "claude-cli" && !kg.build_model && (
                        <div className="settings-kg-warn">
                          This build names no model, so it runs on whatever claude defaults to. Pick
                          one above. A build reads every file in the project, and a large model can
                          spend a whole usage window on a single build.
                        </div>
                      )}
                      {/* The only build nobody presses. It runs the command above
                          over the whole project after every clean merge, so on a
                          busy day it is the largest thing this feature spends, and
                          it is a switch rather than something to find out about
                          afterwards. On by default: a graph that has stopped
                          matching the code is worse than no graph. */}
                      <div className="settings-notif-row">
                        <span className="settings-notif-label">Rebuild after every merge</span>
                        <Toggle
                          checked={kg.rebuild_on_merge}
                          onChange={(rebuild_on_merge) => persistKnowledge({ rebuild_on_merge })}
                        />
                      </div>
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
                        {kg.building ? (
                          <button className="settings-secondary" onClick={stopBuild}>
                            Stop build
                          </button>
                        ) : (
                          <button
                            className="settings-secondary"
                            disabled={!kg.build_installed}
                            onClick={buildGraph}
                          >
                            {kg.graph_built ? "Rebuild graph" : "Build graph"}
                          </button>
                        )}
                        <button className="settings-save" onClick={() => persistKnowledge({})}>Save commands</button>
                      </div>
                    </>
                  )}
                </div>
              ) : null}
            </section>
          )}

          {visible("files") && (
            <section className="settings-section">
              <SectionHead id="files" projectName={projectName} />
              <p className="settings-section-hint">
                Files copied into every new agent worktree. Git worktrees only contain
                committed files, so untracked essentials (local certs, service-account
                keys) must be listed here to reach agents. Untracked{" "}
                <code>.env</code> / <code>.env.*</code> files in the repo root are copied
                automatically. Saved to this machine only (<code>.agency/agency.local.toml</code>).
              </p>
              {!projectId ? (
                <div className="settings-group-card">
                  <span className="settings-notif-label">Pick a project in the sidebar to configure its worktree files.</span>
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
          )}

          {visible("workspace") && (
            <section className="settings-section">
              <SectionHead id="workspace" projectName={projectName} />
              <p className="settings-section-hint">
                Your home for journaling, planning, and cross-project notes: plain
                markdown files on disk. <kbd>{shortcutLabel("⌘⇧D")}</kbd> opens today's journal note.
              </p>
              <div className="settings-group-card">
                <div className="settings-notif-row">
                  <span className="settings-notif-label">
                    Show the workspace
                    {wsOff && " (currently hidden; nothing on disk was deleted)"}
                  </span>
                  <Toggle checked={!wsOff} onChange={(on) => { void toggleWorkspaceVisible(on); }} />
                </div>
                {!wsOff && (workspace ? (
                  <>
                    <div className="settings-notif-row">
                      <span className="settings-notif-label">
                        Location: <code className="settings-meta-val">{workspace.repo_path}</code>
                      </span>
                      <span style={{ display: "flex", gap: 8 }}>
                        <button
                          className="settings-save"
                          disabled={wsMissing}
                          title={wsMissing
                            ? "The workspace folder isn't there, so there is nothing to move"
                            : "Move this folder somewhere else on disk"}
                          onClick={doMoveWorkspace}
                        >Move…</button>
                        <button
                          className="settings-save"
                          title="Use a different folder as the workspace; this one stays on disk"
                          onClick={() => { void pickSwitchWorkspace(); }}
                        >Switch…</button>
                      </span>
                    </div>
                    {/* The folder is gone, so this section's other two actions
                        are the wrong ones: Move can only fail on a folder that
                        is not there, and Switch is for giving up on it. */}
                    {wsMissing && (
                      <div className="settings-notif-row">
                        <span className="settings-notif-label">
                          This folder isn't there. It may have been moved or renamed, or it may
                          be on a disk that isn't connected. Nothing has been lost: point Agency
                          at it again and your journal, notes and issues come back with it.
                        </span>
                        <button
                          className="settings-save"
                          onClick={() => { void locateWorkspace(); }}
                        >Locate folder…</button>
                      </div>
                    )}
                    {wsGitless && !wsMissing && (
                      <div className="settings-notif-row">
                        <span className="settings-notif-label">
                          Git is off, so agents work directly in the folder: no branches,
                          no Source Control, nothing to merge. Initialize a repository to
                          give each one its own branch; nothing is ever pushed anywhere.
                        </span>
                        <button className="settings-save" onClick={enableWorkspaceGit}>Enable git</button>
                      </div>
                    )}
                  </>
                ) : (
                  <div className="settings-notif-row">
                    <span className="settings-notif-label">
                      Not created yet. By default it will live at{" "}
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
          )}

          {visible("diagnostics") && (
            <section className="settings-section">
              <SectionHead id="diagnostics" projectName={projectName} />
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
                {/* Facts about each agent's CLI, with no staleness verdict: the
                    vendors' own update banners already carry that with real data
                    (AGE-146). The copy offer exists because the right update
                    command depends on how the CLI was installed, which the user
                    often does not remember and the resolved path does. */}
                {cliInfo.map((c) => {
                  const cmd = updateCommand(c);
                  return (
                    <div key={c.agent} className="settings-notif-row">
                      <span className="settings-notif-label" title={c.path ?? undefined}>
                        {agentLabel(c.agent)}
                        {c.version ? <> <code className="settings-meta-val">{c.version}</code></> : null}
                      </span>
                      {cmd ? (
                        <button
                          className="settings-ghost-btn"
                          title={cmd}
                          onClick={() => {
                            navigator.clipboard.writeText(cmd)
                              .then(() => toastSuccess(`Copied: ${cmd}`))
                              .catch(() => {});
                          }}
                        >Copy update command</button>
                      ) : !c.path ? (
                        <span className="settings-notif-label">Not found on PATH</span>
                      ) : null}
                    </div>
                  );
                })}
                <div className="settings-notif-row">
                  <span className="settings-notif-label">Log files</span>
                  <button className="settings-ghost-btn" onClick={openLogs}>Open logs</button>
                </div>
                <div className="settings-notif-row">
                  <span className="settings-notif-label">Report an issue</span>
                  <button
                    className="settings-ghost-btn"
                    onClick={() => { openUrl("https://github.com/TennnisAI/Agency/issues/new").catch(() => {}); }}
                  >Open GitHub issues</button>
                </div>
              </div>
              <p className="settings-section-hint">
                Agent versions are read from the CLIs on your machine, and each
                update command matches how that CLI was installed: npm, Homebrew,
                or the vendor's own installer. Agency never updates an agent
                itself.
              </p>
            </section>
          )}

          {visible("about") && (
            <section className="settings-section">
              <SectionHead id="about" projectName={projectName} />
              <div className="settings-group-card">
                <div className="settings-notif-row">
                  <span className="settings-notif-label">Licence</span>
                  <span className="settings-notif-label">Apache-2.0</span>
                </div>
                <div className="settings-notif-row">
                  <span className="settings-notif-label">Third-party notices</span>
                  <button className="settings-ghost-btn" onClick={() => setNoticesOpen(true)}>
                    View notices
                  </button>
                </div>
                <div className="settings-notif-row">
                  <span className="settings-notif-label">Source code</span>
                  <button
                    className="settings-ghost-btn"
                    onClick={() => { openUrl("https://github.com/TennnisAI/Agency").catch(() => {}); }}
                  >Open GitHub</button>
                </div>
              </div>
              <p className="settings-section-hint">
                Agency is built on Rust crates, JavaScript packages and two
                typefaces other people wrote. The notices list every one of them
                and the licence it comes under. The coding agents are not in that
                list: they are separate programs you install yourself, under their
                own licences.
              </p>
            </section>
          )}
        </div>
      </div>

      {noticesOpen && <NoticesDialog onClose={() => setNoticesOpen(false)} />}

      {formOpen && renderProfileDialog()}

      {mcpFormOpen && (
        <FormDialog
          title={mcpEditing ? `Edit ${mcpEditing}` : "New MCP server"}
          subtitle="A tool server Agency writes into every agent workspace."
          submitLabel={mcpEditing ? "Save changes" : "Add server"}
          submitDisabled={
            !mcpDraft.name.trim() ||
            (mcpDraft.transport === "stdio" ? !mcpDraft.command.trim() : !mcpDraft.url.trim())
          }
          onSubmit={addMcpServer}
          onCancel={() => { setMcpFormOpen(false); setMcpDraft(emptyMcpDraft); setMcpEditing(null); }}
        >
          {/* Name and transport share a row: the transport picker decides which
              fields follow, so it belongs beside the name, not buried below. */}
          <Field label="Server name" hint="Identifies the server in each agent's config file.">
            <div className="mcp-name-row">
              <input
                className="settings-input"
                autoFocus
                placeholder="linear"
                value={mcpDraft.name}
                onChange={(e) => setMcpDraft({ ...mcpDraft, name: e.target.value })}
              />
              <div className="mcp-seg" role="group" aria-label="Transport">
                {MCP_TRANSPORTS.map((t) => (
                  <button
                    key={t.id}
                    type="button"
                    className={mcpDraft.transport === t.id ? "on" : ""}
                    aria-pressed={mcpDraft.transport === t.id}
                    onClick={() => setMcpDraft({ ...mcpDraft, transport: t.id })}
                  >
                    {t.label}
                  </button>
                ))}
              </div>
            </div>
          </Field>

          {mcpDraft.transport === "stdio" ? (
            <>
              <div className="field-split">
                <Field label="Command" hint="The executable Agency tells the agent to run.">
                  <input
                    className="settings-input mono"
                    placeholder="npx"
                    value={mcpDraft.command}
                    onChange={(e) => setMcpDraft({ ...mcpDraft, command: e.target.value })}
                  />
                </Field>
                <Field label="Arguments" optional hint="Space separated, passed to the command.">
                  <input
                    className="settings-input mono"
                    placeholder="-y @modelcontextprotocol/server-github"
                    value={mcpDraft.args}
                    onChange={(e) => setMcpDraft({ ...mcpDraft, args: e.target.value })}
                  />
                </Field>
              </div>
              <Field label="Environment variables" optional hint="Added to the server's environment.">
                <KeyValueRows
                  pairs={mcpDraft.env}
                  onChange={(env) => setMcpDraft({ ...mcpDraft, env })}
                  keyPlaceholder="GITHUB_TOKEN"
                  valuePlaceholder="ghp_…"
                  addLabel="Add variable"
                />
              </Field>
            </>
          ) : (
            <>
              <Field
                label="URL"
                hint={
                  mcpDraft.transport === "sse"
                    ? "The server's SSE endpoint."
                    : "The server's streamable-HTTP endpoint."
                }
              >
                <input
                  className="settings-input mono"
                  placeholder={
                    mcpDraft.transport === "sse"
                      ? "https://mcp.example.com/sse"
                      : "https://mcp.example.com/mcp"
                  }
                  value={mcpDraft.url}
                  onChange={(e) => setMcpDraft({ ...mcpDraft, url: e.target.value })}
                />
              </Field>
              <Field label="Headers" optional hint="Sent with every request to the server.">
                <KeyValueRows
                  pairs={mcpDraft.headers}
                  onChange={(headers) => setMcpDraft({ ...mcpDraft, headers })}
                  keyPlaceholder="Authorization"
                  valuePlaceholder="Bearer sk-…"
                  addLabel="Add header"
                />
              </Field>
              <p className="settings-section-hint">
                Signing in with OAuth instead of a header? Add the server, then use{" "}
                <b>Authenticate</b> on its card to register it with an agent's own CLI.
              </p>
            </>
          )}
        </FormDialog>
      )}

      {keyPreview && (
        <ConfirmDialog
          title={`Rename the issue key to ${keyPreview.key}?`}
          body={
            <>
              <p>
                {keyPreview.plan.moves.length === 0
                  ? `This project has no issue files under ${keyPreview.current}, so only the key changes.`
                  : `${keyPreview.plan.moves.length} issue file${keyPreview.plan.moves.length === 1 ? "" : "s"} ` +
                    `move${keyPreview.plan.moves.length === 1 ? "s" : ""} from ${keyPreview.current} to ` +
                    `${keyPreview.key}, and the links between them are rewritten. Links to them from ` +
                    `other projects and from notes keep the old key.`}
              </p>
              {keyPreview.plan.renumbered.length > 0 && (
                <p>
                  {keyPreview.plan.renumbered.length === 1 ? "One number is" : `${keyPreview.plan.renumbered.length} numbers are`}{" "}
                  already taken under {keyPreview.key}, so{" "}
                  {keyPreview.plan.renumbered.map((m) => `${m.from} becomes ${m.to}`).join(", ")}.
                  Links to {keyPreview.plan.renumbered.length === 1 ? "it" : "them"} follow.
                </p>
              )}
              {keyPreview.sharedWith.length > 0 && (
                <p>
                  {keyPreview.sharedWith.join(", ")} already use{keyPreview.sharedWith.length === 1 ? "s" : ""}{" "}
                  {keyPreview.key}. That is what you want for two checkouts of one repo sharing a
                  backlog; otherwise a [[{keyPreview.key}-14]] link can only reach one of them.
                </p>
              )}
            </>
          }
          confirmLabel="Rename"
          busy={keyBusy}
          onConfirm={() => void doKeyRename(keyPreview)}
          onCancel={() => setKeyPreview(null)}
        />
      )}

      {wsSwitch && (
        <ConfirmDialog
          title="Switch workspace?"
          body={wsMissing
            ? `Your journal, notes, and workspace issues will now live in "${wsSwitch}". If that folder is this workspace in its new place, the agents' worktrees in it are reconnected too. A fresh folder starts with the Welcome guide.`
            : `Your journal, notes, and workspace issues will now live in "${wsSwitch}". The current folder stays on disk untouched; choose it again later to switch back. A fresh folder starts with the Welcome guide.`}
          confirmLabel="Switch"
          onConfirm={() => { const d = wsSwitch; setWsSwitch(null); void doSwitchWorkspace(d); }}
          onCancel={() => setWsSwitch(null)}
        />
      )}
    </main>
  );
}
