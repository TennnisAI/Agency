import { invoke, Channel } from "@tauri-apps/api/core";
import { b64ToBytes } from "./b64";

// NOTE: Project comes from agency_core (snake_case serde field names).
export interface Project {
  id: string;
  name: string;
  repo_path: string;
  default_agent: string | null;
  default_provider: string | null;
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
  branch: string;
  status: SessionStatus;
  added: number;
  deleted: number;
  files: number;
}

export const listProjects = () => invoke<Project[]>("list_projects");

export const addProject = (name: string, repoPath: string) =>
  invoke<Project>("add_project", { name, repoPath });

export const removeProject = (id: string) =>
  invoke<void>("remove_project", { id });

export const createRun = (projectId: string, prompt: string, agent: string, base: string) =>
  invoke<RunInfo>("create_run", { projectId, prompt, agent, base });
export const listRuns = (projectId: string) => invoke<RunInfo[]>("list_runs", { projectId });
export const runPreview = (id: string, lines: number) =>
  invoke<string>("run_preview", { id, lines });
export const detachRun = (id: string) => invoke<void>("detach_run", { id });
export const runInput = (id: string, data: string) => invoke<void>("run_input", { id, data });
export const runStatus = (id: string) => invoke<SessionStatus>("run_status", { id });
export const discardRun = (id: string) => invoke<void>("discard_run", { id });
export const rerun = (id: string) => invoke<RunInfo>("rerun", { id });

export function attachRun(id: string, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_run", { id, onChunk });
}

export interface FileChange {
  path: string;
  index: string; // staged status code
  worktree: string; // unstaged status code
}

export interface CommitInfo {
  hash: string;
  summary: string;
}

export const gitStatus = (taskId: string) =>
  invoke<FileChange[]>("git_status", { taskId });

export const gitDiff = (taskId: string, path: string, staged: boolean) =>
  invoke<string>("git_diff", { taskId, path, staged });

export const gitLog = (taskId: string) => invoke<CommitInfo[]>("git_log", { taskId });

export const gitStage = (taskId: string, path: string) =>
  invoke<void>("git_stage", { taskId, path });

export const gitUnstage = (taskId: string, path: string) =>
  invoke<void>("git_unstage", { taskId, path });

export const gitCommit = (taskId: string, message: string) =>
  invoke<void>("git_commit", { taskId, message });

export const gitPush = (taskId: string) => invoke<void>("git_push", { taskId });

export interface AgentProfile {
  name: string;
  command: string;
  args: string[];
  env: [string, string][];
}

export interface ProviderSettings {
  anthropicApiKey: string;
  lmStudioBaseUrl: string;
}

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

export const mergeTask = (taskId: string) => invoke<MergeOutcome>("merge_task", { taskId });
export const abortMergeTask = (taskId: string) =>
  invoke<void>("abort_merge_task", { taskId });
export const resolverInput = (taskId: string, data: string) =>
  invoke<void>("resolver_input", { taskId, data });
export const resolverStatus = (taskId: string) =>
  invoke<SessionStatus>("resolver_status", { taskId });

export function resolveMerge(
  taskId: string,
  resolverProfile: string,
  onBytes: (bytes: Uint8Array) => void,
): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (msg) => onBytes(b64ToBytes(msg.b64));
  return invoke<void>("resolve_merge", { taskId, resolverProfile, onChunk });
}
