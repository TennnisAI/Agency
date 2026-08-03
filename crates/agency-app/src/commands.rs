use agency_core::git::{self, CommitFile, FileDiff, FileChange};
use agency_core::merge::MergeOutcome;
use agency_core::profile::AgentProfile;
use agency_core::registry::{Issue, IssuePatch, IssueStatus, Project, ReviewComment};
use agency_core::term::SessionStatus;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

use agency_core::title::fallback_title;
use crate::state::{AppState, DiscardSummary, FilesConfigDto, KnowledgeConfigDto, McpImportResult, MergePreview, ProviderSettings, RunInfo, RunScriptConfigDto, RunScriptStatusDto, RunSessionInfo};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalChunk {
    pub b64: String,
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

// NOTE on async: read-only / polled commands are `async fn` so Tauri runs them
// on the async runtime instead of the main thread — sync commands execute on
// the main thread, and these shell out to git or round-trip the terminal
// daemon, so they stacked up behind the 1-2s polls and stalled anything queued
// after them (IPC responses, tray, window chrome) for up to a second. Mutating
// commands stay sync on purpose: the main thread serializes them, which
// doubles as a lock against concurrent git index writes.
//
// A mutating command may go async once something else provides that mutual
// exclusion: `create_run` and the teardowns hold `worktree_gate`, `stop_run`
// and `ensure_run_active` write no index at all. The merge family (`merge_task`,
// `finish_merge_task`, `abort_merge_task`) and `rerun` are the ones still
// waiting on a gate — see AGE-54.
#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
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

// ── workspace (the pinned notes/journal project) ────────────────────────────

#[tauri::command]
pub async fn get_workspace(state: State<'_, AppState>) -> Result<Option<Project>, String> {
    state.get_workspace().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn default_workspace_location(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.default_workspace_location().to_string_lossy().into_owned())
}

#[tauri::command]
pub fn create_workspace(
    state: State<'_, AppState>,
    path: String,
    use_git: bool,
) -> Result<Project, String> {
    state
        .create_workspace(std::path::Path::new(&path), use_git)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn move_workspace(state: State<'_, AppState>, new_path: String) -> Result<Project, String> {
    state
        .move_workspace(std::path::Path::new(&new_path))
        .map_err(|e| e.to_string())
}

// async (not sync): closing a project kills every agent's session, shell and
// extra tabs, one daemon round-trip each. On the main thread a busy project
// froze the window until the last kill came back (see the main-thread note
// above); `on_progress` names the agent it is stopping meanwhile.
#[tauri::command]
pub async fn close_project(
    state: State<'_, AppState>,
    id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<(), String> {
    state
        .close_project_with_progress(&id, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

// async + progress for the same reason `discard_run` is, only more so: this is
// that teardown once per agent in the project — worktree removal and all — so
// it is the slowest one in the app. `delete_project_with_progress` takes the
// worktree_gate per run, standing in for the main-thread serialization it used
// to get for free.
#[tauri::command]
pub async fn delete_project(
    state: State<'_, AppState>,
    id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<(), String> {
    state
        .delete_project_with_progress(&id, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

/// Recolor a project's sidebar icon. `color` must be a palette accent name
/// (see `PROJECT_COLORS`); anything else is rejected by the registry.
#[tauri::command]
pub fn set_project_color(state: State<'_, AppState>, id: String, color: String) -> Result<(), String> {
    state.set_project_color(&id, &color).map_err(|e| e.to_string())
}

// async: creating a workspace shells out to `git worktree add`, which checks
// out the whole tree — seconds to minutes on a large repo. As a sync command
// that ran on the main thread and froze the entire UI until it finished. Async
// runs it on the async runtime, and progress streams back over `on_progress` so
// the UI shows the checkout advancing. `create_run_spec` holds `worktree_gate`
// to replace the main-thread serialization this used to rely on.
#[tauri::command]
pub async fn create_run(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    agent: String,
    base: String,
    merge_target: Option<String>,
    // Absent = the historical behaviour: cut a worktree. `false` runs the agent
    // in the project's own checkout on its current branch.
    worktree: Option<bool>,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<RunInfo, String> {
    state
        .create_run_with_progress(
            &project_id,
            &prompt,
            &agent,
            &base,
            merge_target.as_deref(),
            worktree.unwrap_or(true),
            move |p| {
                let _ = on_progress.send(p);
            },
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_loop(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    agent: String,
    base: String,
    merge_target: Option<String>,
    check_command: String,
    max_attempts: u32,
) -> Result<RunInfo, String> {
    state
        .create_loop(
            &project_id,
            &prompt,
            &agent,
            &base,
            merge_target.as_deref(),
            &check_command,
            max_attempts,
        )
        .map_err(|e| e.to_string())
}

// async: this contends on `loop_gate`, which the watcher thread can hold across
// a multi-second daemon spawn. As a sync command it would block the main thread
// (freezing the UI) for that whole wait; async runs it on the async runtime. It
// writes no git index, so it doesn't need the main-thread serialization the
// note above relies on.
#[tauri::command]
pub async fn stop_loop(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.stop_loop(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_project_branches(
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

#[tauri::command]
pub async fn agent_installed(state: State<'_, AppState>, agent: String) -> Result<bool, String> {
    state.agent_installed(&agent).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_install_terminal(
    state: State<'_, AppState>,
    project_id: String,
    agent: String,
    command: String,
) -> Result<RunInfo, String> {
    state
        .create_install_terminal(&project_id, &agent, &command)
        .map_err(|e| e.to_string())
}

/// Quit for real: invoked by the frontend once the user confirms the styled
/// in-app quit dialog (see lifecycle::request_quit).
#[tauri::command]
pub fn confirm_quit(app: tauri::AppHandle) {
    crate::lifecycle::confirm_quit(&app);
}

/// Derive a short title for a run from its first prompt. No-ops if the run is
/// already titled; otherwise stores the first words of the prompt as the title.
#[tauri::command]
pub fn set_run_title(
    state: State<'_, AppState>,
    id: String,
    first_prompt: String,
) -> Result<(), String> {
    // The first prompt doubles as the run's stored prompt (runs are created
    // promptless; the user types into the live agent). Kept even when the
    // title guard below short-circuits.
    let _ = state.store_run_prompt(&id, &first_prompt);
    // Skip if already titled.
    if let Ok(Some(existing)) = state.run_title(&id) {
        if !existing.is_empty() {
            return Ok(());
        }
    }
    let title = fallback_title(&first_prompt);
    if !title.is_empty() {
        let _ = state.store_run_title(&id, &title);
    }
    Ok(())
}

#[tauri::command]
pub async fn list_runs(state: State<'_, AppState>, project_id: String) -> Result<Vec<RunInfo>, String> {
    state.list_runs(&project_id).map_err(|e| e.to_string())
}

// async (not sync): tearing an agent down stops its session, waits on the
// terminal daemon and hands git a worktree that can be a whole dependency tree
// to unlink — seconds to tens of seconds. Sync ran that on the main thread, so
// the entire window froze with no repaint until it finished (see the
// main-thread note at the top of this file). `on_progress` streams the step it
// is on, the same channel shape clone and push report through.
#[tauri::command]
pub async fn discard_run(
    state: State<'_, AppState>,
    id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<(), String> {
    state
        .discard_run_with_progress(&id, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

/// Discard every archived run in a project at once. The UI confirms first; a
/// partial sweep still returns Ok so the caller can report what went and what
/// didn't (see AppState::discard_archived_runs). Async for the same reason
/// `discard_run` is, only more so — this is that teardown once per archived run.
#[tauri::command]
pub async fn discard_archived_runs(
    state: State<'_, AppState>,
    project_id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<DiscardSummary, String> {
    state
        .discard_archived_runs_with_progress(&project_id, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

// async for the same reason `ensure_run_active` is: stopping a looping run
// takes `loop_gate`, which the loop watcher can hold across a slow daemon
// spawn, and the kills themselves are daemon round-trips. It creates no
// worktree and writes no git index — the loop transition it does make is
// already covered by `loop_gate` — so it is safe off the main thread.
#[tauri::command]
pub async fn stop_run(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.stop_run(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_preview(state: State<'_, AppState>, id: String, lines: usize) -> Result<String, String> {
    state.run_preview(&id, lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn attach_run(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
    on_chunk: Channel<TerminalChunk>,
) -> Result<(), String> {
    state
        // Attach at the frontend's real FitAddon dims so the daemon builds its
        // snapshot at the geometry xterm is already showing. Passing a placeholder
        // here made the snapshot paint at the wrong width, and the follow-up resize
        // then forced a reflow that stacked a second, mis-aligned frame ("decomposed"
        // output on switch-in).
        .attach_run(&id, cols.max(1), rows.max(1), move |bytes| {
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
pub async fn run_status(state: State<'_, AppState>, id: String) -> Result<SessionStatus, String> {
    state.run_status(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rerun(state: State<'_, AppState>, id: String) -> Result<RunInfo, String> {
    state.rerun(&id).map_err(|e| e.to_string())
}

// async: an active-loop run makes this take `loop_gate`, which the watcher can
// hold across a slow daemon spawn. Run off the main thread so waiting on the
// gate (or the session round-trip) can't freeze the UI. It creates no worktree
// and writes no git index — it only revives/keeps a daemon session — so it is
// safe off the main thread.
#[tauri::command]
pub async fn ensure_run_active(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.ensure_run_active(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_status(state: State<'_, AppState>, task_id: String) -> Result<Vec<FileChange>, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::status(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_diff(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    staged: bool,
) -> Result<String, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::diff(&wt, &path, staged).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stage(state: State<'_, AppState>, task_id: String, path: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::stage(&wt, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::unstage(&wt, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stage_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stage_all(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::unstage_all(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_discard(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    untracked: bool,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::discard(&wt, &path, untracked).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_discard_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::discard_all(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_commit(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::commit(&wt, &message).map_err(|e| e.to_string())
}

// async (not sync): a push round-trips the network for as long as the upload
// takes — seconds to minutes for a large changeset. A sync command runs on the
// main thread and froze the whole UI mid-push; async runs it off-thread and
// streams `--progress` to the panel so the user sees movement (like clone).
#[tauri::command]
pub async fn git_push(
    state: State<'_, AppState>,
    task_id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::push_with_progress(&wt, move |p| {
        let _ = on_progress.send(p);
    })
    .map_err(|e| e.to_string())
}

// async, streamed: same as `git_push`, but reconciles both directions before
// pushing (VS Code-style "Sync Changes"). Returns `Diverged` when the branch
// can't fast-forward so the UI can offer to rebase instead of failing outright.
#[tauri::command]
pub async fn git_sync(
    state: State<'_, AppState>,
    task_id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<git::SyncOutcome, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::sync(&wt, move |p| {
        let _ = on_progress.send(p);
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_set_remote(state: State<'_, AppState>, task_id: String, url: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::set_origin(&wt, url.trim()).map_err(|e| e.to_string())
}

// async: both round-trip the network (fetch/pull), which must never run on the
// main thread — a slow or offline remote would freeze the UI otherwise.
#[tauri::command]
pub async fn git_fetch(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::fetch(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_pull(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::pull(&wt).map_err(|e| e.to_string())
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
pub async fn run_branches(state: State<'_, AppState>, task_id: String) -> Result<RunBranches, String> {
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
pub async fn agent_onboarding_needed(state: State<'_, AppState>) -> Result<bool, String> {
    state.agent_onboarding_needed().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_agent_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<crate::agent_catalog::CatalogEntryInfo>, String> {
    state.list_agent_catalog().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn enable_agent_profiles(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<(), String> {
    state.enable_agent_profiles(&ids).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn complete_agent_onboarding(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<(), String> {
    state.complete_agent_onboarding(&ids).map_err(|e| e.to_string())
}

// async (not sync): these only read/write the settings KV, never the git index,
// so per the main-thread note above they must run on the async runtime. Keeping
// get_settings sync put it on the main thread right before the (sync) create_run
// on the "New Agent" path, stalling the spawn.
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<ProviderSettings, String> {
    state.get_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, settings: ProviderSettings) -> Result<(), String> {
    state.save_settings(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn merge_preview(state: State<'_, AppState>, task_id: String) -> Result<MergePreview, String> {
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
        let (focused, active) = state.ui_snapshot();
        let suppressed =
            crate::notifier::suppressed(&settings, focused, active.as_deref(), &task_id);
        if settings.merge_attention && !suppressed {
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
pub fn merge_status(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<agency_core::merge::MergeState, String> {
    state.merge_status(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn finish_merge_task(state: State<'_, AppState>, task_id: String) -> Result<MergeOutcome, String> {
    state.finish_merge_task(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn abort_merge_task(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.abort_merge_task(&task_id).map_err(|e| e.to_string())
}

// PR commands are async: each one shells out to `gh` (network calls that can
// take seconds) and must not block the main thread.

#[tauri::command]
pub async fn gh_readiness(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<agency_core::gh::GhReadiness, String> {
    state.gh_readiness(&project_id).map_err(|e| e.to_string())
}

/// gh install + auth state with no repo in play — used by the clone dialog to
/// guide the user toward signing in when a clone fails on authentication.
#[tauri::command]
pub async fn gh_auth_readiness() -> Result<agency_core::gh::GhReadiness, String> {
    Ok(agency_core::gh::GhCli::default().auth_readiness())
}

#[tauri::command]
pub async fn create_pr(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<agency_core::gh::PrInfo, String> {
    state.create_pr(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pr_status(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<crate::state::PrStatus, String> {
    state.pr_status(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_check_feedback(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.send_check_feedback(&task_id).map_err(|e| e.to_string())
}

// ── In-app PR review ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn pr_detail(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
) -> Result<Option<agency_core::gh::PrDetail>, String> {
    state.pr_detail(&project_id, number).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn gh_current_login(state: State<'_, AppState>, project_id: String) -> Result<String, String> {
    state.gh_current_login(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pr_merge_methods(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<agency_core::gh::MergeMethods, String> {
    state.pr_merge_methods(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn merge_pr(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
    method: String,
    delete_branch: bool,
) -> Result<agency_core::gh::PrMergeResult, String> {
    state.merge_pr(&project_id, number, &method, delete_branch).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pr_diff(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
) -> Result<Vec<agency_core::gh::PrFileDiff>, String> {
    state.pr_diff(&project_id, number).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pr_review_threads(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
) -> Result<Vec<agency_core::gh::ReviewThread>, String> {
    state.pr_review_threads(&project_id, number).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn submit_pr_review(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
    event: String,
    body: Option<String>,
    comments: Vec<agency_core::gh::DraftComment>,
) -> Result<(), String> {
    state
        .submit_pr_review(&project_id, number, &event, body.as_deref(), &comments)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reply_pr_comment(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
    in_reply_to: u64,
    body: String,
) -> Result<(), String> {
    state
        .reply_pr_comment(&project_id, number, in_reply_to, &body)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resolve_pr_thread(
    state: State<'_, AppState>,
    project_id: String,
    thread_id: String,
) -> Result<(), String> {
    state.resolve_pr_thread(&project_id, &thread_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unresolve_pr_thread(
    state: State<'_, AppState>,
    project_id: String,
    thread_id: String,
) -> Result<(), String> {
    state.unresolve_pr_thread(&project_id, &thread_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pr_number_for_run(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<u64>, String> {
    state.pr_number_for_run(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_pr_from_branch(
    state: State<'_, AppState>,
    project_id: String,
    head: String,
    base: Option<String>,
    title: Option<String>,
    body: Option<String>,
) -> Result<agency_core::gh::PrInfo, String> {
    state
        .create_pr_from_branch(&project_id, &head, base.as_deref(), title.as_deref(), body.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_race(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    agents: Vec<String>,
    base: String,
    merge_target: Option<String>,
) -> Result<Vec<RunInfo>, String> {
    state
        .create_race(&project_id, &prompt, &agents, &base, merge_target.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_gh_issues(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<agency_core::gh::IssueItem>, String> {
    let repo = state.project_repo_path(&project_id).map_err(|e| e.to_string())?;
    agency_core::gh::GhCli::default().list_issues(&repo).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_gh_prs(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<agency_core::gh::PrInfo>, String> {
    let repo = state.project_repo_path(&project_id).map_err(|e| e.to_string())?;
    agency_core::gh::GhCli::default().list_prs(&repo).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_run_from_issue(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
    agent: String,
) -> Result<RunInfo, String> {
    state.create_run_from_issue(&project_id, number, &agent).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_run_from_pr(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
    agent: String,
) -> Result<RunInfo, String> {
    state.create_run_from_pr(&project_id, number, &agent).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_pr_review_run(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
    agent: String,
    post_comments: bool,
) -> Result<crate::state::PrReviewRun, String> {
    state
        .create_pr_review_run(&project_id, number, &agent, post_comments)
        .map_err(|e| e.to_string())
}

// ── issues (the local tracker; GitHub issues are create_run_from_issue) ─────

#[tauri::command]
pub async fn list_issues(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<Issue>, String> {
    state.list_issues(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_issue(
    state: State<'_, AppState>,
    project_id: String,
    title: String,
    body: String,
    status: IssueStatus,
) -> Result<Issue, String> {
    state.create_issue(&project_id, &title, &body, status).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_issue(
    state: State<'_, AppState>,
    id: String,
    patch: IssuePatch,
) -> Result<Issue, String> {
    state.update_issue(&id, &patch).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_issue(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_issue(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_issue_run(
    state: State<'_, AppState>,
    issue_id: String,
    agent: String,
    base: Option<String>,
    merge_target: Option<String>,
) -> Result<RunInfo, String> {
    state
        .start_issue_run(&issue_id, &agent, base.as_deref(), merge_target.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_issue_race(
    state: State<'_, AppState>,
    issue_id: String,
    agents: Vec<String>,
    base: Option<String>,
    merge_target: Option<String>,
) -> Result<Vec<RunInfo>, String> {
    state
        .start_issue_race(&issue_id, &agents, base.as_deref(), merge_target.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_issue_loop(
    state: State<'_, AppState>,
    issue_id: String,
    agent: String,
    check_command: String,
    max_attempts: u32,
    base: Option<String>,
    merge_target: Option<String>,
) -> Result<RunInfo, String> {
    state
        .start_issue_loop(&issue_id, &agent, &check_command, max_attempts, base.as_deref(), merge_target.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_mcp_servers(
    state: State<'_, AppState>,
) -> Result<Vec<agency_core::mcp::McpServer>, String> {
    state.list_mcp_servers().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_mcp_servers(
    state: State<'_, AppState>,
    servers: Vec<agency_core::mcp::McpServer>,
) -> Result<(), String> {
    state.save_mcp_servers(&servers).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_mcp_json(
    state: State<'_, AppState>,
    text: String,
) -> Result<McpImportResult, String> {
    state.import_mcp_json(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn authenticate_mcp_server(
    state: State<'_, AppState>,
    project_id: String,
    agent: String,
    name: String,
) -> Result<RunInfo, String> {
    state
        .authenticate_mcp_server(&project_id, &agent, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn deauthenticate_mcp_server(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.deauthenticate_mcp_server(&name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_knowledge_config(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<KnowledgeConfigDto, String> {
    state.knowledge_config(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_knowledge_config(
    state: State<'_, AppState>,
    project_id: String,
    graph: bool,
    serve_command: Option<String>,
    build_command: Option<String>,
) -> Result<(), String> {
    state
        .save_knowledge_config(&project_id, graph, serve_command, build_command)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_files_config(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<FilesConfigDto, String> {
    state.files_config(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_files_config(
    state: State<'_, AppState>,
    project_id: String,
    copy: Vec<String>,
) -> Result<(), String> {
    state.save_files_config(&project_id, copy).map_err(|e| e.to_string())
}

/// "Fix with agent" on a conflicted merge: type the conflict into this run's
/// own agent session rather than spawning a separate resolver.
#[tauri::command]
pub fn send_merge_conflict(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.send_merge_conflict(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_parse_diff(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    staged: bool,
) -> Result<FileDiff, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
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
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stage_hunk(&wt, &path, hunk_index).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage_hunk(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::unstage_hunk(&wt, &path, hunk_index).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_log_graph(
    state: State<'_, AppState>,
    task_id: String,
    limit: usize,
) -> Result<Vec<agency_core::git::HistoryItem>, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::log_graph(&wt, limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_branch_info(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<agency_core::git::BranchInfo, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::branch_info(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn inspect_repo(state: State<'_, AppState>, repo_path: String) -> Result<ReadinessDto, String> {
    Ok(readiness_dto(state.inspect_repo(std::path::Path::new(&repo_path))))
}

#[tauri::command]
pub fn init_repo(state: State<'_, AppState>, repo_path: String) -> Result<(), String> {
    state.init_repo(std::path::Path::new(&repo_path)).map_err(|e| e.to_string())
}

// Clones a remote repo into a new folder under `parent_dir` and returns the
// absolute path of the clone, ready to be added as a project. `on_progress`
// streams git's download progress to the dialog.
//
// async (not sync): a clone shells out to git for as long as the download takes
// — minutes for a large repo. A sync command runs on the main thread, which
// froze the whole UI mid-clone (see the main-thread note at the top of this
// file); async runs it on the async runtime instead.
#[tauri::command]
pub async fn clone_repo(
    state: State<'_, AppState>,
    url: String,
    parent_dir: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<String, String> {
    let dest = state
        .clone_repo(&url, std::path::Path::new(&parent_dir), move |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn git_commit_files(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
) -> Result<Vec<CommitFile>, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::commit_files(&wt, &hash).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_commit_diff(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
    path: String,
) -> Result<String, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::commit_diff(&wt, &hash, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_commit_amend(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::commit_amend(&wt, &message).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_checkout_branch(state: State<'_, AppState>, task_id: String, name: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::checkout_branch(&wt, &name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_create_branch(
    state: State<'_, AppState>,
    task_id: String,
    name: String,
    from: Option<String>,
    checkout: bool,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::create_branch(&wt, &name, from.as_deref(), checkout).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_delete_branch(
    state: State<'_, AppState>,
    task_id: String,
    name: String,
    force: bool,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::delete_branch(&wt, &name, force).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_list_branches(state: State<'_, AppState>, task_id: String) -> Result<agency_core::git::ProjectBranches, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::list_branches(&wt).map_err(|e| e.to_string())
}

// async: round-trips the network, must never block the main thread.
#[tauri::command]
pub async fn git_pull_rebase(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::pull_rebase(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_push_force(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::push_force(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_undo_last_commit(state: State<'_, AppState>, task_id: String) -> Result<String, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::undo_last_commit(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_reset_to(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
    mode: String,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::reset_to(&wt, &hash, &mode).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_revert_commit(state: State<'_, AppState>, task_id: String, hash: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::revert_commit(&wt, &hash).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_cherry_pick(state: State<'_, AppState>, task_id: String, hash: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::cherry_pick(&wt, &hash).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stash_list(state: State<'_, AppState>, task_id: String) -> Result<Vec<agency_core::git::StashEntry>, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stash_list(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stash_push(
    state: State<'_, AppState>,
    task_id: String,
    message: Option<String>,
    include_untracked: bool,
) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stash_push(&wt, message.as_deref(), include_untracked).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stash_apply(state: State<'_, AppState>, task_id: String, index: usize) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stash_apply(&wt, index).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stash_pop(state: State<'_, AppState>, task_id: String, index: usize) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stash_pop(&wt, index).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stash_drop(state: State<'_, AppState>, task_id: String, index: usize) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stash_drop(&wt, index).map_err(|e| e.to_string())
}

// async (not sync): the initial commit on a folder that isn't a repo yet means
// `git add -A` over every file in it — minutes for a large tree. A sync command
// runs on the main thread, which froze the whole UI with no feedback (see the
// main-thread note at the top of this file); async runs it off-thread and
// streams the staged-file count to the dialog (like clone).
#[tauri::command]
pub async fn commit_repo(
    state: State<'_, AppState>,
    repo_path: String,
    add_gitignore: bool,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<(), String> {
    state
        .commit_repo(std::path::Path::new(&repo_path), add_gitignore, move |p| {
            let _ = on_progress.send(p);
        })
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
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
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
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
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
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::revert_lines(&wt, &path, hunk_index, &lines).map_err(|e| e.to_string())
}

// ── Run scripts ──────────────────────────────────────────────────────────────
//
// `target` is the workspace the scripts run in: a run id for an agent's
// workspace, or `project:<id>` for the project's own checkout. `script` is the
// entry's name in the project's run list.

#[tauri::command]
pub async fn run_script_config(
    state: State<'_, AppState>,
    target: String,
) -> Result<RunScriptConfigDto, String> {
    state.run_script_config(&target).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_run_scripts(
    state: State<'_, AppState>,
    target: String,
    scripts: Vec<agency_core::config::RunScript>,
) -> Result<(), String> {
    state.save_run_scripts(&target, scripts).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_run_script(
    state: State<'_, AppState>,
    target: String,
    script: String,
) -> Result<(), String> {
    state.start_run_script(&target, &script).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_run_script(
    state: State<'_, AppState>,
    target: String,
    script: String,
) -> Result<(), String> {
    state.stop_run_script(&target, &script).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_scripts_status(
    state: State<'_, AppState>,
    target: String,
) -> Result<Vec<RunScriptStatusDto>, String> {
    state.run_scripts_status(&target).map_err(|e| e.to_string())
}

/// Whether anything is running in this workspace's run scripts, for the dot on
/// the project's Run tab. Agents carry the same flag on their `RunInfo`, so the
/// board doesn't need a call per agent.
#[tauri::command]
pub async fn run_scripts_live(state: State<'_, AppState>, target: String) -> Result<bool, String> {
    state.run_scripts_live(&target).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_script_preview(
    state: State<'_, AppState>,
    target: String,
    script: String,
    lines: usize,
) -> Result<String, String> {
    state.run_script_preview(&target, &script, lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn attach_run_script(
    state: State<'_, AppState>,
    target: String,
    script: String,
    cols: u16,
    rows: u16,
    on_chunk: Channel<TerminalChunk>,
) -> Result<(), String> {
    state
        // See attach_run: attach at the frontend's real FitAddon dims.
        .attach_run_script(&target, &script, cols.max(1), rows.max(1), move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn detach_run_script(state: State<'_, AppState>, target: String, script: String) {
    state.detach_run_script(&target, &script);
}

#[tauri::command]
pub fn run_script_input(
    state: State<'_, AppState>,
    target: String,
    script: String,
    data: String,
) -> Result<(), String> {
    state.run_script_input(&target, &script, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resize_run_script(
    state: State<'_, AppState>,
    target: String,
    script: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state.resize_run_script(&target, &script, cols, rows).map_err(|e| e.to_string())
}

// ── Companion shell (per-run interactive terminal in the run's worktree) ──────

#[tauri::command]
pub fn start_shell(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.start_shell(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_shell(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.stop_shell(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn shell_status(state: State<'_, AppState>, id: String) -> Result<SessionStatus, String> {
    state.shell_status(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn shell_preview(
    state: State<'_, AppState>,
    id: String,
    lines: usize,
) -> Result<String, String> {
    state.shell_preview(&id, lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn attach_shell(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
    on_chunk: Channel<TerminalChunk>,
) -> Result<(), String> {
    state
        // See attach_run: attach at the frontend's real FitAddon dims.
        .attach_shell(&id, cols.max(1), rows.max(1), move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn detach_shell(state: State<'_, AppState>, id: String) {
    state.detach_shell(&id);
}

#[tauri::command]
pub fn shell_input(state: State<'_, AppState>, id: String, data: String) -> Result<(), String> {
    state.shell_input(&id, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resize_shell(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state.resize_shell(&id, cols, rows).map_err(|e| e.to_string())
}

// ── Extra agent sessions (additional agent tabs sharing a run's worktree) ─────
// The returned session ids are composite (`<run_id>--<n>`) and are accepted by
// the ordinary run-terminal commands (attach_run/run_input/resize_run/…).

#[tauri::command]
pub fn start_run_session(
    state: State<'_, AppState>,
    run_id: String,
    agent: Option<String>,
) -> Result<RunSessionInfo, String> {
    state.start_run_session(&run_id, agent.as_deref(), "").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_run_sessions(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Vec<RunSessionInfo>, String> {
    state.run_sessions(&run_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn close_run_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.close_run_session(&id).map_err(|e| e.to_string())
}

// async + progress for the same reason `discard_run` is: archiving auto-commits
// the worktree, may run the project's archive script, and then removes the
// worktree. On the main thread that froze the window until it was done.
#[tauri::command]
pub async fn archive_run(
    state: State<'_, AppState>,
    id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<(), String> {
    state
        .archive_run_with_progress(&id, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn restore_run(state: State<'_, AppState>, id: String) -> Result<RunInfo, String> {
    state.restore_run(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_archived_runs(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<RunInfo>, String> {
    state.list_archived_runs(&project_id).map_err(|e| e.to_string())
}

use crate::notifier::NotifSettings;

/// Sync the native menu's context-dependent items with the current selection:
/// `project` enables the project-gated items (New Agent/Terminal, Source), and
/// `focused_agent` enables the Agent menu.
///
/// async (not sync): this fires on every selection/focus change, and a sync
/// command would run on the main thread and queue behind the mutating git
/// commands (create_run/discard_run) that also live there — adding menu work to
/// the very path it shouldn't slow. As async it runs on the async runtime and
/// only the fast `set_context` closure touches the main thread (menu mutation is
/// main-thread-only on macOS).
#[tauri::command]
pub async fn set_menu_context(app: tauri::AppHandle, project: bool, focused_agent: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        crate::menu::set_context(&handle, project, focused_agent);
    });
}

#[tauri::command]
pub fn set_ui_state(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    focused: bool,
    active_run: Option<String>,
) {
    // A fresh focus edge right after a notification means the user (most
    // likely) clicked it — macOS offers no real click callback, so deep-link
    // to the notified run via the same event the tray menu uses.
    if let Some((project_id, run_id)) = state.set_ui_state(focused, active_run) {
        use tauri::Emitter;
        let _ = app.emit("tray-open-run", crate::tray::OpenRun { project_id, run_id });
    }
}

/// Ask GitHub whether a newer release exists. Async so the curl call never
/// blocks the UI thread; returns a report rather than failing when offline.
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<crate::update::UpdateCheck, String> {
    let current = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || crate::update::check(&current))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_update_check_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    state.update_check_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_update_check_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    state.set_update_check_enabled(enabled).map_err(|e| e.to_string())
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
pub async fn list_dir(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<Vec<agency_core::files::DirEntry>, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::list_dir(&base, &rel_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_file(
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

#[tauri::command]
pub fn create_file(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<(), String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::create_file(&base, &rel_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_dir(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<(), String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::create_dir(&base, &rel_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_path(
    state: State<'_, AppState>,
    root: FileRoot,
    from: String,
    to: String,
) -> Result<(), String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::rename_path(&base, &from, &to).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn trash_path(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<(), String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::trash_path(&base, &rel_path).map_err(|e| e.to_string())
}

/// Append a file/dir to the root's `.gitignore`. Returns whether a new entry was
/// added (false = the path was already ignored) so the UI can toast accordingly.
#[tauri::command]
pub fn add_to_gitignore(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<bool, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::add_to_gitignore(&base, &rel_path).map_err(|e| e.to_string())
}

/// Absolute path of a file/dir within a root, for "Copy path".
#[tauri::command]
pub fn abs_path(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<String, String> {
    let base = resolve_root(&state, &root)?;
    let p = agency_core::files::abs_path(&base, &rel_path).map_err(|e| e.to_string())?;
    Ok(p.to_string_lossy().into_owned())
}

/// Reveal a file/dir in the OS file manager (Finder / Explorer). Runs the
/// opener plugin from Rust, so the path is validated by `resolve_within` rather
/// than the plugin's static capability scope.
#[tauri::command]
pub fn reveal_path(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let base = resolve_root(&state, &root)?;
    let p = agency_core::files::abs_path(&base, &rel_path).map_err(|e| e.to_string())?;
    app.opener()
        .reveal_item_in_dir(p)
        .map_err(|e| e.to_string())
}

/// Rename a run: overwrite its display title unconditionally (unlike
/// set_run_title, which only fills an empty title from the first prompt). An
/// empty/whitespace title clears it, so the name falls back to prompt/branch.
#[tauri::command]
pub fn rename_run(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<(), String> {
    state.store_run_title(&id, title.trim()).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryContents {
    pub b64: String,
    pub mime: String,
    pub too_large: bool,
}

/// Case-insensitive lookup of a project's top-level `docs` directory. Returns
/// the actual on-disk name (`"Docs"`, `"DOCS"`, ...) or None when absent, so the
/// Docs tab can offer to create one.
#[tauri::command]
pub async fn detect_docs_dir(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Option<String>, String> {
    // The workspace's *whole folder* is the vault — its docs root is the empty
    // relative path, not a `docs/` subfolder.
    if state.project_is_workspace(&project_id).map_err(|e| e.to_string())? {
        return Ok(Some(String::new()));
    }
    let base = state.project_repo_path(&project_id).map_err(|e| e.to_string())?;
    agency_core::files::find_dir_case_insensitive(&base, "docs").map_err(|e| e.to_string())
}

/// All markdown files under a root's docs directory in one call — powers the
/// Docs tab's link/tag/search index without N read_file round-trips per poll.
#[tauri::command]
pub async fn read_docs_corpus(
    state: State<'_, AppState>,
    root: FileRoot,
    docs_dir: String,
) -> Result<Vec<agency_core::files::DocFile>, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::read_markdown_corpus(&base, &docs_dir).map_err(|e| e.to_string())
}

/// Stat-only corpus pass: `(path, mtime, size)` per markdown file, so the
/// docs poll can detect change without re-reading bodies.
#[tauri::command]
pub async fn docs_corpus_stats(
    state: State<'_, AppState>,
    root: FileRoot,
    docs_dir: String,
) -> Result<Vec<agency_core::files::DocStat>, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::scan_markdown_stats(&base, &docs_dir).map_err(|e| e.to_string())
}

/// Read a named subset of the docs corpus — the poll's "these changed" list.
#[tauri::command]
pub async fn read_docs_files(
    state: State<'_, AppState>,
    root: FileRoot,
    docs_dir: String,
    paths: Vec<String>,
) -> Result<Vec<agency_core::files::DocFile>, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::read_markdown_files(&base, &docs_dir, &paths).map_err(|e| e.to_string())
}

/// Seed the workspace's Welcome guide if it was deleted and return its note
/// path. Backs the palette's "Workspace Guide" command; creation-time seeding
/// happens in create_workspace. Sync: it writes a file.
#[tauri::command]
pub fn ensure_workspace_guide(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<String, String> {
    let base = state.project_repo_path(&project_id).map_err(|e| e.to_string())?;
    agency_core::guide::ensure_guide(&base).map_err(|e| e.to_string())?;
    Ok(agency_core::guide::GUIDE_FILE.to_string())
}

/// Every checkbox task in a root's docs corpus (one-stop Phase 8) — same walk
/// as the corpus reads, but only task lines cross the IPC boundary. Async:
/// read-only and polled from Home.
#[tauri::command]
pub async fn scan_tasks(
    state: State<'_, AppState>,
    root: FileRoot,
    docs_dir: String,
) -> Result<Vec<agency_core::files::TaskHit>, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::scan_tasks(&base, &docs_dir).map_err(|e| e.to_string())
}

/// Flip one checkbox task in place (one-stop Phase 8). Sync like the other
/// mutating file commands; the core re-verifies the line before writing and
/// returns false when the caller's view was stale.
#[tauri::command]
pub fn toggle_task(
    state: State<'_, AppState>,
    root: FileRoot,
    docs_dir: String,
    rel_path: String,
    line: u32,
    checked: bool,
) -> Result<bool, String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::toggle_task(&base, &docs_dir, &rel_path, line, checked)
        .map_err(|e| e.to_string())
}

/// Content search under `dir` within a root (one-stop Phase 2). Hit paths come
/// back relative to `dir`. Async on purpose: a search must never wedge the
/// main thread — the core enforces hit/byte/time caps so it always returns.
#[tauri::command]
pub async fn search_files(
    state: State<'_, AppState>,
    root: FileRoot,
    dir: String,
    query: agency_core::search::SearchQuery,
) -> Result<Vec<agency_core::search::SearchHit>, String> {
    let base = resolve_root(&state, &root)?;
    let target = agency_core::files::abs_path(&base, &dir).map_err(|e| e.to_string())?;
    agency_core::search::search_files(&target, &query).map_err(|e| e.to_string())
}

/// Sorted relative file paths under `dir` within a root — quick-open's name
/// list (one-stop Phase 3). Same file set as the fallback search engine:
/// gitignore respected in repos, bounded walk elsewhere. Async like
/// search_files: listing a big tree must not wedge the IPC thread.
#[tauri::command]
pub async fn list_files(
    state: State<'_, AppState>,
    root: FileRoot,
    dir: String,
    max_files: usize,
) -> Result<Vec<String>, String> {
    let base = resolve_root(&state, &root)?;
    let target = agency_core::files::abs_path(&base, &dir).map_err(|e| e.to_string())?;
    Ok(agency_core::search::list_root_files(&target, max_files))
}

/// Write base64-decoded bytes to a new file (refuses to clobber). Used for
/// pasting images into docs notes.
#[tauri::command]
pub fn write_file_base64(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
    b64: String,
) -> Result<(), String> {
    let base = resolve_root(&state, &root)?;
    let bytes = STANDARD.decode(b64.as_bytes()).map_err(|e| e.to_string())?;
    agency_core::files::write_file_bytes(&base, &rel_path, &bytes).map_err(|e| e.to_string())
}

/// Copy a file from outside the root into it (refuses to clobber). Used for
/// attaching dropped or picked files to an issue, where the UI holds a path
/// rather than the bytes.
#[tauri::command]
pub fn import_file(
    state: State<'_, AppState>,
    root: FileRoot,
    src_path: String,
    rel_path: String,
) -> Result<(), String> {
    let base = resolve_root(&state, &root)?;
    agency_core::files::import_file(&base, std::path::Path::new(&src_path), &rel_path)
        .map_err(|e| e.to_string())
}

/// Raw file bytes as base64, for the file browser's image/PDF previews.
#[tauri::command]
pub async fn read_file_base64(
    state: State<'_, AppState>,
    root: FileRoot,
    rel_path: String,
) -> Result<BinaryContents, String> {
    let base = resolve_root(&state, &root)?;
    let f = agency_core::files::read_file_bytes(&base, &rel_path).map_err(|e| e.to_string())?;
    Ok(BinaryContents {
        b64: STANDARD.encode(&f.bytes),
        mime: f.mime.to_string(),
        too_large: f.too_large,
    })
}
