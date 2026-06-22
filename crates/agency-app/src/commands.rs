use agency_core::git::{self, CommitFile, FileDiff, FileChange};
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessDto {
    pub state: String,
    pub stageable: bool,
    pub dirty: bool,
}

fn readiness_dto(r: agency_core::setup::RepoReadiness) -> ReadinessDto {
    use agency_core::setup::RepoReadiness::*;
    match r {
        NotARepo => ReadinessDto { state: "notARepo".into(), stageable: false, dirty: false },
        NoCommits { stageable } => ReadinessDto { state: "noCommits".into(), stageable, dirty: false },
        Ready { dirty } => ReadinessDto { state: "ready".into(), stageable: false, dirty },
    }
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
pub fn close_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.close_project(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_project(&id).map_err(|e| e.to_string())
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
pub fn stop_run(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.stop_run(&id).map_err(|e| e.to_string())
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
pub fn resize_run(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state.resize_run(&id, cols, rows).map_err(|e| e.to_string())
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
pub fn git_stage_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stage_all(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::unstage_all(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_discard(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    untracked: bool,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::discard(&wt, &path, untracked).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_discard_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::discard_all(&wt).map_err(|e| e.to_string())
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

#[tauri::command]
pub fn resolver_resize(
    state: State<'_, AppState>,
    task_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state.resolver_resize(&task_id, cols, rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_parse_diff(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    staged: bool,
) -> Result<FileDiff, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    let raw = agency_core::git::diff(&wt, &path, staged).map_err(|e| e.to_string())?;
    Ok(agency_core::git::parse_diff(&raw))
}

#[tauri::command]
pub fn git_stage_hunk(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stage_hunk(&wt, &path, hunk_index).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage_hunk(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::unstage_hunk(&wt, &path, hunk_index).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_log_graph(
    state: State<'_, AppState>,
    task_id: String,
    limit: usize,
) -> Result<Vec<agency_core::git::HistoryItem>, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::log_graph(&wt, limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_branch_info(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<agency_core::git::BranchInfo, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::branch_info(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn inspect_repo(state: State<'_, AppState>, repo_path: String) -> Result<ReadinessDto, String> {
    Ok(readiness_dto(state.inspect_repo(std::path::Path::new(&repo_path))))
}

#[tauri::command]
pub fn init_repo(state: State<'_, AppState>, repo_path: String) -> Result<(), String> {
    state.init_repo(std::path::Path::new(&repo_path)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_commit_files(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
) -> Result<Vec<CommitFile>, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::commit_files(&wt, &hash).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_commit_diff(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
    path: String,
) -> Result<String, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::commit_diff(&wt, &hash, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_commit_amend(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::commit_amend(&wt, &message).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn commit_repo(
    state: State<'_, AppState>,
    repo_path: String,
    add_gitignore: bool,
) -> Result<(), String> {
    state
        .commit_repo(std::path::Path::new(&repo_path), add_gitignore)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stage_lines(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
    lines: Vec<usize>,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stage_lines(&wt, &path, hunk_index, &lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage_lines(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
    lines: Vec<usize>,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::unstage_lines(&wt, &path, hunk_index, &lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_revert_lines(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
    lines: Vec<usize>,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::revert_lines(&wt, &path, hunk_index, &lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_script_configured(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    state.run_script_configured(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_run_script(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.start_run_script(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_run_script(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.stop_run_script(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_script_status(state: State<'_, AppState>, id: String) -> Result<SessionStatus, String> {
    state.run_script_status(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_script_preview(
    state: State<'_, AppState>,
    id: String,
    lines: usize,
) -> Result<String, String> {
    state.run_script_preview(&id, lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn attach_run_script(
    state: State<'_, AppState>,
    id: String,
    on_chunk: Channel<TerminalChunk>,
) -> Result<(), String> {
    state
        .attach_run_script(&id, move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn detach_run_script(state: State<'_, AppState>, id: String) {
    state.detach_run_script(&id);
}

#[tauri::command]
pub fn run_script_input(state: State<'_, AppState>, id: String, data: String) -> Result<(), String> {
    state.run_script_input(&id, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resize_run_script(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state.resize_run_script(&id, cols, rows).map_err(|e| e.to_string())
}
