use agency_core::git::{self, CommitFile, FileChange, FileDiff};
use agency_core::merge::MergeOutcome;
use agency_core::profile::AgentProfile;
use agency_core::registry::{Issue, IssuePatch, IssueStatus, Project, ReviewComment};
use agency_core::term::SessionStatus;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

use crate::state::{
    AppState, DiscardSummary, FilesConfigDto, KnowledgeConfigDto, McpImportResult, MergePreview,
    ProviderSettings, RaceAttempt, RunInfo, RunScriptConfigDto, RunScriptStatusDto, RunSessionInfo,
};
use agency_core::title::fallback_title;

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
        NoCommits { stageable } => {
            ReadinessDto { state: "noCommits".into(), stageable, dirty: false }
        }
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
// exclusion. The gates that stand in for the main thread, and what each covers:
//
//   worktree_gate  one at a time across the app: `create_run` and the
//                  teardowns, which add and remove git worktrees.
//   repo_gates     per project: the merge family, and every mutating `git_*`
//                  command pointed at that project's own checkout (a
//                  `project:<id>` target, a terminal, an agent with no worktree
//                  of its own). They all write the one index the merge moves.
//   spawn_gates    per session: `rerun`, `ensure_run_active` and `stop_run`,
//                  which kill and respawn a session the daemon registers by id.
//   loop_gate      one at a time across the app: every loop transition,
//                  including the driver's own spawns.
//
// A mutating command that writes no index and spawns nothing (`stop_loop`,
// settings, the registry-only commands) needs none of them.
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
    state.add_project(&name, std::path::Path::new(&repo_path)).map_err(|e| e.to_string())
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
    state.create_workspace(std::path::Path::new(&path), use_git).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn move_workspace(state: State<'_, AppState>, new_path: String) -> Result<Project, String> {
    state.move_workspace(std::path::Path::new(&new_path)).map_err(|e| e.to_string())
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
pub fn set_project_color(
    state: State<'_, AppState>,
    id: String,
    color: String,
) -> Result<(), String> {
    state.set_project_color(&id, &color).map_err(|e| e.to_string())
}

// async: creating a workspace shells out to `git worktree add`, which checks
// out the whole tree — seconds to minutes on a large repo. As a sync command
// that ran on the main thread and froze the entire UI until it finished. Async
// runs it on the async runtime, and progress streams back over `on_progress` so
// the UI shows the checkout advancing. `create_run_spec` holds `worktree_gate`
// to replace the main-thread serialization this used to rely on.
/// Validate a model id on its way in from the UI. Every command that starts an
/// agent goes through here, so nothing reaches an argv without being checked
/// once, in one place.
fn checked_model(model: Option<String>) -> Result<Option<String>, String> {
    match model {
        Some(m) => crate::agent_catalog::sanitize_model(&m),
        None => Ok(None),
    }
}

/// The same check for every attempt in a race. A bad id fails the whole race
/// rather than quietly dropping one attempt's model: a race whose attempts did
/// not run on the models asked for compares the wrong things.
fn checked_attempts(attempts: Vec<RaceAttempt>) -> Result<Vec<RaceAttempt>, String> {
    attempts
        .into_iter()
        .map(|a| Ok(RaceAttempt { agent: a.agent, model: checked_model(a.model)? }))
        .collect()
}

#[tauri::command]
pub async fn create_run(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    agent: String,
    model: Option<String>,
    base: String,
    merge_target: Option<String>,
    // Absent = the historical behaviour: cut a worktree. `false` runs the agent
    // in the project's own checkout on its current branch.
    worktree: Option<bool>,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<RunInfo, String> {
    let model = checked_model(model)?;
    state
        .create_run_with_progress(
            &project_id,
            &prompt,
            &agent,
            model.as_deref(),
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
    model: Option<String>,
    base: String,
    merge_target: Option<String>,
    check_command: String,
    max_attempts: u32,
) -> Result<RunInfo, String> {
    let model = checked_model(model)?;
    state
        .create_loop(
            &project_id,
            &prompt,
            &agent,
            model.as_deref(),
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
pub fn create_terminal(state: State<'_, AppState>, project_id: String) -> Result<RunInfo, String> {
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
    state.create_install_terminal(&project_id, &agent, &command).map_err(|e| e.to_string())
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
pub async fn list_runs(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<RunInfo>, String> {
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
pub async fn run_preview(
    state: State<'_, AppState>,
    id: String,
    lines: usize,
) -> Result<String, String> {
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

// async: a rerun is a kill plus a spawn, two daemon round-trips, and on the
// main thread the window sat still for both. `AppState::rerun` holds this
// session's `spawn_gates` entry, which is the mutual exclusion the main thread
// used to provide: without it two overlapping reruns would leave two agent
// processes with only the second one registered.
#[tauri::command]
pub async fn rerun(state: State<'_, AppState>, id: String) -> Result<RunInfo, String> {
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
pub async fn git_status(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<FileChange>, String> {
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

// The mutating git commands below are async and go through `git_mutate` rather
// than `git_root`. Every one of them can be pointed at the project's own
// checkout (a `project:<id>` target, a terminal, an agent working without a
// worktree), and that is the checkout the merge family moves between branches,
// so `git_mutate` holds the project's gate for exactly those targets. An
// agent's private worktree has an index of its own and is written unguarded,
// the way it always was.
#[tauri::command]
pub async fn git_stage(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
) -> Result<(), String> {
    state.git_mutate(&task_id, |wt| git::stage(wt, &path)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_unstage(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
) -> Result<(), String> {
    state.git_mutate(&task_id, |wt| git::unstage(wt, &path)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_stage_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.git_mutate(&task_id, agency_core::git::stage_all).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_unstage_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.git_mutate(&task_id, agency_core::git::unstage_all).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_discard(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    untracked: bool,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::discard(wt, &path, untracked))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_discard_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.git_mutate(&task_id, agency_core::git::discard_all).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_commit(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<(), String> {
    state.git_mutate(&task_id, |wt| git::commit(wt, &message)).map_err(|e| e.to_string())
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
    state
        .push_run(&task_id, move |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

// Stops the push (or the push half of a sync) running against this run's
// worktree by killing the git it is waiting on. Sync and trivial for the same
// reason as `cancel_clone`: it only flips a flag, and it has to be answered
// while the push it cancels still holds the async runtime. `git_push` then
// fails with "cancelled", which is the user's own doing and not a failure to
// report; nothing is cleaned up, since a killed push leaves origin unchanged.
#[tauri::command]
pub fn cancel_push(state: State<'_, AppState>, task_id: String) {
    state.cancel_push(&task_id);
}

// async, streamed: same as `git_push`, but reconciles both directions before
// pushing (VS Code-style "Sync Changes"). Returns `Diverged` when the branch
// can't fast-forward so the UI can offer to rebase instead of failing outright.
// Gated, unlike push: the pull half rewrites the working tree.
#[tauri::command]
pub async fn git_sync(
    state: State<'_, AppState>,
    task_id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<git::SyncOutcome, String> {
    state
        .sync_run(&task_id, move |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_set_remote(
    state: State<'_, AppState>,
    task_id: String,
    url: String,
) -> Result<(), String> {
    state.git_mutate(&task_id, |wt| git::set_origin(wt, url.trim())).map_err(|e| e.to_string())
}

// async: both round-trip the network (fetch/pull), which must never run on the
// main thread — a slow or offline remote would freeze the UI otherwise. Only
// `pull` takes the checkout gate: a fetch writes refs, not the working tree, and
// gating it would park a merge behind a slow remote for nothing.
#[tauri::command]
pub async fn git_fetch(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    git::fetch(&wt).map_err(|e| e.to_string())
}

/// The fetch half of "refresh", done for the user instead of by them: the
/// Source Control panel calls this when it opens and when the window regains
/// focus, so ahead/behind is already current by the time it is read. Throttled
/// and backed off per project in `AppState` (shared with the background sweep),
/// which is why the panel can call it freely.
///
/// A worktree shares its project's object store and remote-tracking refs, so
/// this fetches the project once however many agents are running in it.
/// Deliberately quiet: it reports whether a fetch ran, and swallows the failure
/// otherwise. Nobody asked for this fetch, so a remote that is offline or
/// unauthenticated must not put an error banner over the panel — the manual
/// Fetch/⟲ action still surfaces those.
#[tauri::command]
pub async fn git_auto_fetch(state: State<'_, AppState>, task_id: String) -> Result<bool, String> {
    let project = state.project_of(&task_id).map_err(|e| e.to_string())?;
    match state.fetch_project_if_due(&project, crate::state::FETCH_ON_VIEW_AGE) {
        Ok(fetched) => Ok(fetched),
        Err(e) => {
            log::warn!("auto fetch for project {project}: {e}");
            Ok(false)
        }
    }
}

#[tauri::command]
pub async fn git_pull(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.git_mutate(&task_id, git::pull).map_err(|e| e.to_string())
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
pub async fn run_branches(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<RunBranches, String> {
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

/// What the model picker offers per agent (see `AppState::list_agent_models`).
#[tauri::command]
pub fn list_agent_models(
    state: State<'_, AppState>,
) -> Result<Vec<crate::state::AgentModelInfo>, String> {
    state.list_agent_models().map_err(|e| e.to_string())
}

/// Ask one agent's CLI what models it has (see `AppState::probe_agent_models`).
/// Async because it runs that CLI: the listing commands take about a second to
/// answer, and a sync command would spend that on the main thread with a menu
/// open in front of it.
///
/// `project_id` is the project the picker is open in, which decides where the
/// listing command runs for a CLI that reads config from its directory.
#[tauri::command]
pub async fn probe_agent_models(
    state: State<'_, AppState>,
    agent: String,
    project_id: Option<String>,
) -> Result<Vec<String>, String> {
    state.probe_agent_models(&agent, project_id.as_deref()).map_err(|e| e.to_string())
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
pub async fn save_settings(
    state: State<'_, AppState>,
    settings: ProviderSettings,
) -> Result<(), String> {
    state.save_settings(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn merge_preview(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<MergePreview, String> {
    state.merge_preview(&task_id).map_err(|e| e.to_string())
}

// async + progress: `git merge` runs in the project's shared primary checkout —
// a checkout of the base branch and then the merge itself, seconds each on a
// large repo — and on the main thread the window froze for all of it behind a
// modal that could only say "Merging…". The three merge commands hold that
// project's `repo_gates` entry, which is the mutual exclusion the main thread
// used to provide (see the note at the top of this file), and it is shared with
// every mutating `git_*` command pointed at the same checkout.
#[tauri::command]
pub async fn merge_task(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<MergeOutcome, String> {
    let outcome = state
        .merge_task_with_progress(&task_id, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())?;
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

// async: read-only, and polled every 3s while a conflict is open — exactly the
// shape the note at the top of this file says must stay off the main thread.
#[tauri::command]
pub async fn merge_status(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<agency_core::merge::MergeState, String> {
    state.merge_status(&task_id).map_err(|e| e.to_string())
}

// async + progress, under the same gate as `merge_task`: committing a resolved
// merge and putting the checkout back on its old branch both rewrite the
// working tree.
#[tauri::command]
pub async fn finish_merge_task(
    state: State<'_, AppState>,
    task_id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<MergeOutcome, String> {
    state
        .finish_merge_task_with_progress(&task_id, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

// async + progress, under the same gate: undoing a merge rewrites the working
// tree back, which is as slow as making it.
#[tauri::command]
pub async fn abort_merge_task(
    state: State<'_, AppState>,
    task_id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<(), String> {
    state
        .abort_merge_task_with_progress(&task_id, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
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

/// True if the text is in the agent's session already, false if it is queued
/// behind the turn the agent is in the middle of (see `crate::sendq`).
#[tauri::command]
pub async fn send_check_feedback(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<bool, String> {
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
pub async fn gh_current_login(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<String, String> {
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
    state.reply_pr_comment(&project_id, number, in_reply_to, &body).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn edit_pr(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
    title: Option<String>,
    body: Option<String>,
) -> Result<(), String> {
    state.edit_pr(&project_id, number, title.as_deref(), body.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn edit_pr_comment(
    state: State<'_, AppState>,
    project_id: String,
    comment_id: u64,
    body: String,
) -> Result<(), String> {
    state.edit_pr_comment(&project_id, comment_id, &body).map_err(|e| e.to_string())
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
        .create_pr_from_branch(
            &project_id,
            &head,
            base.as_deref(),
            title.as_deref(),
            body.as_deref(),
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_race(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    attempts: Vec<RaceAttempt>,
    base: String,
    merge_target: Option<String>,
) -> Result<Vec<RunInfo>, String> {
    let attempts = checked_attempts(attempts)?;
    state
        .create_race(&project_id, &prompt, &attempts, &base, merge_target.as_deref())
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
    model: Option<String>,
) -> Result<RunInfo, String> {
    let model = checked_model(model)?;
    state
        .create_run_from_issue(&project_id, number, &agent, model.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_run_from_pr(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
    agent: String,
    model: Option<String>,
) -> Result<RunInfo, String> {
    let model = checked_model(model)?;
    state
        .create_run_from_pr(&project_id, number, &agent, model.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_pr_review_run(
    state: State<'_, AppState>,
    project_id: String,
    number: u64,
    agent: String,
    model: Option<String>,
    post_comments: bool,
) -> Result<crate::state::PrReviewRun, String> {
    let model = checked_model(model)?;
    state
        .create_pr_review_run(&project_id, number, &agent, model.as_deref(), post_comments)
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

// Comments live in the issue file and have more than one writer, so they are
// their own three calls rather than a field of `update_issue`: each one reads
// the file, applies just its change, and returns the issue as it now stands.
// `created_at` is the comment's id within its issue.

#[tauri::command]
pub fn add_issue_comment(
    state: State<'_, AppState>,
    issue_id: String,
    body: String,
) -> Result<Issue, String> {
    state.add_issue_comment(&issue_id, &body).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_issue_comment(
    state: State<'_, AppState>,
    issue_id: String,
    created_at: i64,
    body: String,
) -> Result<Issue, String> {
    state.update_issue_comment(&issue_id, created_at, &body).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_issue_comment(
    state: State<'_, AppState>,
    issue_id: String,
    created_at: i64,
) -> Result<Issue, String> {
    state.delete_issue_comment(&issue_id, created_at).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_issue_run(
    state: State<'_, AppState>,
    issue_id: String,
    agent: String,
    model: Option<String>,
    base: Option<String>,
    merge_target: Option<String>,
) -> Result<RunInfo, String> {
    let model = checked_model(model)?;
    state
        .start_issue_run(
            &issue_id,
            &agent,
            model.as_deref(),
            base.as_deref(),
            merge_target.as_deref(),
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_issue_race(
    state: State<'_, AppState>,
    issue_id: String,
    attempts: Vec<RaceAttempt>,
    base: Option<String>,
    merge_target: Option<String>,
) -> Result<Vec<RunInfo>, String> {
    let attempts = checked_attempts(attempts)?;
    state
        .start_issue_race(&issue_id, &attempts, base.as_deref(), merge_target.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_issue_loop(
    state: State<'_, AppState>,
    issue_id: String,
    agent: String,
    model: Option<String>,
    check_command: String,
    max_attempts: u32,
    base: Option<String>,
    merge_target: Option<String>,
) -> Result<RunInfo, String> {
    let model = checked_model(model)?;
    state
        .start_issue_loop(
            &issue_id,
            &agent,
            model.as_deref(),
            &check_command,
            max_attempts,
            base.as_deref(),
            merge_target.as_deref(),
        )
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
    state.authenticate_mcp_server(&project_id, &agent, &name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn deauthenticate_mcp_server(
    state: State<'_, AppState>,
    agent: String,
    name: String,
) -> Result<(), String> {
    state.deauthenticate_mcp_server(&agent, &name).map_err(|e| e.to_string())
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

/// Start a knowledge-graph build for a project. Returns as soon as the build is
/// running; progress and failure come back through `get_knowledge_config`.
#[tauri::command]
pub fn build_knowledge_graph(state: State<'_, AppState>, project_id: String) -> Result<(), String> {
    state.build_knowledge_graph(&project_id).map_err(|e| e.to_string())
}

/// Open a terminal that installs the graphify tooling. Returns the run so the
/// UI can jump into it and watch the install.
#[tauri::command]
pub fn install_knowledge_tooling(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<RunInfo, String> {
    state.install_knowledge_tooling(&project_id).map_err(|e| e.to_string())
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
/// own agent session rather than spawning a separate resolver. True if it went
/// in now, false if it is queued behind the agent's current turn.
#[tauri::command]
pub fn send_merge_conflict(state: State<'_, AppState>, task_id: String) -> Result<bool, String> {
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobSideDto {
    pub b64: String,
    pub size: u64,
    pub too_large: bool,
}

/// Before/after bytes of one file, for changes a text diff can't render.
/// A side is null when the file doesn't exist there (added or deleted).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobSidesDto {
    pub mime: String,
    pub old: Option<BlobSideDto>,
    pub new: Option<BlobSideDto>,
}

fn blob_side_dto(side: git::BlobSide) -> BlobSideDto {
    BlobSideDto { b64: STANDARD.encode(&side.bytes), size: side.size, too_large: side.too_large }
}

/// The two sides of a binary file's change, so the diff viewer can show an image
/// before and after instead of "no textual changes". `hash` selects a commit
/// (against its first parent); without one, `staged` picks HEAD-vs-index or
/// index-vs-working-tree, matching `git_parse_diff`.
#[tauri::command]
pub async fn git_blob_sides(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    staged: bool,
    hash: Option<String>,
) -> Result<BlobSidesDto, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    let mode = match hash {
        Some(h) => git::BlobMode::Commit(h),
        None if staged => git::BlobMode::Staged,
        None => git::BlobMode::Unstaged,
    };
    let (old, new) = git::blob_sides(&wt, &path, &mode).map_err(|e| e.to_string())?;
    Ok(BlobSidesDto {
        mime: agency_core::files::mime_for(&path).to_string(),
        old: old.map(blob_side_dto),
        new: new.map(blob_side_dto),
    })
}

#[tauri::command]
pub async fn git_stage_hunk(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::stage_hunk(wt, &path, hunk_index))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_unstage_hunk(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::unstage_hunk(wt, &path, hunk_index))
        .map_err(|e| e.to_string())
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
pub async fn inspect_repo(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<ReadinessDto, String> {
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

// Stops the clone started by `clone_repo` for this url/parent pair by killing
// the git it is waiting on; the half-downloaded folder is deleted with it. Sync
// and trivial for the same reason as `cancel_repo_setup`: it only flips a flag,
// and it has to be answered while the clone it cancels is still running.
#[tauri::command]
pub fn cancel_clone(state: State<'_, AppState>, url: String, parent_dir: String) {
    state.cancel_clone(&url, std::path::Path::new(&parent_dir));
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
pub async fn git_commit_amend(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::commit_amend(wt, &message))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_checkout_branch(
    state: State<'_, AppState>,
    task_id: String,
    name: String,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::checkout_branch(wt, &name))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_create_branch(
    state: State<'_, AppState>,
    task_id: String,
    name: String,
    from: Option<String>,
    checkout: bool,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| {
            agency_core::git::create_branch(wt, &name, from.as_deref(), checkout)
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_delete_branch(
    state: State<'_, AppState>,
    task_id: String,
    name: String,
    force: bool,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::delete_branch(wt, &name, force))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_list_branches(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<agency_core::git::ProjectBranches, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::list_branches(&wt).map_err(|e| e.to_string())
}

// async: round-trips the network, must never block the main thread. Gated like
// `git_pull` (and unlike `git_push_force`): a rebase rewrites the working tree.
#[tauri::command]
pub async fn git_pull_rebase(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.git_mutate(&task_id, agency_core::git::pull_rebase).map_err(|e| e.to_string())
}

// async and streamed, like `git_push`: a force push after a rebase re-uploads
// the whole branch, so it is the same long upload and reports on the same
// channel. `cancel_push` stops it too — it is registered under the same
// worktree key.
#[tauri::command]
pub async fn git_push_force(
    state: State<'_, AppState>,
    task_id: String,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<(), String> {
    state
        .push_force_run(&task_id, move |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_undo_last_commit(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<String, String> {
    state.git_mutate(&task_id, agency_core::git::undo_last_commit).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_reset_to(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
    mode: String,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::reset_to(wt, &hash, &mode))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_revert_commit(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::revert_commit(wt, &hash))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_cherry_pick(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::cherry_pick(wt, &hash))
        .map_err(|e| e.to_string())
}

// Read-only, so no gate and no thread of its own to fight for: it only lists
// what `stash push` left behind.
#[tauri::command]
pub async fn git_stash_list(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<agency_core::git::StashEntry>, String> {
    let wt = state.git_root(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stash_list(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_stash_push(
    state: State<'_, AppState>,
    task_id: String,
    message: Option<String>,
    include_untracked: bool,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| {
            agency_core::git::stash_push(wt, message.as_deref(), include_untracked)
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_stash_apply(
    state: State<'_, AppState>,
    task_id: String,
    index: usize,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::stash_apply(wt, index))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_stash_pop(
    state: State<'_, AppState>,
    task_id: String,
    index: usize,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::stash_pop(wt, index))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_stash_drop(
    state: State<'_, AppState>,
    task_id: String,
    index: usize,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::stash_drop(wt, index))
        .map_err(|e| e.to_string())
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
    ignore_paths: Vec<String>,
    on_progress: Channel<agency_core::setup::CloneProgress>,
) -> Result<(), String> {
    state
        .commit_repo(std::path::Path::new(&repo_path), add_gitignore, ignore_paths, move |p| {
            let _ = on_progress.send(p);
        })
        .map_err(|e| e.to_string())
}

// Stops the staging pass started by `commit_repo` for this folder. Sync and
// trivial on purpose: it only flips a flag, and it has to be answered while the
// command it cancels is still running.
#[tauri::command]
pub fn cancel_repo_setup(state: State<'_, AppState>, repo_path: String) {
    state.cancel_repo_setup(std::path::Path::new(&repo_path));
}

// Looks for files big enough that committing them is probably a mistake — model
// weights, datasets, video. Only ever called for a folder the setup dialog is
// already open over, so it costs nothing on the ordinary path of adding a
// project that is already a repository.
#[tauri::command]
pub async fn scan_large_files(
    repo_path: String,
) -> Result<agency_core::setup::LargeFileScan, String> {
    let path = std::path::PathBuf::from(repo_path);
    tauri::async_runtime::spawn_blocking(move || agency_core::setup::scan_large_files(&path))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_stage_lines(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
    lines: Vec<usize>,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::stage_lines(wt, &path, hunk_index, &lines))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_unstage_lines(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
    lines: Vec<usize>,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::unstage_lines(wt, &path, hunk_index, &lines))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_revert_lines(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
    lines: Vec<usize>,
) -> Result<(), String> {
    state
        .git_mutate(&task_id, |wt| agency_core::git::revert_lines(wt, &path, hunk_index, &lines))
        .map_err(|e| e.to_string())
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

/// Every run whose preview MCP server is up, for the app-level keeper that
/// hosts a hidden preview whenever the Run tab's own pane is not on screen —
/// without one, the dispatched agent's preview tools would only work while
/// the user happens to be looking at the Run tab (AGE-143).
#[tauri::command]
pub async fn preview_targets(
    state: State<'_, AppState>,
) -> Result<Vec<crate::state::PreviewTargetDto>, String> {
    Ok(state.preview_targets())
}

/// The Run tab reporting where its preview pane sits on screen (`None` when it
/// leaves). Feeds the crop of the native preview screenshot.
#[tauri::command]
pub fn set_preview_rect(
    state: State<'_, AppState>,
    run_id: String,
    rect: Option<crate::state::PreviewRect>,
) -> Result<(), String> {
    state.set_preview_rect(&run_id, rect);
    Ok(())
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

/// What archiving or deleting this run would remove. Read before either dialog
/// is shown, so the wording is about this branch rather than about the verb.
///
/// async, like its neighbours: it runs a handful of git probes, and on a repo
/// with a lot of remote refs `branch --remotes --contains` is not instant.
#[tauri::command]
pub async fn run_cleanup(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::state::RunCleanup, String> {
    state.run_cleanup(&id).map_err(|e| e.to_string())
}

/// The archived run's record, as markdown. `None` for a run archived before
/// records existed.
#[tauri::command]
pub async fn read_run_record(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<String>, String> {
    state.read_run_record(&id).map_err(|e| e.to_string())
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
pub fn save_notif_settings(
    state: State<'_, AppState>,
    settings: NotifSettings,
) -> Result<(), String> {
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
    state.add_review_comment(&run_id, &path, line_start, line_end, &body).map_err(|e| e.to_string())
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

/// True if the comments are in the agent's session already, false if they are
/// queued behind the turn it is in the middle of (see `crate::sendq`).
#[tauri::command]
pub fn send_review_comments(state: State<'_, AppState>, run_id: String) -> Result<bool, String> {
    state.send_review_comments(&run_id).map_err(|e| e.to_string())
}

/// What Agency is still holding for this run's sessions and has not typed in
/// yet, oldest first (see `crate::sendq`).
#[tauri::command]
pub fn list_queued_messages(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Vec<crate::state::QueuedMessageInfo>, String> {
    Ok(state.list_queued_messages(&run_id))
}

/// Drop one waiting message. False means it was delivered (or discarded) before
/// the click landed, so there was nothing left to drop.
#[tauri::command]
pub fn cancel_queued_message(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<bool, String> {
    Ok(state.cancel_queued_message(&session_id, &text))
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
    app.opener().reveal_item_in_dir(p).map_err(|e| e.to_string())
}

/// How many candidates one hovered terminal line may ask about. A line holds a
/// handful of path-shaped words at most; the cap keeps a pathological line
/// (minified JSON, a `find` dump wrapped into one logical line) from turning a
/// hover into hundreds of stat calls.
const MAX_LINK_CANDIDATES: usize = 32;

/// Which of `paths` — the path-shaped words on the terminal line under the
/// pointer — actually exist, so only those are underlined as links. Answers
/// positionally: `None` where nothing resolved.
#[tauri::command]
pub async fn resolve_term_paths(
    state: State<'_, AppState>,
    root: FileRoot,
    paths: Vec<String>,
) -> Result<Vec<Option<agency_core::files::LinkedPath>>, String> {
    let base = resolve_root(&state, &root)?;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    Ok(paths
        .iter()
        .take(MAX_LINK_CANDIDATES)
        .map(|p| agency_core::files::resolve_printed_path(&base, p, home.as_deref()))
        .collect())
}

/// Hand a clicked terminal path to the OS: reveal a directory in the file
/// manager, open a file with its default app. Only for what the Files tab
/// can't show — anything inside the root opens in the app instead.
#[tauri::command]
pub fn open_term_path(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    root: FileRoot,
    path: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let base = resolve_root(&state, &root)?;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let target = agency_core::files::resolve_printed_path(&base, &path, home.as_deref())
        .ok_or_else(|| format!("path does not exist: {path}"))?;
    if target.is_dir {
        app.opener().reveal_item_in_dir(&target.abs_path).map_err(|e| e.to_string())
    } else {
        app.opener().open_path(&target.abs_path, None::<&str>).map_err(|e| e.to_string())
    }
}

/// Rename a run: overwrite its display title unconditionally (unlike
/// set_run_title, which only fills an empty title from the first prompt). An
/// empty/whitespace title clears it, so the name falls back to prompt/branch.
#[tauri::command]
pub fn rename_run(state: State<'_, AppState>, id: String, title: String) -> Result<(), String> {
    state.store_run_title(&id, title.trim()).map_err(|e| e.to_string())
}

/// Rename a run's branch, in git and in the registry, and return the name that
/// was actually applied (the `agent/` prefix is kept whether or not the caller
/// typed it). Sync, like the other single-ref git commands: it moves one ref
/// and writes one row, with no checkout to wait on.
#[tauri::command]
pub fn rename_run_branch(
    state: State<'_, AppState>,
    id: String,
    branch: String,
) -> Result<String, String> {
    state.rename_run_branch(&id, &branch).map_err(|e| e.to_string())
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

/// Stat-only corpus pass: `(path, mtime, size)` per markdown file plus every
/// folder and every attachment, so the docs poll can detect change without
/// re-reading bodies and still see a folder that holds no notes yet — and the
/// images and PDFs sitting in it.
#[tauri::command]
pub async fn docs_corpus_stats(
    state: State<'_, AppState>,
    root: FileRoot,
    docs_dir: String,
) -> Result<agency_core::files::DocsScan, String> {
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
/// happens in create_workspace. An existing note is left alone apart from the
/// stale privacy claim, which `guide::repair_opening` rewrites in place. Sync:
/// it writes a file.
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
