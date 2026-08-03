import { invoke, Channel } from "@tauri-apps/api/core";
import { b64ToBytes } from "./b64";

// NOTE: Project comes from agency_core (snake_case serde field names).
export interface Project {
  id: string;
  name: string;
  repo_path: string;
  default_agent: string | null;
  default_provider: string | null;
  color: string | null;
  // 3-letter issue key ("AGE") the project's issues are numbered under.
  issue_key: string | null;
  // null = normal repo project; "workspace" = the pinned notes/journal home.
  kind: string | null;
}

export type SessionStatus =
  | { state: "running" }
  | { state: "exited"; code: number }
  | { state: "gone" };

// Loop recipe/progress (camelCase serde, see agency_core::loops). A run with
// loopConfig set is a loop: its agent is re-invoked headless until the check
// command exits 0 or the attempt cap is spent.
export interface LoopConfig {
  checkCommand: string;
  maxAttempts: number;
  checkTimeoutSecs: number;
}

export type LoopStatus = "awaitingAgent" | "checking" | "complete" | "stalled" | "stopped";

export interface LoopState {
  status: LoopStatus;
  attempt: number;
  consecutiveFailures: number;
  lastCheckExit: number | null;
  updatedAt: number;
}

// Live activity signal derived from pane output (camelCase serde, see
// agency_app::activity). "working" = output recently; "waiting" = quiet after
// a user-driven turn (finished, or blocked on input), decaying to idle after
// ~30m; "idle" = quiet with no turn in flight (never prompted, or waited too
// long). `since` is epoch ms when the current state began. Null until the
// backend's first 2s poll observes the run.
export interface RunActivity {
  state: "working" | "waiting" | "idle";
  since: number;
}

export interface RunInfo {
  id: string;
  projectId: string;
  agent: string;
  prompt: string;
  title: string | null;
  branch: string;
  status: SessionStatus;
  activity: RunActivity | null;
  added: number;
  deleted: number;
  files: number;
  port: number | null;
  kind: "agent" | "terminal";
  // False = the run works in the project's main checkout on its current branch
  // instead of an isolated worktree, so it has no branch of its own to merge or
  // open a PR from, and discarding it leaves the checkout untouched.
  worktree: boolean;
  // Epoch seconds; archivedAt is null for live runs.
  createdAt: number;
  archivedAt: number | null;
  raceId: string | null;
  loopConfig: LoopConfig | null;
  loopState: LoopState | null;
  // Local issue this run was dispatched from (see startIssueRun).
  issueId: string | null;
}

// ── issues (the local per-project tracker) ──────────────────────────────────

export type IssueStatus = "backlog" | "todo" | "in_progress" | "in_review" | "done" | "cancelled";

export interface Issue {
  id: string;
  projectId: string;
  // Per-project number, displayed with the project's issue key: AGE-14.
  seq: number;
  title: string;
  body: string;
  status: IssueStatus;
  // 0 none · 1 low · 2 medium · 3 high · 4 urgent.
  priority: number;
  // Civil dates, "YYYY-MM-DD" — lexicographic order is date order.
  due: string | null;
  scheduled: string | null;
  // Manual board order within a status group, ascending.
  rank: number | null;
  createdAt: number;
  updatedAt: number;
}

// Partial update: omitted fields keep their values. For the nullable fields
// an explicit null clears the value (absent still means untouched).
export interface IssuePatch {
  title?: string;
  body?: string;
  status?: IssueStatus;
  priority?: number;
  due?: string | null;
  scheduled?: string | null;
  rank?: number | null;
}

export const listProjects = () => invoke<Project[]>("list_projects");

// `color` must be a palette accent name (see PROJECT_COLOR_NAMES); the backend
// rejects anything else.
export const setProjectColor = (id: string, color: string) =>
  invoke<void>("set_project_color", { id, color });

export const listIssues = (projectId: string) => invoke<Issue[]>("list_issues", { projectId });
export const createIssue = (projectId: string, title: string, body: string, status: IssueStatus) =>
  invoke<Issue>("create_issue", { projectId, title, body, status });
export const updateIssue = (id: string, patch: IssuePatch) =>
  invoke<Issue>("update_issue", { id, patch });
export const deleteIssue = (id: string) => invoke<void>("delete_issue", { id });
export const startIssueRun = (issueId: string, agent: string, base?: string | null, mergeTarget?: string | null) =>
  invoke<RunInfo>("start_issue_run", { issueId, agent, base: base ?? null, mergeTarget: mergeTarget ?? null });
export const startIssueRace = (issueId: string, agents: string[], base?: string | null, mergeTarget?: string | null) =>
  invoke<RunInfo[]>("start_issue_race", { issueId, agents, base: base ?? null, mergeTarget: mergeTarget ?? null });
export const startIssueLoop = (
  issueId: string,
  agent: string,
  checkCommand: string,
  maxAttempts: number,
  base?: string | null,
  mergeTarget?: string | null,
) =>
  invoke<RunInfo>("start_issue_loop", {
    issueId, agent, checkCommand, maxAttempts, base: base ?? null, mergeTarget: mergeTarget ?? null,
  });

export const addProject = (name: string, repoPath: string) =>
  invoke<Project>("add_project", { name, repoPath });

// ── workspace (the pinned notes/journal project) ────────────────────────────

export const getWorkspace = () => invoke<Project | null>("get_workspace");
export const defaultWorkspaceLocation = () => invoke<string>("default_workspace_location");
// Creates (or adopts) the workspace folder. With `useGit` it is initialized and
// given an initial commit so agents can run in it; without, it's just a folder.
export const createWorkspace = (path: string, useGit: boolean) =>
  invoke<Project>("create_workspace", { path, useGit });
// Moves the workspace folder on disk and repoints the project at it.
export const moveWorkspace = (newPath: string) =>
  invoke<Project>("move_workspace", { newPath });
// Re-seed the workspace's Welcome guide if deleted; returns its note path.
export const ensureWorkspaceGuide = (projectId: string) =>
  invoke<string>("ensure_workspace_guide", { projectId });

export type RepoReadiness = {
  state: "notARepo" | "noCommits" | "ready";
  stageable: boolean;
  dirty: boolean;
};

export const inspectRepo = (repoPath: string) =>
  invoke<RepoReadiness>("inspect_repo", { repoPath });
export const initRepo = (repoPath: string) =>
  invoke<void>("init_repo", { repoPath });
// A progress update streamed from `git clone --progress` during a clone.
export type CloneProgress = { phase: string; percent: number | null; detail: string };

// Clones `url` into a new folder under `parentDir`; resolves to the clone's path.
// `onProgress`, if given, is called as git reports download progress.
export function cloneRepo(
  url: string,
  parentDir: string,
  onProgress?: (p: CloneProgress) => void,
): Promise<string> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<string>("clone_repo", { url, parentDir, onProgress: onProgressChannel });
}
// Stages everything in `repoPath` and makes the initial commit. `onProgress`, if
// given, is called as files are staged — a folder with a big tree takes a while.
export function commitRepo(
  repoPath: string,
  addGitignore: boolean,
  onProgress?: (p: CloneProgress) => void,
): Promise<void> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("commit_repo", { repoPath, addGitignore, onProgress: onProgressChannel });
}

export const closeProject = (id: string) => invoke<void>("close_project", { id });
export const deleteProject = (id: string) => invoke<void>("delete_project", { id });

// Creates an agent workspace (git worktree + first session). `onProgress`, if
// given, is called as the worktree is checked out and essentials copied — a
// large repo takes a while, so the UI shows movement instead of freezing.
// `worktree: false` starts the agent in the project's own checkout, on the
// branch already there; `base`/`mergeTarget` are then ignored.
export function createRun(
  projectId: string,
  prompt: string,
  agent: string,
  base: string,
  mergeTarget?: string | null,
  onProgress?: (p: CloneProgress) => void,
  worktree = true,
): Promise<RunInfo> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<RunInfo>("create_run", {
    projectId, prompt, agent, base, mergeTarget: mergeTarget ?? null, worktree,
    onProgress: onProgressChannel,
  });
}
export const createLoop = (
  projectId: string,
  prompt: string,
  agent: string,
  base: string,
  mergeTarget: string | null,
  checkCommand: string,
  maxAttempts: number,
) =>
  invoke<RunInfo>("create_loop", {
    projectId, prompt, agent, base, mergeTarget, checkCommand, maxAttempts,
  });
export const stopLoop = (id: string) => invoke<void>("stop_loop", { id });
export const createTerminal = (projectId: string) =>
  invoke<RunInfo>("create_terminal", { projectId });
export const agentInstalled = (agent: string) =>
  invoke<boolean>("agent_installed", { agent });
export const createInstallTerminal = (projectId: string, agent: string, command: string) =>
  invoke<RunInfo>("create_install_terminal", { projectId, agent, command });
export const confirmQuit = () => invoke<void>("confirm_quit");
export const setRunTitle = (id: string, firstPrompt: string) =>
  invoke<void>("set_run_title", { id, firstPrompt });
// Rename a run: overwrites the display title (empty clears it → falls back to prompt/branch).
export const renameRun = (id: string, title: string) =>
  invoke<void>("rename_run", { id, title });
export const listRuns = (projectId: string) => invoke<RunInfo[]>("list_runs", { projectId });
export const runPreview = (id: string, lines: number) =>
  invoke<string>("run_preview", { id, lines });
export const detachRun = (id: string) => invoke<void>("detach_run", { id });
export const runInput = (id: string, data: string) => invoke<void>("run_input", { id, data });
export const resizeRun = (id: string, cols: number, rows: number) =>
  invoke<void>("resize_run", { id, cols, rows });
export const runStatus = (id: string) => invoke<SessionStatus>("run_status", { id });
export const discardRun = (id: string) => invoke<void>("discard_run", { id });
export const stopRun = (id: string) => invoke<void>("stop_run", { id });
export const rerun = (id: string) => invoke<RunInfo>("rerun", { id });
export const ensureRunActive = (id: string) => invoke<void>("ensure_run_active", { id });
export const archiveRun = (id: string) => invoke<void>("archive_run", { id });
export const restoreRun = (id: string) => invoke<RunInfo>("restore_run", { id });
export const listArchivedRuns = (projectId: string) =>
  invoke<RunInfo[]>("list_archived_runs", { projectId });
/** Result of a bulk discard: some runs can fail while the rest still go. */
export type DiscardSummary = { discarded: number; failed: string[] };
export const discardArchivedRuns = (projectId: string) =>
  invoke<DiscardSummary>("discard_archived_runs", { projectId });

export function attachRun(id: string, cols: number, rows: number, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_run", { id, cols, rows, onChunk });
}

// One candidate run command detected in the project (package.json script,
// Makefile target, …), offered when adding a script.
export type RunSuggestion = {
  label: string;
  name: string;
  command: string;
  detail: string;
  web: boolean;
};

// One entry in the project's run list.
export type RunScript = {
  name: string;
  command: string;
  // Serves a web app on $AGENCY_PORT, so it gets a URL, the preview pane and
  // "Open in browser". A build or a test run doesn't.
  web: boolean;
  // Starting this one stops every other run script in the project.
  nonconcurrent: boolean;
};

// The project's run scripts as the Run tab sees them. Empty `scripts` is what
// the setup card renders for.
export type RunScriptConfig = {
  scripts: RunScript[];
  shared: boolean;
  suggestions: RunSuggestion[];
  workspace: string;
  port: number | null;
};

export type RunScriptStatus = { name: string; status: SessionStatus };

// Which workspace a run script runs in: an agent's (its run id) or the
// project's own checkout. The same token shape Source Control takes.
export const projectTarget = (projectId: string) => `project:${projectId}`;

export const runScriptConfig = (target: string) =>
  invoke<RunScriptConfig>("run_script_config", { target });
export const saveRunScripts = (target: string, scripts: RunScript[]) =>
  invoke<void>("save_run_scripts", { target, scripts });
export const startRunScript = (target: string, script: string) =>
  invoke<void>("start_run_script", { target, script });
export const stopRunScript = (target: string, script: string) =>
  invoke<void>("stop_run_script", { target, script });
export const runScriptsStatus = (target: string) =>
  invoke<RunScriptStatus[]>("run_scripts_status", { target });

// A terminal pane carries a single id, but a run script is addressed by
// workspace *and* script name. These pack the pair into one key for the
// FocusTerminal stream and unpack it again at the IPC boundary. Neither a run
// id nor `project:<id>` can contain "#", so the first one always splits them.
export const runScriptKey = (target: string, script: string) => `${target}#${script}`;
function splitRunScriptKey(key: string): { target: string; script: string } {
  const at = key.indexOf("#");
  return at < 0
    ? { target: key, script: "" }
    : { target: key.slice(0, at), script: key.slice(at + 1) };
}

export const runScriptPreview = (key: string, lines: number) =>
  invoke<string>("run_script_preview", { ...splitRunScriptKey(key), lines });
export const detachRunScript = (key: string) =>
  invoke<void>("detach_run_script", splitRunScriptKey(key));
export const runScriptInput = (key: string, data: string) =>
  invoke<void>("run_script_input", { ...splitRunScriptKey(key), data });
export const resizeRunScript = (key: string, cols: number, rows: number) =>
  invoke<void>("resize_run_script", { ...splitRunScriptKey(key), cols, rows });

export function attachRunScript(key: string, cols: number, rows: number, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_run_script", { ...splitRunScriptKey(key), cols, rows, onChunk });
}

// Companion shell: a per-run interactive terminal sharing the run's worktree,
// independent of the agent and run-script sessions.
export const startShell = (id: string) => invoke<void>("start_shell", { id });
export const stopShell = (id: string) => invoke<void>("stop_shell", { id });
export const shellStatus = (id: string) => invoke<SessionStatus>("shell_status", { id });
export const shellPreview = (id: string, lines: number) =>
  invoke<string>("shell_preview", { id, lines });
export const detachShell = (id: string) => invoke<void>("detach_shell", { id });
export const shellInput = (id: string, data: string) =>
  invoke<void>("shell_input", { id, data });
export const resizeShell = (id: string, cols: number, rows: number) =>
  invoke<void>("resize_shell", { id, cols, rows });

export function attachShell(id: string, cols: number, rows: number, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_shell", { id, cols, rows, onChunk });
}

// Extra agent sessions: additional agent tabs sharing a run's worktree. Ids
// are composite (`<runId>--<n>`) and are accepted by the ordinary run-terminal
// calls (attachRun / runInput / resizeRun / runPreview / ensureRunActive).
export interface RunSessionInfo {
  id: string;
  runId: string;
  agent: string;
  status: SessionStatus;
}

export const startRunSession = (runId: string, agent?: string) =>
  invoke<RunSessionInfo>("start_run_session", { runId, agent: agent ?? null });
export const listRunSessions = (runId: string) =>
  invoke<RunSessionInfo[]>("list_run_sessions", { runId });
export const closeRunSession = (id: string) => invoke<void>("close_run_session", { id });

export interface FileChange {
  path: string;
  index: string; // staged status code
  worktree: string; // unstaged status code
}

export const gitStatus = (taskId: string) =>
  invoke<FileChange[]>("git_status", { taskId });

export const gitDiff = (taskId: string, path: string, staged: boolean) =>
  invoke<string>("git_diff", { taskId, path, staged });

export const gitStage = (taskId: string, path: string) =>
  invoke<void>("git_stage", { taskId, path });

export const gitUnstage = (taskId: string, path: string) =>
  invoke<void>("git_unstage", { taskId, path });
export const gitStageAll = (taskId: string) => invoke<void>("git_stage_all", { taskId });
export const gitUnstageAll = (taskId: string) => invoke<void>("git_unstage_all", { taskId });
export const gitDiscard = (taskId: string, path: string, untracked: boolean) =>
  invoke<void>("git_discard", { taskId, path, untracked });
export const gitDiscardAll = (taskId: string) => invoke<void>("git_discard_all", { taskId });

export const gitCommit = (taskId: string, message: string) =>
  invoke<void>("git_commit", { taskId, message });

// Pushes the current branch to origin, streaming git's progress to `onProgress`
// (same shape as clone) so the UI can show a determinate bar for large pushes.
export function gitPush(
  taskId: string,
  onProgress?: (p: CloneProgress) => void,
): Promise<void> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("git_push", { taskId, onProgress: onProgressChannel });
}
export const gitFetch = (taskId: string) => invoke<void>("git_fetch", { taskId });
export const gitPull = (taskId: string) => invoke<void>("git_pull", { taskId });

// VS Code-style "Sync Changes": fetch, fast-forward in incoming commits, then
// push local ones (streaming push progress, same shape as `gitPush`). Resolves
// to "diverged" when the branch can't fast-forward — nothing was changed and
// the caller should offer to rebase.
export type SyncOutcome = "synced" | "diverged";
export function gitSync(
  taskId: string,
  onProgress?: (p: CloneProgress) => void,
): Promise<SyncOutcome> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<SyncOutcome>("git_sync", { taskId, onProgress: onProgressChannel });
}

export const gitSetRemote = (taskId: string, url: string) =>
  invoke<void>("git_set_remote", { taskId, url });

export interface RunBranches {
  branch: string;
  base: string;
}
export const runBranches = (taskId: string) =>
  invoke<RunBranches>("run_branches", { taskId });

// NOTE: field names mirror agency_core's AgentProfile (snake_case serde).
export interface AgentProfile {
  name: string;
  command: string;
  args: string[];
  env: [string, string][];
  resume_args: string[] | null;
  loop_args: string[] | null;
}

export interface ProviderSettings {
  lmStudioBaseUrl: string;
  // Agent id "New Agent" spawns; null = auto (project's last-used agent).
  defaultAgent: string | null;
  // Whether the add-agent menu starts with "Own worktree" ticked. A default
  // only: the menu still offers both on every spawn.
  defaultWorktree: boolean;
}

export type McpTransport = "stdio" | "http" | "sse";

// One MCP server definition: either stdio (command/args/env) or remote (url).
// `transport` is inferred when null; `headers` carry auth for remote servers;
// `userScope` marks a server registered/authenticated with the agent CLI itself
// (Authenticate flow) so Agency stops emitting it into per-worktree config.
export interface McpServer {
  name: string;
  command: string | null;
  args: string[];
  env: Record<string, string>;
  url: string | null;
  transport: McpTransport | null;
  headers: Record<string, string>;
  userScope: boolean;
}

// Result of importing an mcp.json: the full list after merge, plus how many
// servers the file contributed (entries with neither command nor url are skipped).
export interface McpImportResult {
  servers: McpServer[];
  imported: number;
}

export const listMcpServers = () => invoke<McpServer[]>("list_mcp_servers");
export const saveMcpServers = (servers: McpServer[]) =>
  invoke<void>("save_mcp_servers", { servers });
export const importMcpJson = (text: string) =>
  invoke<McpImportResult>("import_mcp_json", { text });
export const authenticateMcpServer = (projectId: string, agent: string, name: string) =>
  invoke<RunInfo>("authenticate_mcp_server", { projectId, agent, name });
export const deauthenticateMcpServer = (name: string) =>
  invoke<void>("deauthenticate_mcp_server", { name });

// Per-project graphify knowledge-graph config (persisted to the project's
// gitignored .agency/agency.local.toml). *_command are null when unset, in
// which case *_default is what actually runs; *_installed reports PATH presence.
export interface KnowledgeConfig {
  graph: boolean;
  serve_command: string | null;
  build_command: string | null;
  serve_default: string;
  build_default: string;
  serve_installed: boolean;
  build_installed: boolean;
}

export const getKnowledgeConfig = (projectId: string) =>
  invoke<KnowledgeConfig>("get_knowledge_config", { projectId });
export const saveKnowledgeConfig = (
  projectId: string,
  graph: boolean,
  serveCommand: string | null,
  buildCommand: string | null,
) => invoke<void>("save_knowledge_config", { projectId, graph, serveCommand, buildCommand });

// Per-project list of files copied into every new agent worktree (persisted to
// the project's gitignored .agency/agency.local.toml). `detectedEnv` is the set
// of untracked root .env* files copied automatically on top of `copy`.
export interface FilesConfig {
  copy: string[];
  detectedEnv: string[];
}

export const getFilesConfig = (projectId: string) =>
  invoke<FilesConfig>("get_files_config", { projectId });
export const saveFilesConfig = (projectId: string, copy: string[]) =>
  invoke<void>("save_files_config", { projectId, copy });

export const listProfiles = () => invoke<AgentProfile[]>("list_profiles");
export const saveProfile = (profile: AgentProfile) =>
  invoke<void>("save_profile", { profile });
export const deleteProfile = (name: string) => invoke<void>("delete_profile", { name });

/** One built-in agent from the catalog (may or may not be enabled yet). */
export interface CatalogEntry {
  id: string;
  command: string;
  resumeArgs: string[] | null;
  loopArgs: string[] | null;
  enabled: boolean;
  installed: boolean;
  /** Whether Agency emits per-workspace MCP config for this agent. */
  supportsMcp: boolean;
}

export const agentOnboardingNeeded = () => invoke<boolean>("agent_onboarding_needed");
export const listAgentCatalog = () => invoke<CatalogEntry[]>("list_agent_catalog");
export const enableAgentProfiles = (ids: string[]) =>
  invoke<void>("enable_agent_profiles", { ids });
export const completeAgentOnboarding = (ids: string[]) =>
  invoke<void>("complete_agent_onboarding", { ids });

export const getSettings = () => invoke<ProviderSettings>("get_settings");
export const saveSettings = (settings: ProviderSettings) =>
  invoke<void>("save_settings", { settings });

export type MergeOutcome =
  | { kind: "clean"; commit: string }
  | { kind: "conflicts"; files: string[] };

export interface MergePreview {
  base: string;
  branch: string;
  commitsAhead: number;
  commitsBehind: number;
  worktreeDirty: boolean;
  dirtyFiles: string[];
}

// How far the user is from usable PR features; each non-ready state maps to a
// guided setup step (install gh → gh auth login → add a GitHub remote).
export type GhReadiness = "notInstalled" | "notAuthenticated" | "noGithubRemote" | "ready";

export interface PrInfo {
  number: number;
  url: string;
  title: string;
  state: "OPEN" | "CLOSED" | "MERGED";
  isDraft: boolean;
  baseRefName: string;
  headRefName: string;
}

export interface CheckItem {
  name: string;
  bucket: "pass" | "fail" | "pending" | "skipping" | "cancel" | "";
  link: string;
  description: string;
}

export interface PrStatus {
  pr: PrInfo | null;
  checks: CheckItem[];
}

export interface IssueItem {
  number: number;
  title: string;
}

// ── In-app PR review ────────────────────────────────────────────────────────

export interface Author {
  login: string;
}

export interface PrDetail {
  number: number;
  url: string;
  title: string;
  state: "OPEN" | "CLOSED" | "MERGED";
  isDraft: boolean;
  baseRefName: string;
  headRefName: string;
  body: string;
  headRefOid: string;
  mergeable: string; // MERGEABLE | CONFLICTING | UNKNOWN
  reviewDecision: string | null; // APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED | null
  author: Author;
  createdAt: string;
  updatedAt: string;
}

// One file of a PR diff: the local FileDiff shape plus path metadata.
export interface PrFileDiff extends FileDiff {
  path: string;
  oldPath: string | null;
  binary: boolean;
}

export interface PrReviewComment {
  id: string;
  databaseId: number;
  path: string;
  line: number | null;
  originalLine: number | null;
  diffHunk: string;
  body: string;
  author: string;
  createdAt: string;
  inReplyToId: string | null;
}

export interface ReviewThread {
  id: string;
  isResolved: boolean;
  isOutdated: boolean;
  path: string;
  line: number | null;
  diffSide: string; // RIGHT | LEFT
  comments: PrReviewComment[];
}

export type ReviewEvent = "APPROVE" | "REQUEST_CHANGES" | "COMMENT";

// A new inline comment to include when submitting a review. Anchoring (line +
// side, and start_* for multi-line) is computed on the frontend from the
// selected diff rows.
export interface DraftComment {
  path: string;
  body: string;
  line: number;
  side: "RIGHT" | "LEFT";
  startLine?: number | null;
  startSide?: "RIGHT" | "LEFT" | null;
}

export const createRace = (
  projectId: string,
  prompt: string,
  agents: string[],
  base: string,
  mergeTarget?: string | null,
) => invoke<RunInfo[]>("create_race", { projectId, prompt, agents, base, mergeTarget: mergeTarget ?? null });
export const listGhIssues = (projectId: string) =>
  invoke<IssueItem[]>("list_gh_issues", { projectId });
export const listGhPrs = (projectId: string) => invoke<PrInfo[]>("list_gh_prs", { projectId });
export const createRunFromIssue = (projectId: string, number: number, agent: string) =>
  invoke<RunInfo>("create_run_from_issue", { projectId, number, agent });
export const createRunFromPr = (projectId: string, number: number, agent: string) =>
  invoke<RunInfo>("create_run_from_pr", { projectId, number, agent });

// Where an agent PR review landed. `sessionId` is set when the review had to run
// as an extra tab inside an existing run (the PR's branch was already checked
// out there) — focus that tab rather than the run's primary agent.
export interface PrReviewRun {
  run: RunInfo;
  sessionId: string | null;
}
// Start an agent that reviews a PR and then stays around to fix what it found.
// `postComments` publishes the findings to the PR on GitHub.
export const createPrReviewRun = (
  projectId: string,
  number: number,
  agent: string,
  postComments: boolean,
) => invoke<PrReviewRun>("create_pr_review_run", { projectId, number, agent, postComments });

export const ghReadiness = (projectId: string) =>
  invoke<GhReadiness>("gh_readiness", { projectId });
// gh install + auth state without a repo — for the clone dialog's sign-in guidance.
export const ghAuthReadiness = () => invoke<GhReadiness>("gh_auth_readiness");
export const createPr = (taskId: string) => invoke<PrInfo>("create_pr", { taskId });
export const prStatus = (taskId: string) => invoke<PrStatus>("pr_status", { taskId });
export const sendCheckFeedback = (taskId: string) =>
  invoke<void>("send_check_feedback", { taskId });

// In-app PR review, keyed by (projectId, prNumber).
export const prDetail = (projectId: string, number: number) =>
  invoke<PrDetail | null>("pr_detail", { projectId, number });
export const ghCurrentLogin = (projectId: string) =>
  invoke<string>("gh_current_login", { projectId });
export interface MergeMethods {
  merge: boolean;
  squash: boolean;
  rebase: boolean;
}
export type MergeMethod = "merge" | "squash" | "rebase";
/** What happened to the head branch after a successful PR merge. */
export interface PrMergeResult {
  branchDeleted: string | null;
  // Set when the merge landed but branch cleanup didn't; a note, not a failure.
  warning: string | null;
}
export const prMergeMethods = (projectId: string) =>
  invoke<MergeMethods>("pr_merge_methods", { projectId });
export const mergePr = (projectId: string, number: number, method: MergeMethod, deleteBranch: boolean) =>
  invoke<PrMergeResult>("merge_pr", { projectId, number, method, deleteBranch });
export const prDiff = (projectId: string, number: number) =>
  invoke<PrFileDiff[]>("pr_diff", { projectId, number });
export const prReviewThreads = (projectId: string, number: number) =>
  invoke<ReviewThread[]>("pr_review_threads", { projectId, number });
export const submitPrReview = (
  projectId: string,
  number: number,
  event: ReviewEvent,
  body: string | null,
  comments: DraftComment[],
) => invoke<void>("submit_pr_review", { projectId, number, event, body, comments });
export const replyPrComment = (projectId: string, number: number, inReplyTo: number, body: string) =>
  invoke<void>("reply_pr_comment", { projectId, number, inReplyTo, body });
export const resolvePrThread = (projectId: string, threadId: string) =>
  invoke<void>("resolve_pr_thread", { projectId, threadId });
export const unresolvePrThread = (projectId: string, threadId: string) =>
  invoke<void>("unresolve_pr_thread", { projectId, threadId });
export const prNumberForRun = (taskId: string) =>
  invoke<number | null>("pr_number_for_run", { taskId });
export const createPrFromBranch = (
  projectId: string,
  head: string,
  base: string | null,
  title: string | null,
  body: string | null,
) => invoke<PrInfo>("create_pr_from_branch", { projectId, head, base, title, body });

export const mergePreview = (taskId: string) =>
  invoke<MergePreview>("merge_preview", { taskId });
export const mergeTask = (taskId: string) => invoke<MergeOutcome>("merge_task", { taskId });
/** Where a conflicted merge stands, asked of git rather than re-run. */
export interface MergeState {
  merging: boolean;
  unresolved: string[];
  merged: boolean;
}
export const mergeStatus = (taskId: string) =>
  invoke<MergeState>("merge_status", { taskId });
// Commit a resolved merge (or accept one the resolver committed itself) and
// put the checkout back on the branch it was on.
export const finishMergeTask = (taskId: string) =>
  invoke<MergeOutcome>("finish_merge_task", { taskId });
export const abortMergeTask = (taskId: string) =>
  invoke<void>("abort_merge_task", { taskId });
// "Fix with agent": types the conflict, with git's own status output, into this
// run's live agent session.
export const sendMergeConflict = (taskId: string) =>
  invoke<void>("send_merge_conflict", { taskId });

export interface Hunk {
  header: string;
  lines: string[];
}
export interface FileDiff {
  header: string;
  hunks: Hunk[];
}

export const gitParseDiff = (taskId: string, path: string, staged: boolean) =>
  invoke<FileDiff>("git_parse_diff", { taskId, path, staged });
export const gitStageHunk = (taskId: string, path: string, hunkIndex: number) =>
  invoke<void>("git_stage_hunk", { taskId, path, hunkIndex });
export const gitUnstageHunk = (taskId: string, path: string, hunkIndex: number) =>
  invoke<void>("git_unstage_hunk", { taskId, path, hunkIndex });
export interface HistoryItem {
  hash: string;
  parents: string[];
  author: string;
  email: string;
  date: number;
  subject: string;
  refs: string[];
}

export interface BranchInfo {
  branch: string;
  upstream: string | null;
  ahead: number;
  behind: number;
  base: string | null;
  hasRemote: boolean;
}

export const gitLogGraph = (taskId: string, limit: number) =>
  invoke<HistoryItem[]>("git_log_graph", { taskId, limit });
export const gitBranchInfo = (taskId: string) =>
  invoke<BranchInfo>("git_branch_info", { taskId });

export interface ProjectBranches {
  current: string;
  branches: string[];
}

export const listProjectBranches = (projectId: string) =>
  invoke<ProjectBranches>("list_project_branches", { projectId });

export interface CommitFile {
  path: string;
  status: string;
}

export const gitCommitFiles = (taskId: string, hash: string) =>
  invoke<CommitFile[]>("git_commit_files", { taskId, hash });
export const gitCommitDiff = (taskId: string, hash: string, path: string) =>
  invoke<string>("git_commit_diff", { taskId, hash, path });
export const gitCommitAmend = (taskId: string, message: string) =>
  invoke<void>("git_commit_amend", { taskId, message });

export const gitCheckoutBranch = (taskId: string, name: string) =>
  invoke<void>("git_checkout_branch", { taskId, name });
export const gitCreateBranch = (taskId: string, name: string, from: string | null, checkout: boolean) =>
  invoke<void>("git_create_branch", { taskId, name, from, checkout });
export const gitDeleteBranch = (taskId: string, name: string, force: boolean) =>
  invoke<void>("git_delete_branch", { taskId, name, force });
export const gitListBranches = (taskId: string) =>
  invoke<ProjectBranches>("git_list_branches", { taskId });
export const gitPullRebase = (taskId: string) => invoke<void>("git_pull_rebase", { taskId });
export const gitPushForce = (taskId: string) => invoke<void>("git_push_force", { taskId });
/** Resolves to the undone commit's message so it can be restored into the input. */
export const gitUndoLastCommit = (taskId: string) =>
  invoke<string>("git_undo_last_commit", { taskId });
export const gitResetTo = (taskId: string, hash: string, mode: "soft" | "mixed" | "hard") =>
  invoke<void>("git_reset_to", { taskId, hash, mode });
export const gitRevertCommit = (taskId: string, hash: string) =>
  invoke<void>("git_revert_commit", { taskId, hash });
export const gitCherryPick = (taskId: string, hash: string) =>
  invoke<void>("git_cherry_pick", { taskId, hash });

export interface StashEntry {
  index: number;
  message: string;
}

export const gitStashList = (taskId: string) =>
  invoke<StashEntry[]>("git_stash_list", { taskId });
export const gitStashPush = (taskId: string, message: string | null, includeUntracked: boolean) =>
  invoke<void>("git_stash_push", { taskId, message, includeUntracked });
export const gitStashApply = (taskId: string, index: number) =>
  invoke<void>("git_stash_apply", { taskId, index });
export const gitStashPop = (taskId: string, index: number) =>
  invoke<void>("git_stash_pop", { taskId, index });
export const gitStashDrop = (taskId: string, index: number) =>
  invoke<void>("git_stash_drop", { taskId, index });

export const gitStageLines = (taskId: string, path: string, hunkIndex: number, lines: number[]) =>
  invoke<void>("git_stage_lines", { taskId, path, hunkIndex, lines });
export const gitUnstageLines = (taskId: string, path: string, hunkIndex: number, lines: number[]) =>
  invoke<void>("git_unstage_lines", { taskId, path, hunkIndex, lines });
export const gitRevertLines = (taskId: string, path: string, hunkIndex: number, lines: number[]) =>
  invoke<void>("git_revert_lines", { taskId, path, hunkIndex, lines });

export interface NotifSettings {
  agentFinished: boolean;
  agentIdle: boolean;
  runCrashed: boolean;
  mergeAttention: boolean;
  loopEvents: boolean;
  /** Skip notifications for the run that's open in a focused window; every
   *  other run still notifies while you work in the app. */
  onlyWhenWatching: boolean;
  idleSecs: number;
}

export const setUiState = (focused: boolean, activeRun: string | null) =>
  invoke<void>("set_ui_state", { focused, activeRun });
// Toggle the native menu's context-dependent items: project-gated (New
// Agent/Terminal, Source) and agent-gated (the Agent menu).
export const setMenuContext = (project: boolean, focusedAgent: boolean) =>
  invoke<void>("set_menu_context", { project, focusedAgent });
export const getNotifSettings = () => invoke<NotifSettings>("get_notif_settings");
export const saveNotifSettings = (settings: NotifSettings) =>
  invoke<void>("save_notif_settings", { settings });

export interface UpdateCheck {
  current: string;
  /** Latest published release, or null when the check couldn't complete. */
  latest: string | null;
  updateAvailable: boolean;
  /** Releases page to send the user to. */
  url: string;
  /** Why the check came back empty; null on success. */
  error: string | null;
}

/** Asks GitHub for the latest release. Resolves (never rejects) when offline —
 *  inspect `error`. Agency downloads nothing; the user installs the DMG. */
export const checkForUpdate = () => invoke<UpdateCheck>("check_for_update");
export const getUpdateCheckEnabled = () => invoke<boolean>("get_update_check_enabled");
export const setUpdateCheckEnabled = (enabled: boolean) =>
  invoke<void>("set_update_check_enabled", { enabled });

export interface ReviewComment {
  id: string;
  runId: string;
  path: string;
  lineStart: number;
  lineEnd: number;
  body: string;
  sent: boolean;
  createdAt: number;
}

export const addReviewComment = (
  runId: string, path: string, lineStart: number, lineEnd: number, body: string,
) => invoke<ReviewComment>("add_review_comment", { runId, path, lineStart, lineEnd, body });
export const listReviewComments = (runId: string) =>
  invoke<ReviewComment[]>("list_review_comments", { runId });
export const deleteReviewComment = (id: string) =>
  invoke<void>("delete_review_comment", { id });
export const sendReviewComments = (runId: string) =>
  invoke<void>("send_review_comments", { runId });

export type FileRoot = { kind: "run"; id: string } | { kind: "project"; id: string };

/**
 * The file root behind a git panel's task token, mirroring the backend's
 * `git_root`: a "project:<id>" token means the project's main checkout, any
 * other token is a run's worktree.
 */
export const fileRootOf = (taskId: string): FileRoot =>
  taskId.startsWith("project:")
    ? { kind: "project", id: taskId.slice("project:".length) }
    : { kind: "run", id: taskId };

export interface DirEntry {
  name: string;
  isDir: boolean;
  // For directories: whether they hold at least one entry (drives the expand arrow).
  hasChildren: boolean;
}

export interface FileContents {
  text: string;
  binary: boolean;
  tooLarge: boolean;
}

export const listDir = (root: FileRoot, relPath: string) =>
  invoke<DirEntry[]>("list_dir", { root, relPath });

export const readFile = (root: FileRoot, relPath: string) =>
  invoke<FileContents>("read_file", { root, relPath });

export interface BinaryContents {
  b64: string;
  mime: string;
  tooLarge: boolean;
}

export const readFileBase64 = (root: FileRoot, relPath: string) =>
  invoke<BinaryContents>("read_file_base64", { root, relPath });

export const writeFile = (root: FileRoot, relPath: string, contents: string) =>
  invoke<void>("write_file", { root, relPath, contents });

export const createFile = (root: FileRoot, relPath: string) =>
  invoke<void>("create_file", { root, relPath });
export const createDir = (root: FileRoot, relPath: string) =>
  invoke<void>("create_dir", { root, relPath });
export const renamePath = (root: FileRoot, from: string, to: string) =>
  invoke<void>("rename_path", { root, from, to });
export const trashPath = (root: FileRoot, relPath: string) =>
  invoke<void>("trash_path", { root, relPath });
// Appends the path to the root's .gitignore. Resolves to true if a new entry was
// written, false if the path was already ignored.
export const addToGitignore = (root: FileRoot, relPath: string) =>
  invoke<boolean>("add_to_gitignore", { root, relPath });
export const absPath = (root: FileRoot, relPath: string) =>
  invoke<string>("abs_path", { root, relPath });
export const revealPath = (root: FileRoot, relPath: string) =>
  invoke<void>("reveal_path", { root, relPath });

// Actual on-disk name of the project's top-level docs folder (case-insensitive
// match), or null when the project has none.
export const detectDocsDir = (projectId: string) =>
  invoke<string | null>("detect_docs_dir", { projectId });

export interface DocFile {
  // Path relative to the docs dir, "/"-separated.
  path: string;
  text: string;
  tooLarge: boolean;
}

// Every markdown file under the docs dir in one call — feeds the docs index.
export const readDocsCorpus = (root: FileRoot, docsDir: string) =>
  invoke<DocFile[]>("read_docs_corpus", { root, docsDir });

// Stat-only corpus pass: change signatures without body reads. Steady-state
// docs polls diff these and re-read only what changed.
export interface DocStat {
  path: string;
  mtimeMs: number;
  size: number;
}
export const docsCorpusStats = (root: FileRoot, docsDir: string) =>
  invoke<DocStat[]>("docs_corpus_stats", { root, docsDir });
// Read a named subset of the corpus (the poll's "these changed" list).
export const readDocsFiles = (root: FileRoot, docsDir: string, paths: string[]) =>
  invoke<DocFile[]>("read_docs_files", { root, docsDir, paths });

// ── checkbox tasks (one-stop Phase 8) ───────────────────────────────────────

export interface TaskHit {
  path: string; // relative to the docs dir, "/"-separated
  line: number; // 0-based
  checked: boolean;
  text: string; // marker stripped, trimmed
}

// Every checkbox task in a root's docs corpus (same file set as the index).
export const scanTasks = (root: FileRoot, docsDir: string) =>
  invoke<TaskHit[]>("scan_tasks", { root, docsDir });

// Flip one task's checkbox in place. False = the line no longer holds the
// expected task (external edit); the caller refreshes instead of writing.
export const toggleTask = (root: FileRoot, docsDir: string, relPath: string, line: number, checked: boolean) =>
  invoke<boolean>("toggle_task", { root, docsDir, relPath, line, checked });

// ── content search (one-stop Phase 2 primitive) ─────────────────────────────

export interface SearchQuery {
  query: string;
  // Treat query as a regex; default literal.
  regex?: boolean;
  // Case-sensitive when true; default insensitive.
  case?: boolean;
  // Gitignore-style globs ("*.md" matches at any depth); empty = all files.
  globs?: string[];
  maxHits?: number;
}

export interface BackendSearchHit {
  path: string; // relative to the searched dir
  line: number; // 1-based
  col: number; // 1-based
  text: string; // the matching line, length-capped
}

// Content search under `dir` within a root. Bounded (hits/bytes/time) and
// best-effort: hitting a cap returns what was collected.
export const searchFiles = (root: FileRoot, dir: string, query: SearchQuery) =>
  invoke<BackendSearchHit[]>("search_files", { root, dir, query });

// Sorted relative file paths under `dir` — quick-open's name list. Same file
// set as the search fallback (gitignore respected in repos), capped backend-side.
export const listFiles = (root: FileRoot, dir: string, maxFiles: number) =>
  invoke<string[]>("list_files", { root, dir, maxFiles });

// Write base64 bytes to a NEW file (fails on an existing path). For image paste.
export const writeFileBase64 = (root: FileRoot, relPath: string, b64: string) =>
  invoke<void>("write_file_base64", { root, relPath, b64 });

// Copy a file from anywhere on disk into a NEW path inside the root (fails on
// an existing path). For attaching files dropped from Finder or picked in the
// dialog, where the UI has a path but not the bytes.
export const importFile = (root: FileRoot, srcPath: string, relPath: string) =>
  invoke<void>("import_file", { root, srcPath, relPath });
