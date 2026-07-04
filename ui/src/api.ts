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
}

export type SessionStatus =
  | { state: "running" }
  | { state: "exited"; code: number }
  | { state: "gone" };

export interface RunInfo {
  id: string;
  projectId: string;
  agent: string;
  prompt: string;
  title: string | null;
  branch: string;
  status: SessionStatus;
  added: number;
  deleted: number;
  files: number;
  port: number | null;
  kind: "agent" | "terminal";
  archivedAt: number | null;
  raceId: string | null;
}

export const listProjects = () => invoke<Project[]>("list_projects");

export const addProject = (name: string, repoPath: string) =>
  invoke<Project>("add_project", { name, repoPath });

export type RepoReadiness = {
  state: "notARepo" | "noCommits" | "ready";
  stageable: boolean;
  dirty: boolean;
};

export const inspectRepo = (repoPath: string) =>
  invoke<RepoReadiness>("inspect_repo", { repoPath });
export const initRepo = (repoPath: string) =>
  invoke<void>("init_repo", { repoPath });
export const commitRepo = (repoPath: string, addGitignore: boolean) =>
  invoke<void>("commit_repo", { repoPath, addGitignore });

export const closeProject = (id: string) => invoke<void>("close_project", { id });
export const deleteProject = (id: string) => invoke<void>("delete_project", { id });

export const createRun = (projectId: string, prompt: string, agent: string, base: string, mergeTarget?: string | null) =>
  invoke<RunInfo>("create_run", { projectId, prompt, agent, base, mergeTarget: mergeTarget ?? null });
export const createTerminal = (projectId: string) =>
  invoke<RunInfo>("create_terminal", { projectId });
export const agentInstalled = (agent: string) =>
  invoke<boolean>("agent_installed", { agent });
export const createInstallTerminal = (projectId: string, agent: string, command: string) =>
  invoke<RunInfo>("create_install_terminal", { projectId, agent, command });
export const confirmQuit = () => invoke<void>("confirm_quit");
export const setRunTitle = (id: string, firstPrompt: string) =>
  invoke<void>("set_run_title", { id, firstPrompt });
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

export function attachRun(id: string, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_run", { id, onChunk });
}

export const runScriptConfigured = (id: string) =>
  invoke<boolean>("run_script_configured", { id });
export const startRunScript = (id: string) => invoke<void>("start_run_script", { id });
export const stopRunScript = (id: string) => invoke<void>("stop_run_script", { id });
export const runScriptStatus = (id: string) =>
  invoke<SessionStatus>("run_script_status", { id });
export const runScriptPreview = (id: string, lines: number) =>
  invoke<string>("run_script_preview", { id, lines });
export const detachRunScript = (id: string) => invoke<void>("detach_run_script", { id });
export const runScriptInput = (id: string, data: string) =>
  invoke<void>("run_script_input", { id, data });
export const resizeRunScript = (id: string, cols: number, rows: number) =>
  invoke<void>("resize_run_script", { id, cols, rows });

export function attachRunScript(id: string, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_run_script", { id, onChunk });
}

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

export const gitPush = (taskId: string) => invoke<void>("git_push", { taskId });

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
}

export interface ProviderSettings {
  anthropicApiKey: string;
  lmStudioBaseUrl: string;
}

// One MCP server definition: either stdio (command/args/env) or remote (url).
export interface McpServer {
  name: string;
  command: string | null;
  args: string[];
  env: Record<string, string>;
  url: string | null;
}

export const listMcpServers = () => invoke<McpServer[]>("list_mcp_servers");
export const saveMcpServers = (servers: McpServer[]) =>
  invoke<void>("save_mcp_servers", { servers });

export const listProfiles = () => invoke<AgentProfile[]>("list_profiles");
export const saveProfile = (profile: AgentProfile) =>
  invoke<void>("save_profile", { profile });
export const deleteProfile = (name: string) => invoke<void>("delete_profile", { name });
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

export const ghReadiness = (projectId: string) =>
  invoke<GhReadiness>("gh_readiness", { projectId });
export const createPr = (taskId: string) => invoke<PrInfo>("create_pr", { taskId });
export const prStatus = (taskId: string) => invoke<PrStatus>("pr_status", { taskId });
export const sendCheckFeedback = (taskId: string) =>
  invoke<void>("send_check_feedback", { taskId });

export const mergePreview = (taskId: string) =>
  invoke<MergePreview>("merge_preview", { taskId });
export const mergeTask = (taskId: string) => invoke<MergeOutcome>("merge_task", { taskId });
export const abortMergeTask = (taskId: string) =>
  invoke<void>("abort_merge_task", { taskId });
export const resolverInput = (taskId: string, data: string) =>
  invoke<void>("resolver_input", { taskId, data });
export interface StatusDto {
  state: "running" | "idle" | "exited" | "crashed";
  code: number | null;
}

export const resolverStatus = (taskId: string) =>
  invoke<StatusDto>("resolver_status", { taskId });
export const resolverResize = (taskId: string, cols: number, rows: number) =>
  invoke<void>("resolver_resize", { taskId, cols, rows });

export function resolveMerge(
  taskId: string,
  resolverProfile: string,
  onBytes: (bytes: Uint8Array) => void,
): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (msg) => onBytes(b64ToBytes(msg.b64));
  return invoke<void>("resolve_merge", { taskId, resolverProfile, onChunk });
}

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
  onlyWhenUnfocused: boolean;
  idleSecs: number;
}

export const setUiState = (focused: boolean, activeRun: string | null) =>
  invoke<void>("set_ui_state", { focused, activeRun });
export const getNotifSettings = () => invoke<NotifSettings>("get_notif_settings");
export const saveNotifSettings = (settings: NotifSettings) =>
  invoke<void>("save_notif_settings", { settings });

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

export interface DirEntry {
  name: string;
  isDir: boolean;
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
