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

export interface TaskInfo {
  taskId: string;
  branch: string;
}

export interface StatusDto {
  state: string;
  code: number | null;
}

export const listProjects = () => invoke<Project[]>("list_projects");

export const addProject = (name: string, repoPath: string) =>
  invoke<Project>("add_project", { name, repoPath });

export const removeProject = (id: string) =>
  invoke<void>("remove_project", { id });

export const sendInput = (taskId: string, data: string) =>
  invoke<void>("send_input", { taskId, data });

export const taskStatus = (taskId: string) =>
  invoke<StatusDto>("task_status", { taskId });

export const stopTask = (taskId: string) => invoke<void>("stop_task", { taskId });

export function startTask(
  projectId: string,
  prompt: string,
  profile: string,
  onBytes: (bytes: Uint8Array) => void,
): Promise<TaskInfo> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (msg) => onBytes(b64ToBytes(msg.b64));
  return invoke<TaskInfo>("start_task", { projectId, prompt, profile, onChunk });
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
