use agency_core::git::{self, CommitInfo, FileChange};
use agency_core::merge::MergeOutcome;
use agency_core::profile::AgentProfile;
use agency_core::registry::Project;
use agency_core::supervisor::AgentStatus;
use agency_core::tmux::SessionStatus;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::state::{AppState, ProviderSettings, RunInfo};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalChunk {
    pub b64: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub state: String,
    pub code: Option<i32>,
}

fn agent_status_dto(status: AgentStatus) -> StatusDto {
    match status {
        AgentStatus::Running => StatusDto { state: "running".into(), code: None },
        AgentStatus::Idle => StatusDto { state: "idle".into(), code: None },
        AgentStatus::Exited(c) => StatusDto { state: "exited".into(), code: Some(c) },
        AgentStatus::Crashed => StatusDto { state: "crashed".into(), code: None },
    }
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    state.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_project(
    state: State<'_, AppState>,
    name: String,
    repo_path: String,
) -> Result<Project, String> {
    state
        .add_project(&name, std::path::Path::new(&repo_path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.remove_project(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_run(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    agent: String,
    base: String,
) -> Result<RunInfo, String> {
    state
        .create_run(&project_id, &prompt, &agent, &base)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_runs(state: State<'_, AppState>, project_id: String) -> Result<Vec<RunInfo>, String> {
    state.list_runs(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn discard_run(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.discard_run(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_preview(state: State<'_, AppState>, id: String, lines: usize) -> Result<String, String> {
    state.run_preview(&id, lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn attach_run(
    state: State<'_, AppState>,
    id: String,
    on_chunk: Channel<TerminalChunk>,
) -> Result<(), String> {
    state
        .attach_run(&id, move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn detach_run(state: State<'_, AppState>, id: String) {
    state.detach_run(&id);
}

#[tauri::command]
pub fn run_input(state: State<'_, AppState>, id: String, data: String) -> Result<(), String> {
    state.run_input(&id, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_status(state: State<'_, AppState>, id: String) -> Result<SessionStatus, String> {
    state.run_status(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rerun(state: State<'_, AppState>, id: String) -> Result<RunInfo, String> {
    state.rerun(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_status(state: State<'_, AppState>, task_id: String) -> Result<Vec<FileChange>, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::status(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_diff(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    staged: bool,
) -> Result<String, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::diff(&wt, &path, staged).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_log(state: State<'_, AppState>, task_id: String) -> Result<Vec<CommitInfo>, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::log(&wt, 100).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stage(state: State<'_, AppState>, task_id: String, path: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::stage(&wt, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::unstage(&wt, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_commit(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::commit(&wt, &message).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_push(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::push(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> Result<Vec<AgentProfile>, String> {
    state.list_profiles().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_profile(state: State<'_, AppState>, profile: AgentProfile) -> Result<(), String> {
    state.register_profile(profile).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.delete_profile(&name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<ProviderSettings, String> {
    state.get_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: ProviderSettings) -> Result<(), String> {
    state.save_settings(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn merge_task(state: State<'_, AppState>, task_id: String) -> Result<MergeOutcome, String> {
    state.merge_task(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn abort_merge_task(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.abort_merge_task(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resolve_merge(
    state: State<'_, AppState>,
    task_id: String,
    resolver_profile: String,
    on_chunk: Channel<TerminalChunk>,
) -> Result<(), String> {
    state
        .resolve_merge(&task_id, &resolver_profile, move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resolver_input(state: State<'_, AppState>, task_id: String, data: String) -> Result<(), String> {
    state.resolver_input(&task_id, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resolver_status(state: State<'_, AppState>, task_id: String) -> Result<StatusDto, String> {
    state.resolver_status(&task_id).map(agent_status_dto).map_err(|e| e.to_string())
}
