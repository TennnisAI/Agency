use agency_core::git::{self, CommitFile, FileDiff, FileChange};
use agency_core::merge::MergeOutcome;
use agency_core::profile::AgentProfile;
use agency_core::registry::{Project, ReviewComment};
use agency_core::supervisor::AgentStatus;
use agency_core::term::SessionStatus;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

use agency_core::title::{fallback_title, sanitize_title};
use tauri::Manager;
use crate::state::{AppState, MergePreview, ProviderSettings, RunInfo};

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
    merge_target: Option<String>,
) -> Result<RunInfo, String> {
    state
        .create_run(&project_id, &prompt, &agent, &base, merge_target.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_project_branches(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<agency_core::git::ProjectBranches, String> {
    state.list_project_branches(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_terminal(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<RunInfo, String> {
    state.create_terminal(&project_id).map_err(|e| e.to_string())
}

const TITLE_MODEL_ANTHROPIC: &str = "claude-haiku-4-5-20251001";

/// Generate a short title for a run from its first prompt, off the UI thread.
/// No-ops if the run already has a title. Falls back to the first words of the
/// prompt when no provider is configured or the request fails.
#[tauri::command]
pub fn set_run_title(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    first_prompt: String,
) -> Result<(), String> {
    // Cheap guard on the calling thread: skip if already titled.
    if let Ok(Some(existing)) = state.run_title(&id) {
        if !existing.is_empty() {
            return Ok(());
        }
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        let st = handle.state::<AppState>();
        // Re-check under the worker to avoid a race with a concurrent call.
        if matches!(st.run_title(&id), Ok(Some(t)) if !t.is_empty()) {
            return;
        }
        let settings = st.get_settings().unwrap_or(ProviderSettings {
            anthropic_api_key: String::new(),
            lm_studio_base_url: String::new(),
        });
        let title = llm_title(&settings, &first_prompt).unwrap_or_else(|| fallback_title(&first_prompt));
        if !title.is_empty() {
            let _ = st.store_run_title(&id, &title);
        }
    });
    Ok(())
}

fn llm_title(settings: &ProviderSettings, first_prompt: &str) -> Option<String> {
    let instruction = format!(
        "Generate a concise 3-5 word title for a coding task described by this first instruction. \
         Reply with only the title, no quotes and no trailing punctuation.\n\nInstruction: {first_prompt}",
    );
    // connect_timeout keeps a hung or unreachable endpoint from stalling the
    // whole 15s budget — relevant when no provider is configured and the
    // default localhost LM Studio URL isn't actually listening. The longer
    // overall timeout still gives a real provider time to generate.
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;

    if !settings.anthropic_api_key.is_empty() {
        let resp = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &settings.anthropic_api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": TITLE_MODEL_ANTHROPIC,
                "max_tokens": 32,
                "messages": [{ "role": "user", "content": instruction }],
            }))
            .send()
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = resp.json().ok()?;
        let text = body["content"][0]["text"].as_str()?;
        let title = sanitize_title(text);
        return (!title.is_empty()).then_some(title);
    }

    if !settings.lm_studio_base_url.is_empty() {
        let url = format!("{}/chat/completions", settings.lm_studio_base_url.trim_end_matches('/'));
        let resp = client
            .post(url)
            .json(&serde_json::json!({
                "model": "local-model",
                "max_tokens": 32,
                "messages": [{ "role": "user", "content": instruction }],
            }))
            .send()
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = resp.json().ok()?;
        let text = body["choices"][0]["message"]["content"].as_str()?;
        let title = sanitize_title(text);
        return (!title.is_empty()).then_some(title);
    }

    None
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
        // The frontend immediately follows attach with a resize to its real
        // FitAddon dims; 220x50 is the placeholder initial size until then.
        .attach_run(&id, 220, 50, move |bytes| {
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
pub fn git_set_remote(state: State<'_, AppState>, task_id: String, url: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::set_origin(&wt, url.trim()).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBranches {
    /// Live current branch of the run's worktree (the "source").
    pub branch: String,
    /// Resolved merge target branch name (the "destination").
    pub base: String,
}

#[tauri::command]
pub fn run_branches(state: State<'_, AppState>, task_id: String) -> Result<RunBranches, String> {
    let (branch, base) = state.run_branches(&task_id).map_err(|e| e.to_string())?;
    Ok(RunBranches { branch, base })
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
pub fn merge_preview(state: State<'_, AppState>, task_id: String) -> Result<MergePreview, String> {
    state.merge_preview(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn merge_task(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<MergeOutcome, String> {
    let outcome = state.merge_task(&task_id).map_err(|e| e.to_string())?;
    if let MergeOutcome::Conflicts { files } = &outcome {
        let settings = state.notif_settings().unwrap_or_default();
        let (focused, _active) = state.ui_snapshot();
        if settings.merge_attention && !(settings.only_when_unfocused && focused) {
            use tauri_plugin_notification::NotificationExt;
            let _ = app
                .notification()
                .builder()
                .title("Merge needs attention")
                .body(format!("{} file(s) conflict", files.len()))
                .show();
        }
    }
    Ok(outcome)
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
        // See attach_run: real dims arrive via the follow-up resize command.
        .attach_run_script(&id, 220, 50, move |bytes| {
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

#[tauri::command]
pub fn archive_run(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.archive_run(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn restore_run(state: State<'_, AppState>, id: String) -> Result<RunInfo, String> {
    state.restore_run(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_archived_runs(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<RunInfo>, String> {
    state.list_archived_runs(&project_id).map_err(|e| e.to_string())
}

use crate::notifier::NotifSettings;

#[tauri::command]
pub fn set_ui_state(state: State<'_, AppState>, focused: bool, active_run: Option<String>) {
    state.set_ui_state(focused, active_run);
}

#[tauri::command]
pub fn get_notif_settings(state: State<'_, AppState>) -> Result<NotifSettings, String> {
    state.notif_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_notif_settings(state: State<'_, AppState>, settings: NotifSettings) -> Result<(), String> {
    state.save_notif_settings(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_review_comment(
    state: State<'_, AppState>,
    run_id: String,
    path: String,
    line_start: u32,
    line_end: u32,
    body: String,
) -> Result<ReviewComment, String> {
    state
        .add_review_comment(&run_id, &path, line_start, line_end, &body)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_review_comments(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Vec<ReviewComment>, String> {
    state.list_review_comments(&run_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_review_comment(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_review_comment(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn send_review_comments(state: State<'_, AppState>, run_id: String) -> Result<(), String> {
    state.send_review_comments(&run_id).map_err(|e| e.to_string())
}

/// Selects which directory the file commands operate on: a run's worktree or a
/// project's main checkout.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FileRoot {
    Run { id: String },
    Project { id: String },
}

fn resolve_root(state: &AppState, root: &FileRoot) -> Result<std::path::PathBuf, String> {
    match root {
        FileRoot::Run { id } => state.worktree_path(id).map_err(|e| e.to_string()),
        FileRoot::Project { id } => state.project_repo_path(id).map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub fn list_dir(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<Vec<agency_core::files::DirEntry>, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::list_dir(&base, &rel_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn read_file(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<agency_core::files::FileContents, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::read_file(&base, &rel_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn write_file(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
    contents: String,
) -> Result<(), String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::write_file(&base, &rel_path, &contents).map_err(|e| e.to_string())
}
