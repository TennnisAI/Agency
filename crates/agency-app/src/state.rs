use crate::notifier;
use agency_core::profile::AgentProfile;
use agency_core::registry::{IssueStatus, Project, Registry};
use agency_core::term::client::{Subscription, TermClient};
use agency_core::term::SessionStatus;
use agency_core::worktree::WorktreeManager;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};
use uuid;

const SETTING_LM_STUDIO_URL: &str = "lm_studio_base_url";
const DEFAULT_LM_STUDIO_URL: &str = "http://localhost:1234/v1";
// Agent the "New Agent" menu/shortcut spawns. Empty = auto (project's last-used).
const SETTING_DEFAULT_AGENT: &str = "default_agent";
const SETTING_NOTIF: &str = "notification_settings";
const SETTING_MCP: &str = "mcp_servers";
/// Set to "1" once the user finishes agent-profile onboarding (or is migrated).
const SETTING_AGENT_ONBOARDING: &str = "agent_onboarding_completed";
/// "0" disables the passive update check. Unset = enabled (the beta default).
const SETTING_UPDATE_CHECK: &str = "update_check_enabled";
/// "0" makes the add-agent menu default to working in the project checkout
/// instead of cutting a worktree. Unset = worktrees on, the isolated default.
const SETTING_DEFAULT_WORKTREE: &str = "default_worktree";
/// Prefixes a per-agent setting holding the model last launched with. The
/// value is the model id, or the empty string for the agent's own default —
/// which is a choice in its own right, so it has to be distinguishable from
/// never having chosen one.
const SETTING_AGENT_MODEL_PREFIX: &str = "agent_model:";
/// Prefixes a per-agent setting holding recently used model ids, newest first,
/// newline-joined. It is what makes a model with no published alias (every
/// Codex and Cursor id) a one-click choice the second time.
const SETTING_AGENT_MODELS_PREFIX: &str = "agent_models:";
/// How many recent model ids to keep per agent. Enough to cover switching
/// between a few models; short enough that the picker stays a menu.
const MODEL_MRU_MAX: usize = 6;

/// How long a failing origin is left alone before the next automatic fetch,
/// doubling per consecutive failure up to [`FETCH_BACKOFF_MAX`]. Also the
/// starting point after any success.
const FETCH_BACKOFF_BASE: Duration = Duration::from_secs(300);
const FETCH_BACKOFF_MAX: Duration = Duration::from_secs(1800);
/// Ceiling on how stale remote-tracking refs can get while the app is open:
/// the background sweep fetches any project not contacted this recently.
pub const FETCH_SWEEP_AGE: Duration = Duration::from_secs(300);
/// Freshness the Source Control panel asks for when it opens or the window is
/// focused. Short, because those are the moments the user is about to *read*
/// ahead/behind; long enough that clicking between agents in one project
/// doesn't fetch per click.
pub const FETCH_ON_VIEW_AGE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    pub lm_studio_base_url: String,
    /// Agent id the "New Agent" menu/shortcut spawns. `None` = auto (fall back
    /// to the project's last-used agent).
    pub default_agent: Option<String>,
    /// Whether the add-agent menu starts with "Own worktree" ticked. Off means
    /// new agents work in the project checkout unless the user ticks the box
    /// for that spawn. A default only: every menu still offers both.
    pub default_worktree: bool,
}

/// A project's effective knowledge-graph config for the settings UI. Command
/// overrides are `None` when unset (the `*_default` fields show what runs then);
/// the `*_installed` flags report whether that tooling is actually on PATH.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeConfigDto {
    pub graph: bool,
    pub serve_command: Option<String>,
    pub build_command: Option<String>,
    pub serve_default: String,
    pub build_default: String,
    pub serve_installed: bool,
    pub build_installed: bool,
    /// The graph file the serve command reads, and whether it exists yet. Until
    /// a build has produced it there is nothing to serve, so the MCP server is
    /// not injected — the UI says so rather than leaving the feature silent.
    pub graph_path: String,
    pub graph_built: bool,
    /// A build is running right now (kicked off by enabling the graph, by the
    /// Build button, or by a merge). The UI polls while this is true.
    pub building: bool,
    /// Why the last finished build failed, `None` if it succeeded or none ran.
    pub last_build_error: Option<String>,
    /// What to run to get the tooling when `*_installed` is false.
    pub install_command: String,
}

/// Progress of a project's graph build. One entry per repo, kept in memory:
/// after a restart the graph file itself is the source of truth.
#[derive(Debug, Clone, Default)]
struct KgBuild {
    running: bool,
    /// Failure reason from the last finished build; cleared when one succeeds.
    error: Option<String>,
}

/// Files copied into every new worktree. `copy` is the user-configured
/// (per-machine) list; `detected_env` is the auto-detected set of untracked
/// root `.env*` files that are always copied on top, shown so the UI can tell
/// the user what happens without configuration.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesConfigDto {
    pub copy: Vec<String>,
    pub detected_env: Vec<String>,
}

/// Everything the Run tab needs to list a project's run scripts — and to set
/// the first one up when there are none. `workspace` and `port` are the
/// directory the commands run in and the `AGENCY_PORT` they will see, both
/// shown so the user can tell what they're configuring.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunScriptConfigDto {
    pub scripts: Vec<agency_core::config::RunScript>,
    /// The scripts come from the tracked `agency.toml`, so they are shared with
    /// the team. Edits still go to the local override, which shadows them.
    pub shared: bool,
    pub suggestions: Vec<agency_core::runsetup::RunSuggestion>,
    pub workspace: String,
    pub port: Option<u16>,
}

/// One script's live session state, as `run_scripts_status` reports it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunScriptStatusDto {
    pub name: String,
    pub status: SessionStatus,
}

/// The workspace a run script runs in, resolved from the Run tab's target
/// token. `name` is what the command sees as `AGENCY_WORKSPACE_NAME`.
struct RunTarget {
    project_id: String,
    name: String,
    repo: std::path::PathBuf,
    cwd: std::path::PathBuf,
    port: Option<u16>,
}

/// The directory a git command works in, resolved from a Source Control target
/// token, plus the project whose own checkout that directory is (`None` when it
/// is an agent's private worktree). See [`AppState::git_target`].
struct GitTarget {
    path: std::path::PathBuf,
    primary: Option<String>,
}

/// Result of importing an `mcp.json`: the full app-global list after the merge,
/// plus how many servers the file contributed (so the UI can confirm the count).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpImportResult {
    pub servers: Vec<agency_core::mcp::McpServer>,
    pub imported: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub id: String,
    pub project_id: String,
    pub agent: String,
    pub prompt: String,
    pub title: Option<String>,
    pub branch: String,
    pub status: SessionStatus,
    /// Live working/waiting/idle signal from the notifier's pane poll. None
    /// until the first tick observes the run (~2s after spawn or app start);
    /// the UI treats a running agent without it as working.
    pub activity: Option<crate::activity::ActivityInfo>,
    /// Tokens and cost for this run, read from the agent's own transcript.
    /// `None` means we cannot see this agent's spend at all, which is the
    /// case for every agent whose transcript format we have not read. That is
    /// deliberately distinct from a zero: showing "0 tokens" for an agent we
    /// cannot account for would be a false claim, not a missing one.
    pub usage: Option<agency_core::usage::UsageInfo>,
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
    pub port: Option<u16>,
    pub kind: String,
    /// True while at least one of the project's run scripts is still running in
    /// this run's workspace, so the board can show it without anyone opening
    /// the Run tab. Which script it is lives in the Run tab; this is the dot.
    pub run_scripts_live: bool,
    /// False = the run works in the project's main checkout rather than an
    /// isolated worktree, so the UI hides merge/PR/archive-the-worktree.
    pub worktree: bool,
    pub race_id: Option<String>,
    pub loop_config: Option<agency_core::loops::LoopConfig>,
    pub loop_state: Option<agency_core::loops::LoopState>,
    pub issue_id: Option<String>,
    /// Model this run was launched on. None = the agent's own default.
    pub model: Option<String>,
    /// Epoch seconds. Exposed for time views (the weekly note); archived_at is
    /// None for live runs and last-archive-wins after a restore cycle.
    pub created_at: i64,
    pub archived_at: Option<i64>,
}

/// What one agent's model picker offers, as sent to the UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelInfo {
    pub agent: String,
    /// False = this CLI takes no model flag, so the picker is not offered for
    /// it at all and its runs use whatever it is configured to use.
    pub supported: bool,
    /// Stable vendor aliases worth one click. Often empty: an agent whose
    /// model names are dated offers none rather than offering stale ones.
    pub suggested: Vec<String>,
    /// Model ids used before with this agent, newest first.
    pub recent: Vec<String>,
    /// The model chosen last time. None = the agent's own default.
    pub selected: Option<String>,
    /// The agent's own command for listing its models, shown as a hint.
    pub list_command: Option<String>,
}

/// An extra agent tab sharing a run's worktree, as shown in the UI. `id` is
/// the composite session key (`<run_id>--<n>`) accepted by every run-terminal
/// command (attach/input/resize/preview), so the UI drives these tabs through
/// the exact same plumbing as the primary agent terminal.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSessionInfo {
    pub id: String,
    pub run_id: String,
    pub agent: String,
    pub status: SessionStatus,
}

/// Where an agent PR review landed. `session_id` is set when the review had to
/// run as an extra tab inside an existing run (the PR's branch was already
/// checked out there); the UI focuses that tab instead of the run's primary
/// agent. None means the review got a workspace of its own.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewRun {
    pub run: RunInfo,
    pub session_id: Option<String>,
}

/// Everything create_run_spec needs to make a workspace + session. The public
/// entry points (plain create, racing, from-issue, from-PR) differ only in
/// which fields they fill.
struct NewRunSpec<'a> {
    project_id: &'a str,
    prompt: &'a str,
    agent: &'a str,
    base: &'a str,
    merge_target: Option<&'a str>,
    race_id: Option<String>,
    title: Option<String>,
    /// Check out this existing branch instead of cutting `agent/<id>` off base.
    existing_branch: Option<String>,
    /// Present = create a looping run: spawn the agent headless (profile
    /// loop_args) and let the loop driver re-run it until checks pass.
    loop_config: Option<agency_core::loops::LoopConfig>,
    /// False = skip the worktree entirely and run the agent in the project's
    /// main checkout, on whatever branch is already there. Only the plain
    /// single-agent flow offers this; races and loops always want isolation.
    worktree: bool,
    /// Local issue this run is dispatched from (see start_issue_*): stored on
    /// the run so merge/PR/discard can drive the issue's status.
    issue_id: Option<String>,
    /// Model to launch this agent on, or None for the agent's own default.
    /// Validated by the command layer (`sanitize_model`) before it gets here.
    model: Option<&'a str>,
}

/// What an "Approve & merge" would do, computed before running it so the UI can
/// explain the outcome instead of silently merging. `commits_ahead == 0` means
/// the branch has no new commits (merge is a no-op); `commits_behind > 0` means
/// the base moved on since the branch was created (stale branch — merge may
/// conflict); `worktree_dirty` flags uncommitted agent work that a branch merge
/// would leave behind.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePreview {
    pub base: String,
    pub branch: String,
    pub commits_ahead: usize,
    pub commits_behind: usize,
    pub worktree_dirty: bool,
    pub dirty_files: Vec<String>,
}

/// Outcome of a bulk discard of a project's archived runs. Reported per-run
/// rather than as one pass/fail so the UI can say how much actually went and
/// still surface what didn't (see discard_archived_runs).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardSummary {
    pub discarded: usize,
    /// One message per run that couldn't be discarded; empty on a clean sweep.
    pub failed: Vec<String>,
}

/// A run's PR plus check rollup, polled by the merge modal.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    pub pr: Option<agency_core::gh::PrInfo>,
    pub checks: Vec<agency_core::gh::CheckItem>,
}

/// Compose a single-line review-feedback message for the agent. Single-line so
/// TUI agents don't submit early on embedded newlines.
fn compose_feedback(comments: &[agency_core::registry::ReviewComment]) -> String {
    let parts: Vec<String> = comments
        .iter()
        .map(|c| {
            let loc = if c.line_end != c.line_start {
                format!("{}:{}-{}", c.path, c.line_start, c.line_end)
            } else {
                format!("{}:{}", c.path, c.line_start)
            };
            let body = c.body.replace(['\n', '\r'], " ");
            format!("[{}] {}", loc, body)
        })
        .collect();
    format!("Please address these review comments: {}", parts.join(" | "))
}

/// How many conflicted paths the prompt names before it summarizes the rest.
/// The message is typed into a live agent session, so it has to stay readable;
/// an agent that has the first 40 has plenty to start on and can run `git
/// status` itself for the tail.
const MERGE_CONFLICT_FILE_CAP: usize = 40;

/// The unmerged paths of a conflicted merge, in `git status --short` form
/// (`UU src/a.rs`). Only the unmerged ones: a merge in progress also stages
/// every cleanly merged file, and listing those would bury the conflicts.
fn conflict_status(repo: &Path) -> Vec<String> {
    agency_core::git::status(repo)
        .unwrap_or_default()
        .into_iter()
        // The seven unmerged porcelain states: DD, AU, UD, UA, DU, AA, UU.
        .filter(|c| {
            c.index == "U"
                || c.worktree == "U"
                || (c.index == "D" && c.worktree == "D")
                || (c.index == "A" && c.worktree == "A")
        })
        .map(|c| {
            let path = c.path.replace(['\n', '\r'], " ");
            format!("{}{} {}", c.index, c.worktree, path)
        })
        .collect()
}

/// The prompt typed into an agent's session when its merge conflicts. Single
/// line by construction: `send_text` terminates with a carriage return, so an
/// embedded newline would submit half a message. The checkout path is spelled
/// out because the merge is in progress in the project's main checkout, not in
/// the agent's worktree; an agent that assumed its own cwd would "resolve"
/// files the merge never touched.
fn compose_merge_conflict(repo: &Path, branch: &str, base: &str, status: &[String]) -> String {
    let git_output = if status.is_empty() {
        "(git reports no unmerged files; check `git status` yourself)".to_string()
    } else if status.len() > MERGE_CONFLICT_FILE_CAP {
        format!(
            "{} and {} more (run `git status` for the full list)",
            status[..MERGE_CONFLICT_FILE_CAP].join(" | "),
            status.len() - MERGE_CONFLICT_FILE_CAP,
        )
    } else {
        status.join(" | ")
    };
    format!(
        "Help me fix this merge conflict: {git_output}. That is `git status --short` in {repo}, \
         the project's main checkout, where a merge of {branch} into {base} is in progress. It is \
         not this worktree, so point git at that path. Please resolve every conflict, remove all \
         conflict markers, and `git add` each file you fix. Do not commit; Agency completes the \
         merge once nothing is left unmerged.",
        repo = repo.display(),
    )
}

/// Socket the terminal daemon listens on, derived from the app data dir.
fn termd_socket(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("termd.sock")
}

/// Path to the terminal daemon binary. In a bundled/release build it ships next
/// to the app executable (Tauri externalBin sidecar). Under `cargo test` the
/// current exe is a test binary in `target/<profile>/deps/`, while the daemon is
/// built one level up in `target/<profile>/`, so we also probe the parent dir.
/// Falls back to a bare name for `$PATH` resolution.
fn termd_bin() -> std::path::PathBuf {
    let exe = std::env::current_exe().ok();
    let dir = exe.as_ref().and_then(|p| p.parent());
    // Sibling of the executable (bundled app / `cargo run`).
    let sibling = dir.map(|d| d.join("agency-termd"));
    if let Some(p) = sibling.as_ref() {
        if p.exists() {
            return p.clone();
        }
    }
    // One level up (cargo's `deps/` test/bench layout points here).
    if let Some(p) = dir.and_then(|d| d.parent()).map(|d| d.join("agency-termd")) {
        if p.exists() {
            return p;
        }
    }
    sibling.unwrap_or_else(|| std::path::PathBuf::from("agency-termd"))
}

fn session_name(id: &str) -> String {
    format!("agency-{id}")
}

/// Split a session id into (run_id, tab_seq). Extra agent sessions sharing a
/// run's worktree are keyed `<run_id>--<n>`; a task id can never contain `--`
/// (slugify collapses every separator run into a single hyphen), so the first
/// `--` is unambiguous. Plain run ids return (id, None).
fn split_session_id(id: &str) -> (&str, Option<u32>) {
    match id.split_once("--") {
        Some((run, seq)) => match seq.parse::<u32>() {
            Ok(n) => (run, Some(n)),
            Err(_) => (id, None),
        },
        None => (id, None),
    }
}

/// Agency's own additions to an agent's argv: the flags that make it read the
/// MCP config Agency emitted into `worktree`. Kept apart from the profile's
/// user-edited args (`so_far`, the argv built for this launch) so the two can't
/// clobber each other, and dropped entirely when `so_far` already carries the
/// same flag. Empty for every agent that finds the emitted file unaided — today
/// only Copilot needs telling, because its workspace config is gated behind a
/// folder-trust prompt that every fresh worktree re-triggers.
///
/// Keyed on the profile *name* (the agent id, e.g. "copilot"), not the command:
/// that is what MCP emission is keyed on too, so a custom profile named after a
/// known agent gets both halves or neither.
fn mcp_launch_args(profile: &AgentProfile, worktree: &Path, so_far: &[String]) -> Vec<String> {
    agency_core::mcp::launch_args_unless_set(&profile.name, worktree, so_far)
}

/// A copy of `profile` whose every launch recipe pins `model`.
///
/// The model flag is folded into the profile once, here, rather than threaded
/// through `fresh_agent_argv`/`agent_argv`/`loop_argv` separately — a run that
/// started on one model and resumed, reran or looped on another would be a
/// silent, expensive lie, and one join point is what makes that impossible.
/// It lands at the end of each recipe so it never comes between a flag and the
/// value the user wrote next to it, and so the loop path keeps `{{prompt}}`
/// where the recipe put it.
fn with_model(profile: &AgentProfile, model: Option<&str>) -> AgentProfile {
    let extra = crate::agent_catalog::model_args(&profile.name, model);
    if extra.is_empty() {
        return profile.clone();
    }
    let append = |recipe: &Option<Vec<String>>| {
        recipe.as_ref().map(|r| r.iter().cloned().chain(extra.iter().cloned()).collect())
    };
    AgentProfile {
        args: profile.args.iter().cloned().chain(extra.iter().cloned()).collect(),
        resume_args: append(&profile.resume_args),
        loop_args: append(&profile.loop_args),
        ..profile.clone()
    }
}

/// Put `model` at the front of a newline-joined most-recently-used list,
/// dropping any earlier occurrence and anything past the cap. Pure so the
/// list's behaviour is testable without a database.
fn push_mru(list: &str, model: &str) -> String {
    let mut out = vec![model.to_string()];
    out.extend(
        list.lines().map(str::trim).filter(|l| !l.is_empty() && *l != model).map(String::from),
    );
    out.truncate(MODEL_MRU_MAX);
    out.join("\n")
}

/// Record the model an agent was just launched on, so the picker reopens on it.
fn remember_model(reg: &Registry, agent: &str, model: Option<&str>) -> Result<()> {
    reg.set_setting(&format!("{SETTING_AGENT_MODEL_PREFIX}{agent}"), model.unwrap_or(""))?;
    if let Some(model) = model {
        let key = format!("{SETTING_AGENT_MODELS_PREFIX}{agent}");
        let recent = reg.get_setting(&key)?.unwrap_or_default();
        reg.set_setting(&key, &push_mru(&recent, model))?;
    }
    Ok(())
}

/// Decide the (command, args) to launch for an agent run. With `use_resume` and a
/// resume recipe present, launch the resume args (no prompt). Otherwise launch a
/// fresh session from the rendered prompt. The optional setup script wraps the
/// command in both cases (same as create_run/rerun).
fn agent_argv(
    profile: &AgentProfile,
    worktree: &Path,
    prompt: &str,
    use_resume: bool,
    setup: Option<&str>,
) -> (String, Vec<String>) {
    let mut base_args: Vec<String> = match (use_resume, &profile.resume_args) {
        (true, Some(resume)) => resume.clone(),
        _ => profile.render_args(prompt).into_iter().filter(|a| !a.is_empty()).collect(),
    };
    let mcp = mcp_launch_args(profile, worktree, &base_args);
    base_args.extend(mcp);
    agency_core::scripts::wrap_setup(setup, &profile.command, &base_args)
}

/// How `prompt` is handed to `agent` on a fresh launch: the arguments to append,
/// per that CLI's own recipe (see [`crate::agent_catalog::PromptDelivery`]).
/// Empty when the CLI can't be given an opening prompt at all — the session
/// still comes up in the worktree, and the prompt stays on the run for the user
/// to hand over in the live terminal, which beats an argv the CLI rejects.
fn prompt_args(agent: &str, prompt: &str) -> Vec<String> {
    match crate::agent_catalog::prompt_delivery(agent) {
        crate::agent_catalog::PromptDelivery::Positional => vec![prompt.to_string()],
        crate::agent_catalog::PromptDelivery::Args(recipe) => {
            recipe.iter().map(|a| a.replace("{{prompt}}", prompt)).collect()
        }
        crate::agent_catalog::PromptDelivery::Unsupported => {
            log::warn!(
                "{agent} takes no opening prompt on the command line; starting it promptless \
                 (paste the prompt into its terminal)"
            );
            Vec::new()
        }
    }
}

/// The (command, args) for a fresh agent session that should open with `prompt`
/// already delivered — positionally, behind a flag, or not at all, depending on
/// the agent (see [`prompt_args`]) — unless the profile places it itself with a
/// `{{prompt}}` token. An empty prompt yields the plain promptless argv, so this
/// is a drop-in for [`agent_argv`] on the fresh (non-resume) path.
fn fresh_agent_argv(
    profile: &AgentProfile,
    worktree: &Path,
    prompt: &str,
    setup: Option<&str>,
) -> (String, Vec<String>) {
    let mut args: Vec<String> =
        profile.render_args(prompt).into_iter().filter(|a| !a.is_empty()).collect();
    // Ahead of the prompt below: a flag after a positional argument is the shape
    // most CLIs are least happy with.
    let mcp = mcp_launch_args(profile, worktree, &args);
    args.extend(mcp);
    if !prompt.trim().is_empty() && !profile.args.iter().any(|a| a.contains("{{prompt}}")) {
        args.extend(prompt_args(&profile.name, prompt));
    }
    agency_core::scripts::wrap_setup(setup, &profile.command, &args)
}

/// The prompt an agent opens with when a local issue is dispatched to it: the
/// issue itself, then the least the agent needs to know about the tracker.
/// Pure so the wording is testable without a repo or a live agent.
///
/// Issue files are not tracked by git, so they exist only in the project's own
/// checkout: a run working in `.agency/worktrees/<id>/` has no copy of its own.
/// Every path here is therefore absolute, pointing at the one shared tracker.
///
/// The closing paragraph is deliberately spare. Measured over ~170 real
/// dispatched sessions, the older wording ("this issue is the file X",
/// frontmatter schema, follow-up numbering, README pointer) sent 36% of agents
/// to Read a file whose entire body was already in the prompt — in every case
/// within the first three tool calls — and 31% to list or grep the issues
/// folder that early too. Naming a file invites opening it, so the text now
/// says outright that the issue is already here, and holds back the tracker's
/// conventions for the one case that needs them: filing a follow-up.
fn issue_prompt(
    label: &str,
    title: &str,
    body: &str,
    comments: &[agency_core::registry::IssueComment],
    root: &Path,
) -> String {
    let mut prompt = if body.trim().is_empty() {
        format!("Work on issue {label}: {title}")
    } else {
        format!("Work on issue {label}: {title}\n\n{body}")
    };
    // The thread is part of the ask: it is where the issue gets corrected,
    // narrowed, or argued with after it was filed. Same reasoning as the body
    // — it travels in the prompt so nobody has to go and open the file.
    if !comments.is_empty() {
        prompt.push_str(&format!("\n\nComments on {label}, oldest first:\n"));
        for c in comments {
            prompt.push_str(&format!(
                "\n{} ({}):\n{}\n",
                c.author,
                agency_core::issuefs::epoch_to_rfc3339(c.created_at),
                c.body.trim(),
            ));
        }
    }
    // Attachments are relative links in the body (`assets/…`), which reads
    // as a dead path unless the agent is told what they resolve to.
    let attached = agency_core::issuefs::body_attachments(body);
    if !attached.is_empty() {
        let list = attached
            .iter()
            .map(|p| format!("`{}`", root.join(p).display()))
            .collect::<Vec<_>>()
            .join(", ");
        prompt.push_str(&format!(
            "\n\nThe `assets/…` links in this issue are files attached to it, \
             stored at: {list}. Read them for context (images included)."
        ));
    }
    let issues_dir = root.join(agency_core::issuefs::ISSUES_DIR);
    prompt.push_str(&format!(
        "\n\nThat is the whole of {label}, so there is nothing to go and read. The issue \
         is the file `{issue_file}`, which lives in this project's Agency tracker outside \
         your worktree and is untracked by git: edit it in place if you have something to \
         change there, and the change takes effect at once, with no commit or merge \
         involved. Merging this run marks {label} done automatically, so you do not have \
         to. To file a follow-up issue, read `{readme}` first; that is the only reason to \
         open that folder.",
        issue_file = issues_dir.join(format!("{label}.md")).display(),
        readme = issues_dir.join("README.md").display(),
    ));
    prompt
}

/// The prompt an agent PR review opens with. Pure so the wording is testable
/// without a repo or a live agent. `post_comments` decides whether the agent
/// publishes its findings to the PR on GitHub or only reports them in its own
/// terminal; either way it is told to stay available for follow-up fixes.
fn pr_review_prompt(
    number: u64,
    title: &str,
    url: &str,
    base: &str,
    post_comments: bool,
) -> String {
    let mut p = format!(
        "Review GitHub pull request #{number}: {title}\n\n\
         The PR's head branch is checked out in this workspace. Read the change with \
         `gh pr diff {number}`, or `git diff {base}...HEAD`, and review it for correctness, \
         bugs, security problems, missing tests, and anything else that should block the \
         merge. Read the surrounding code too, not just the diff.\n\n"
    );
    if post_comments {
        p.push_str(&format!(
            "When you are done, publish the review to GitHub with the `gh` CLI, posting the \
             summary and every inline comment as a single review:\n\n\
             SLUG=$(gh repo view --json nameWithOwner -q .nameWithOwner)\n\
             gh api --method POST \"repos/$SLUG/pulls/{number}/reviews\" --input - <<'JSON'\n"
        ));
        // Not a format string: the JSON braces are literal.
        p.push_str(
            "{\"event\": \"COMMENT\", \"body\": \"<summary>\", \"comments\": [{\"path\": \"<file>\", \"line\": <line>, \"side\": \"RIGHT\", \"body\": \"<comment>\"}]}\n\
             JSON\n\n\
             Every comment's path and line must still be part of the diff. `side` is \"RIGHT\" \
             for added and unchanged lines, \"LEFT\" for deleted ones. Use the COMMENT event: \
             GitHub rejects approving or requesting changes on a PR you opened yourself.\n\n",
        );
    } else {
        p.push_str(
            "Report your findings here in the terminal. Do not post anything to GitHub \
             unless I ask you to.\n\n",
        );
    }
    p.push_str(&format!(
        "Then stay available: I may ask you to fix what you found. Commit fixes to this \
         branch and push to update the PR.\n\nPR link: {url}\n"
    ));
    p
}

/// The (command, args) for one headless loop attempt: the profile's loop
/// recipe with `{{prompt}}` filled in, wrapped by the optional setup script.
/// Errors when the profile has no loop recipe — such agents can't loop.
fn loop_argv(
    profile: &AgentProfile,
    worktree: &Path,
    prompt: &str,
    setup: Option<&str>,
) -> Result<(String, Vec<String>)> {
    let recipe = profile.loop_args.as_ref().filter(|r| !r.is_empty()).ok_or_else(|| {
        anyhow!("agent '{}' has no loop recipe — set the profile's loop args first", profile.name)
    })?;
    let mut args: Vec<String> = recipe.iter().map(|a| a.replace("{{prompt}}", prompt)).collect();
    // Ahead of wherever the prompt lands, for the same reason as the
    // interactive path: keep Agency's flags out from behind a positional.
    let mcp = mcp_launch_args(profile, worktree, &args);
    match recipe.iter().position(|a| a.contains("{{prompt}}")) {
        Some(i) => {
            args.splice(i..i, mcp);
        }
        None => args.extend(mcp),
    }
    // A recipe without a {{prompt}} token still gets the prompt, as the final
    // positional arg — mirroring the interactive path. Attempts must never
    // silently launch promptless: the loop would burn its whole budget on
    // no-op runs with nothing surfaced.
    if !recipe.iter().any(|a| a.contains("{{prompt}}")) {
        args.push(prompt.to_string());
    }
    Ok(agency_core::scripts::wrap_setup(setup, &profile.command, &args))
}

/// True when the run is a loop that is still driving (non-terminal state) —
/// its agent session belongs to the loop driver, so interactive spawn paths
/// (attach-resume, rerun, generic stop) must stand aside or end the loop.
fn has_active_loop(run: &agency_core::registry::Run) -> bool {
    run.loop_config.is_some()
        && run.loop_state.as_ref().map(|s| !s.status.is_terminal()).unwrap_or(false)
}

/// Announce a teardown step on the same channel shape clone/push/spawn report
/// through, so the UI can render all of them with one progress readout. No
/// percent: git says nothing about how far through removing a worktree it is,
/// and a determinate bar frozen at 50% reads worse than an honest sweep.
fn step(on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress), phase: &str, detail: &str) {
    on_progress(agency_core::setup::CloneProgress {
        phase: phase.to_string(),
        percent: None,
        detail: detail.to_string(),
    });
}

/// What a merge, finish or abort says when the project's checkout gate is
/// already held. Every one of them fails fast rather than queueing: whatever
/// has the gate is rewriting the very checkout they were about to inspect, so
/// what they read before waiting is stale by the time their turn comes.
fn busy_checkout_error() -> &'static str {
    "the project's checkout is busy: another merge or git command is running there. Try again once it finishes."
}

/// Detail line for a teardown that walks a list of runs: which one of how many
/// it is on, and what that one is called. Position first — over a sweep the
/// phases repeat, and without the count they read as one teardown restarting.
fn sweep_detail(label: &str, i: usize, total: usize) -> String {
    format!("{} of {total}: {label}", i + 1)
}

/// What to call a run in the UI: its title once one has been derived, its
/// branch until then.
fn run_label(run: &agency_core::registry::Run) -> String {
    run.title.clone().unwrap_or_else(|| run.branch.clone())
}

/// Daemon session for one run script. Keyed by (workspace, script name), so a
/// project can have `dev` serving while `build` runs, and the same script runs
/// independently in every agent's workspace. `target` is a run id or the
/// `project:<id>` token for the project's own checkout — neither can contain
/// `#`, so the split back out is unambiguous.
fn run_session_name(target: &str, script: &str) -> String {
    format!("agency-run-{target}#{script}")
}

/// Every run-script session belonging to `target`. The bare `agency-run-<id>`
/// form is the pre-AGE-34 name, from before scripts were named: it is matched
/// too so an app update doesn't strand a running dev server no UI can reach.
/// The `#` separator keeps `agency-run-fix-a1#dev` out of `fix`'s sessions.
fn run_session_names_for(target: &str, live: &[String]) -> Vec<String> {
    let legacy = format!("agency-run-{target}");
    let prefix = format!("{legacy}#");
    live.iter().filter(|n| **n == legacy || n.starts_with(&prefix)).cloned().collect()
}

/// Live status of every run script started in `target`'s workspace, keyed by
/// script name, read out of one daemon session listing. A script that was never
/// started has no session and so no entry — which is exactly what the crash
/// detector wants, since it only watches for a *running* script exiting.
fn run_script_statuses_from(
    target: &str,
    live: &[(String, SessionStatus)],
) -> std::collections::BTreeMap<String, SessionStatus> {
    let legacy = format!("agency-run-{target}");
    let prefix = format!("{legacy}#");
    live.iter()
        .filter_map(|(name, status)| {
            if let Some(script) = name.strip_prefix(&prefix) {
                Some((script.to_string(), status.clone()))
            } else if *name == legacy {
                Some((agency_core::config::DEFAULT_RUN_NAME.to_string(), status.clone()))
            } else {
                None
            }
        })
        .collect()
}

/// Whether any run script is still running in `target`'s workspace — the board's
/// "something is live here" dot. Reads a session listing the caller already has,
/// so a whole project's worth of runs costs one daemon round-trip rather than a
/// listing per run (the reason AGE-34 left this off the board).
fn any_run_script_live(target: &str, live: &[(String, SessionStatus)]) -> bool {
    run_script_statuses_from(target, live).values().any(|s| matches!(s, SessionStatus::Running))
}

/// Daemon session for a run's companion shell — an interactive terminal the user
/// can open alongside the agent, sharing the same worktree. Distinct from both
/// the agent session (`agency-<id>`) and the run-script session (`agency-run-<id>`).
fn shell_session_name(id: &str) -> String {
    format!("agency-shell-{id}")
}

/// The agent name reserved for terminal sessions. Not an agent profile — every
/// terminal Agency opens runs `login_shell()` directly (see `SHELL_AGENT` uses),
/// so no row in the profiles table backs it and none of the agent pickers list it.
pub const SHELL_AGENT: &str = "shell";

/// The user's login shell, for every interactive terminal Agency opens. One
/// helper so the fallback can't drift between spawn sites (it previously varied
/// between `/bin/zsh` and `/bin/bash` for the same feature).
fn login_shell() -> String {
    std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".to_string())
}

fn validate_provider_url(raw: &str) -> Result<()> {
    if raw.is_empty() {
        return Ok(());
    }
    let url = url::Url::parse(raw).map_err(|e| anyhow!("invalid URL: {e}"))?;
    match url.scheme() {
        "https" => {}
        "http" => {
            let host = url.host_str().unwrap_or("");
            if host != "localhost" && host != "127.0.0.1" && host != "[::1]" {
                bail!("http is only allowed for localhost; use https for remote hosts");
            }
        }
        other => bail!("unsupported URL scheme: {other}"),
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("URL must not contain embedded credentials");
    }
    Ok(())
}

/// Build a readable, unique task id from the prompt: a slug derived from the
/// prompt text plus a short suffix that disambiguates runs sharing a prompt.
/// The id doubles as the worktree dir, branch (`agent/<id>`) and daemon session
/// name, so it stays restricted to `[a-z0-9-]`, which is safe for all three.
pub fn new_task_id(prompt: &str) -> String {
    format!("{}-{}", slugify(prompt), short_suffix())
}

/// The directory a run's agent, scripts and git commands operate in: its own
/// worktree when it has one, else the project's main checkout.
///
/// Every call site that used to spell `repo/.agency/worktrees/<id>` inline goes
/// through this, so worktree-less runs (terminals, and agents started directly
/// on the checked-out branch) land in the repo root instead of a path that
/// doesn't exist.
fn workspace_dir(repo: &Path, run: &agency_core::registry::Run) -> std::path::PathBuf {
    if run.worktree {
        repo.join(".agency").join("worktrees").join(&run.id)
    } else {
        repo.to_path_buf()
    }
}

/// Whether a project folder has no git repository at all. Such a project is
/// still a project — docs, files, terminals and agents working directly in the
/// folder all behave normally — but every branch-shaped flow (worktrees,
/// merges, races, loops) is unavailable in it.
///
/// `None` when git could not be run there, which is not an answer either way.
fn is_gitless(repo: &Path) -> Option<bool> {
    agency_core::setup::inside_work_tree(repo).map(|inside| !inside)
}

/// [`is_gitless`], refusing to guess.
///
/// This answer decides whether an agent gets an isolated worktree or is turned
/// loose in the user's own checkout on their own branch, so reading "git would
/// not run" as "plain folder" would silently do the more destructive thing to a
/// perfectly normal repo — a real risk here, since a Finder-launched bundle has
/// famously ended up without git on `PATH`. Fail loudly instead.
fn require_gitless_known(repo: &Path) -> Result<bool> {
    is_gitless(repo).ok_or_else(|| {
        anyhow!(
            "could not run git in {} to tell whether it is a repository; \
             check that git is installed and the folder is available",
            repo.display()
        )
    })
}

/// Reject a branch-level operation (merge, PR) on a run that has no branch of
/// its own — its commits are already on the checkout's branch, so "landing"
/// them is meaningless and would target whatever the user is working on.
///
/// The UI hides these controls for such runs; this is the backstop that keeps
/// a stale window or a scripted call from acting on the wrong branch.
///
/// `repo` is the project folder, used only to word the refusal: the run's
/// stored branch is empty for every terminal and for runs created before the
/// folder had a repository, so it says nothing about whether one exists now.
fn require_own_branch(run: &agency_core::registry::Run, repo: &Path, action: &str) -> Result<()> {
    if run.worktree {
        return Ok(());
    }
    // Read the checkout live, as `run_info_from` does, so the refusal names the
    // branch the user can actually see rather than a stale or never-set one.
    // Only a folder positively confirmed to have no repository gets the "not a
    // git repository" wording — a detached HEAD or an unrunnable git would make
    // that claim false.
    match agency_core::merge::current_branch(repo) {
        Some(branch) => bail!(
            "this agent works directly in the project checkout on {branch}, so there is \
             nothing to {action}; use Source Control to commit, publish or open a PR from \
             that branch"
        ),
        None if is_gitless(repo) == Some(true) => bail!(
            "this agent works directly in the project folder, which is not a git repository, \
             so there is nothing to {action}"
        ),
        None => bail!(
            "this agent works directly in the project checkout, which is not on a branch, \
             so there is nothing to {action}"
        ),
    }
}

/// Refuse a merge whose branch has gone, with a message that says so.
///
/// The run's branch is recorded when the worktree is cut, but git is the truth
/// and someone can delete or rename that branch from outside Agency. Without
/// this check the first thing to touch it is a `base..branch` range, and the
/// user gets git's raw "ambiguous argument 'main..agent/foo': unknown revision
/// or path not in the working tree", followed by advice about using `--` to
/// separate paths from revisions. That reads as a syntax bug in Agency and
/// says nothing about the branch being gone or what to do next.
///
/// Observed after a history rewrite deleted a merged agent branch while its
/// run was still on the board. Deliberately not applied to `merge_status`,
/// `finish_merge_task_*` or `abort_merge_task_*`: those are the ways out of a
/// merge already in progress, and blocking them would strand the user.
fn require_branch_exists(run: &agency_core::registry::Run, repo: &Path) -> Result<()> {
    if agency_core::merge::branch_exists(repo, &run.branch) {
        return Ok(());
    }
    bail!(
        "this agent's branch {} no longer exists, so there is nothing to merge; it was \
         deleted or renamed outside Agency, and if its work already landed you can archive \
         the agent to clear it",
        run.branch
    )
}

/// Lowercase the prompt, keep ASCII alphanumerics, collapse every other run of
/// characters into a single hyphen, and cap the length so branch names stay
/// short. Falls back to "agent" when the prompt has no usable characters.
fn slugify(prompt: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in prompt.chars() {
        if slug.len() >= 40 {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "agent".to_string()
    } else {
        slug.to_string()
    }
}

/// A short base36 suffix derived from the clock and a process-wide counter, so
/// two runs created from the same prompt (e.g. reruns) get distinct ids.
fn short_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut v = nanos.rotate_left(21) ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut s = [0u8; 4];
    for slot in s.iter_mut() {
        *slot = DIGITS[(v % 36) as usize];
        v /= 36;
    }
    String::from_utf8_lossy(&s).into_owned()
}

/// Lowest free port-block base: the first `base + slot*block_size` (slot = 0,1,2…)
/// not already in `used`. Returns `None` only if the `u16` space overflows first.
fn pick_port(used: &std::collections::HashSet<u16>, base: u16, block_size: u16) -> Option<u16> {
    let block_size = block_size.max(1);
    let mut slot: u16 = 0;
    loop {
        let candidate = base.checked_add(slot.checked_mul(block_size)?)?;
        if !used.contains(&candidate) {
            return Some(candidate);
        }
        slot = slot.checked_add(1)?;
    }
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Issue titles are one H1 line in the file; collapse all whitespace runs
/// (newlines included) to single spaces so a pasted multi-line title can't
/// smear into the body on the next parse.
fn normalize_title(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn now_millis() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Diagnostic logging for the PTY/terminal pipeline, gated behind the
/// `AGENCY_DEBUG_PTY` env var (empty/unset = disabled, so it's a single env read
/// on the hot path otherwise). Set it to `1` to log to
/// `<tmpdir>/agency-pty-debug.log`, or to an absolute path to log there. Used to
/// trace what input reaches a terminal session on navigate-away/back, since the
/// shell exits 0 on a stray EOF that nothing in the backend is known to send.
fn pty_debug(msg: &str) {
    use std::io::Write;
    let val = match std::env::var("AGENCY_DEBUG_PTY") {
        Ok(v) if !v.is_empty() => v,
        _ => return,
    };
    let path = if val.contains('/') {
        std::path::PathBuf::from(val)
    } else {
        std::env::temp_dir().join("agency-pty-debug.log")
    };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {}", now_millis(), msg);
    }
}

/// Render bytes for the debug log: space-separated hex plus a human-readable
/// escaped form, so a lone `\x04` (EOF) or `\r`/`\n` jumps out.
fn fmt_bytes(data: &[u8]) -> String {
    let hex: Vec<String> = data.iter().map(|b| format!("{b:02x}")).collect();
    format!("[{}] {:?}", hex.join(" "), String::from_utf8_lossy(data))
}

/// The run the most recent notification was about. macOS gives us no
/// notification-click callback (the plugin's actions API is mobile-only), but
/// clicking a notification *activates the app* — so we deep-link to this run
/// on the next unfocused→focused edge instead.
struct PendingOpen {
    project_id: String,
    run_id: String,
    at: std::time::Instant,
}

/// How long a notification stays deep-linkable. Long enough to cover reading
/// the banner and clicking it; short enough that a manual return to the app an
/// hour later doesn't teleport the user to a stale run.
const PENDING_OPEN_TTL: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Default)]
struct UiState {
    focused: bool,
    /// The run whose agent pane the UI currently has on screen (not merely
    /// selected — see the frontend's `onScreenRunId`). With `focused`, this is
    /// the whole basis for staying quiet: notifications skip this one run and
    /// nothing else.
    active_run: Option<String>,
    pending_open: Option<PendingOpen>,
}

pub struct AppState {
    registry: Mutex<Registry>,
    attaches: Mutex<HashMap<String, Subscription>>,
    run_attaches: Mutex<HashMap<String, Subscription>>,
    /// Attach handles for the per-run companion shell (`agency-shell-<id>`): an
    /// interactive login shell sharing the run's worktree, independent of the
    /// agent session and the run-script session. See `start_shell`.
    shell_attaches: Mutex<HashMap<String, Subscription>>,
    term: RwLock<TermClient>,
    data_dir: PathBuf,
    /// Run id → the branch the main checkout was on when that run's merge hit
    /// conflicts, so finishing or aborting the merge can put it back. Only
    /// `merge()`'s own clean path restores by itself; a conflicted merge stays
    /// on the base branch until resolution ends. In-memory: an app restart
    /// mid-merge just means the checkout is left on the base branch.
    merge_origins: Mutex<HashMap<String, String>>,
    ui: Mutex<UiState>,
    /// Run ids that have received user input since their last "waiting for input"
    /// notification. Drives idle-notification gating (see `notifier::step`).
    input_seen: Mutex<HashSet<String>>,
    /// Per-run busy/idle state, written by the notifier tick from its pane-hash
    /// diff and read into `RunInfo` (see `crate::activity`). In-memory only:
    /// it re-derives within one tick of an app start.
    activity: Mutex<HashMap<String, crate::activity::ActivityEntry>>,
    /// Per-run token and cost accounting, read from the agent's own transcript
    /// by the notifier tick. The cache carries the per-file stamps that keep
    /// the re-read nearly free; the `Usage` beside it is the directory total
    /// `RunInfo` serves. In-memory only: the transcripts are the source of
    /// truth, so a restart re-derives within one tick.
    usage: Mutex<HashMap<String, (agency_core::usage::UsageCache, agency_core::usage::Usage)>>,
    /// Run ids that have ever been given a turn (typed Enter, or created with a
    /// non-empty prompt). Unlike `input_seen` this is never consumed — it
    /// separates "waiting on the user" from "idle, never prompted" in
    /// `activity::classify`. In-memory: forgotten runs just show idle.
    prompted: Mutex<HashSet<String>>,
    /// Per-project lock on the project's own checkout: its index, its working
    /// tree, and the branch it stands on. Held by the merge family (whose
    /// status check → checkout → merge sequence is not atomic) and by every
    /// mutating `git_*` command whose target resolves to that checkout — a
    /// `project:<id>` token, a terminal, or an agent working without a
    /// worktree. All of those are async now, so this gate is the mutual
    /// exclusion the main thread used to provide for free (see the note at the
    /// top of `commands.rs`), and being per project means merging one repo no
    /// longer makes another one's Source Control panel wait.
    ///
    /// An agent's own worktree has its own index and gets no gate: the agent
    /// process commits there whenever it likes, outside anything Agency holds,
    /// so serializing our writes against each other would buy nothing.
    repo_gates: crate::gates::KeyedGates,
    /// Per-session lock on kill-then-spawn. The terminal daemon's registry
    /// inserts by session id, so two overlapping spawns of one id leave two
    /// agent processes running with only the second registered — the first
    /// keeps its pty and can never be killed again. `rerun`, `ensure_run_active`
    /// and `stop_run` are all async, so this is what makes that impossible now
    /// that the main thread doesn't. Looping runs are covered by `loop_gate`
    /// instead; both are taken in that order (spawn gate first) wherever a path
    /// needs the two, and `drive_loops` never takes this one.
    spawn_gates: crate::gates::KeyedGates,
    /// Serializes workspace creation (worktree add + branch cut). `create_run`
    /// is async so a large checkout can't freeze the UI; this replaces the
    /// main-thread serialization that previously kept concurrent creates from
    /// racing on git's worktree/branch state.
    worktree_gate: Mutex<()>,
    /// In-flight loop check commands, one slot per looping run id. The check
    /// runs on its own thread (never on the 2s poll tick); `drive_loops`
    /// drains finished slots into the looper state machine.
    checks: Mutex<HashMap<String, std::sync::Arc<CheckSlot>>>,
    /// Serializes loop transitions: `drive_loops` holds this per run across its
    /// read → step → persist → act sequence, and `stop_loop` holds it across
    /// persist-Stopped → kill. Without it an in-flight tick that read the run
    /// before a Stop observes the freshly killed session as Gone and respawns
    /// an attempt that no later tick would ever kill.
    loop_gate: Mutex<()>,
    /// Whether any loop might be active — the `drive_loops` fast path, so the
    /// no-loops case (the common one) costs one atomic load per tick instead
    /// of a registry scan. Starts true so loops persisted by a previous app
    /// run are picked up on the first tick; cleared when a full pass finds
    /// none, re-set by loop creation.
    loops_active: std::sync::atomic::AtomicBool,
    /// Bumped whenever a loop is created. `drive_loops` snapshots it before its
    /// scan and refuses to clear `loops_active` if it changed mid-pass — without
    /// this, a loop created in the exact window where a finishing pass is about
    /// to park the driver would clear the flag it just set and never be driven.
    loop_generation: std::sync::atomic::AtomicU64,
    /// Per-project signature of `.agency/issues/` (`scan_issue_stats`
    /// digest). `list_issues` reconciles the index from the files only when
    /// this changes, so an idle Issues-tab poll costs one readdir.
    issue_sigs: Mutex<HashMap<String, String>>,
    /// Per-project `git fetch` bookkeeping, shared by every automatic fetch —
    /// the background sweep and the UI's on-open/on-focus requests. See
    /// [`AppState::fetch_project_if_due`]. In-memory: an app restart just means
    /// the first request for each project fetches.
    fetches: Mutex<HashMap<String, FetchSched>>,
    /// Per-repo knowledge-graph build state, keyed by primary repo path. Guards
    /// against two builds of one repo overlapping (they write the same
    /// `graphify-out/`) and carries the running flag and last failure to the
    /// settings UI. In-memory: a restart just forgets a stale failure.
    kg_builds: std::sync::Arc<Mutex<HashMap<PathBuf, KgBuild>>>,
    /// Cancel tokens for in-flight repo-setup commits, keyed by folder. The
    /// setup dialog's Cancel flips one so the `git add -A` behind it stops:
    /// staging a folder of model weights can run for many minutes, and until
    /// this existed the only way out was to quit the app.
    setup_cancels: Mutex<HashMap<PathBuf, agency_core::setup::CancelToken>>,
}

/// When a project's origin was last contacted, and when it may be again.
struct FetchSched {
    /// Start of the last attempt, successful or not. `None` = never fetched,
    /// so the next request goes through whatever its `min_age`.
    last_attempt: Option<Instant>,
    /// Set only while a remote is failing: nothing is attempted before it.
    retry_after: Option<Instant>,
    /// Delay the *next* consecutive failure earns, doubling to the cap.
    backoff: Duration,
    /// A fetch is running now. Requests are dropped rather than queued: they
    /// all want the same thing, and the one in flight is already delivering it.
    in_flight: bool,
}

impl Default for FetchSched {
    fn default() -> Self {
        FetchSched {
            last_attempt: None,
            retry_after: None,
            backoff: FETCH_BACKOFF_BASE,
            in_flight: false,
        }
    }
}

/// Progress of a loop's check command. `Done(None)` = killed on timeout or
/// terminated by signal; anything but `Done(Some(0))` counts as "not done yet".
#[derive(Debug, Clone, Copy)]
enum CheckStatus {
    Running,
    Done(Option<i32>),
}

/// One in-flight check: the thread writes `status` when the child finishes;
/// `cancelled` tells the thread to kill the child now (loop stopped) instead
/// of letting it run to the timeout in a worktree about to be discarded.
struct CheckSlot {
    status: Mutex<CheckStatus>,
    cancelled: std::sync::atomic::AtomicBool,
}

/// A loop reaching a terminal state this tick, for the watcher thread to
/// notify about. `done` distinguishes Complete from Stalled.
pub struct LoopNotice {
    pub project_id: String,
    pub run_id: String,
    pub label: String,
    pub done: bool,
    pub attempt: u32,
}

impl AppState {
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn new(db_path: &Path, data_dir: &Path) -> Result<AppState> {
        let registry = Registry::open(db_path)?;
        // Terminals are not agents: they run `login_shell()` directly, so no
        // profile row backs them. Older installs seeded a "shell" profile that
        // showed up as an editable (and deletable) card in Settings; drop it.
        // No profiles are auto-seeded now — users pick them during onboarding
        // (or add them later from Settings → Add from catalog).
        registry.delete_profile(SHELL_AGENT)?;
        // Retrofit resume/loop recipes onto *existing* catalog profiles only
        // when unset, so a user's customized command/args/env is never
        // clobbered and deleted builtins stay gone across launches.
        for entry in crate::agent_catalog::builtins() {
            if registry.get_profile(entry.id)?.is_some() {
                registry.ensure_profile_resume_args(entry.id, &entry.resume_args)?;
                registry.ensure_profile_loop_args(entry.id, &entry.loop_args)?;
            }
        }
        // Existing installs already have agent profiles — mark onboarding done
        // so they aren't interrupted by the picker. Fresh DBs have no profiles
        // at all, so the flag stays unset and onboarding shows on first launch.
        let onboarding_done =
            registry.get_setting(SETTING_AGENT_ONBOARDING)?.as_deref() == Some("1");
        if !onboarding_done && !registry.list_profiles()?.is_empty() {
            registry.set_setting(SETTING_AGENT_ONBOARDING, "1")?;
        }
        let state = AppState {
            registry: Mutex::new(registry),
            attaches: Mutex::new(HashMap::new()),
            run_attaches: Mutex::new(HashMap::new()),
            shell_attaches: Mutex::new(HashMap::new()),
            term: RwLock::new(TermClient::connect_or_spawn(termd_socket(data_dir), termd_bin())?),
            data_dir: data_dir.to_path_buf(),
            merge_origins: Mutex::new(HashMap::new()),
            ui: Mutex::new(UiState { focused: true, active_run: None, pending_open: None }),
            input_seen: Mutex::new(HashSet::new()),
            activity: Mutex::new(HashMap::new()),
            usage: Mutex::new(HashMap::new()),
            prompted: Mutex::new(HashSet::new()),
            repo_gates: crate::gates::KeyedGates::new(),
            spawn_gates: crate::gates::KeyedGates::new(),
            worktree_gate: Mutex::new(()),
            checks: Mutex::new(HashMap::new()),
            loop_gate: Mutex::new(()),
            loops_active: std::sync::atomic::AtomicBool::new(true),
            loop_generation: std::sync::atomic::AtomicU64::new(0),
            issue_sigs: Mutex::new(HashMap::new()),
            fetches: Mutex::new(HashMap::new()),
            kg_builds: std::sync::Arc::new(Mutex::new(HashMap::new())),
            setup_cancels: Mutex::new(HashMap::new()),
        };
        // Rehydrate: any run the daemon still hosts is adopted as-is; the watch
        // loop (watch_snapshot) then reports live status. Nothing to spawn here —
        // surviving sessions are already running in the daemon.
        if let Ok(sessions) = state.term.read().unwrap().list() {
            log::info!("termd: adopted {} surviving session(s)", sessions.len());
        }
        Ok(state)
    }

    fn provider_env(&self) -> Result<Vec<(String, String)>> {
        // Point OpenAI-protocol agents at the configured local model (LM Studio
        // by default). Agents with their own CLI auth (claude, codex, …) ignore
        // these. No cloud keys are injected — each agent uses its own login.
        let s = self.get_settings()?;
        Ok(vec![
            ("OPENAI_BASE_URL".into(), s.lm_studio_base_url),
            ("OPENAI_API_KEY".into(), "lm-studio".into()),
        ])
    }

    pub fn register_profile(&self, profile: AgentProfile) -> Result<()> {
        self.registry.lock().unwrap().upsert_profile(&profile)
    }

    pub fn profile_names(&self) -> Result<Vec<String>> {
        Ok(self.registry.lock().unwrap().list_profiles()?.into_iter().map(|p| p.name).collect())
    }

    pub fn list_profiles(&self) -> Result<Vec<AgentProfile>> {
        self.registry.lock().unwrap().list_profiles()
    }

    pub fn delete_profile(&self, name: &str) -> Result<()> {
        self.registry.lock().unwrap().delete_profile(name)
    }

    /// Whether agent-profile onboarding still needs to run.
    pub fn agent_onboarding_needed(&self) -> Result<bool> {
        let reg = self.registry.lock().unwrap();
        Ok(reg.get_setting(SETTING_AGENT_ONBOARDING)?.as_deref() != Some("1"))
    }

    /// What the model picker needs for every enabled agent profile: whether a
    /// model can be set at all, what to offer, and what was chosen last.
    ///
    /// Suggestions are stable vendor aliases only (see the catalog), so this is
    /// deliberately not a list of every model that exists — a typed id and the
    /// per-agent recents are what cover the rest. Cheap enough to call every
    /// time a menu opens: settings reads and static data, no PATH probing and
    /// no launching of agent CLIs.
    pub fn list_agent_models(&self) -> Result<Vec<AgentModelInfo>> {
        let reg = self.registry.lock().unwrap();
        let mut out = Vec::new();
        for profile in reg.list_profiles()? {
            let entry = crate::agent_catalog::find(&profile.name);
            let selected = reg
                .get_setting(&format!("{SETTING_AGENT_MODEL_PREFIX}{}", profile.name))?
                .filter(|m| !m.is_empty());
            let recent = reg
                .get_setting(&format!("{SETTING_AGENT_MODELS_PREFIX}{}", profile.name))?
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
            out.push(AgentModelInfo {
                agent: profile.name.clone(),
                supported: crate::agent_catalog::supports_model(&profile.name),
                suggested: entry
                    .map(|e| e.models.iter().map(|m| (*m).to_string()).collect())
                    .unwrap_or_default(),
                recent,
                selected,
                list_command: entry.and_then(|e| e.list_models).map(String::from),
            });
        }
        Ok(out)
    }

    /// Catalog entries with enabled/installed flags for the UI picker.
    pub fn list_agent_catalog(&self) -> Result<Vec<crate::agent_catalog::CatalogEntryInfo>> {
        let builtins = crate::agent_catalog::builtins();
        // Read the enabled flags under the lock, then drop it before probing
        // PATH (a per-command filesystem scan) so the registry mutex isn't held
        // across ~10 syscall-heavy lookups.
        let enabled: Vec<bool> = {
            let reg = self.registry.lock().unwrap();
            builtins.iter().map(|e| Ok(reg.get_profile(e.id)?.is_some())).collect::<Result<_>>()?
        };
        Ok(builtins
            .iter()
            .zip(enabled)
            .map(|(entry, enabled)| crate::agent_catalog::CatalogEntryInfo {
                id: entry.id.to_string(),
                command: entry.command.to_string(),
                resume_args: entry.resume_args.clone(),
                loop_args: entry.loop_args.clone(),
                enabled,
                installed: command_on_path(entry.command),
                supports_mcp: agency_core::mcp::agent_supported(entry.id),
                supports_mcp_auth: agency_core::mcp::auth_supported(entry.id),
                accepts_prompt: entry.prompt != crate::agent_catalog::PromptDelivery::Unsupported,
            })
            .collect())
    }

    /// Upsert catalog recipes for the given ids (idempotent). Unknown ids error.
    pub fn enable_agent_profiles(&self, ids: &[String]) -> Result<()> {
        let reg = self.registry.lock().unwrap();
        for id in ids {
            let entry = crate::agent_catalog::find(id)
                .ok_or_else(|| anyhow!("unknown catalog agent: {id}"))?;
            // Don't clobber a customized profile the user already has.
            if reg.get_profile(id)?.is_none() {
                reg.upsert_profile(&crate::agent_catalog::profile_for(entry))?;
            }
        }
        Ok(())
    }

    /// Enable selected catalog agents and mark onboarding complete.
    pub fn complete_agent_onboarding(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            // Allow completing with only custom profiles already saved — the UI
            // guarantees at least one profile exists before calling.
            if self.list_profiles()?.is_empty() {
                bail!("select at least one agent profile");
            }
        } else {
            self.enable_agent_profiles(ids)?;
        }
        self.registry.lock().unwrap().set_setting(SETTING_AGENT_ONBOARDING, "1")?;
        Ok(())
    }

    pub fn get_settings(&self) -> Result<ProviderSettings> {
        let reg = self.registry.lock().unwrap();
        Ok(ProviderSettings {
            lm_studio_base_url: reg
                .get_setting(SETTING_LM_STUDIO_URL)?
                .unwrap_or_else(|| DEFAULT_LM_STUDIO_URL.to_string()),
            // Stored as "" when unset; surface that as None so the UI shows "Auto".
            default_agent: reg.get_setting(SETTING_DEFAULT_AGENT)?.filter(|s| !s.is_empty()),
            // Unset = on, so existing installs keep cutting worktrees.
            default_worktree: reg.get_setting(SETTING_DEFAULT_WORKTREE)? != Some("0".to_string()),
        })
    }

    pub fn save_settings(&self, s: &ProviderSettings) -> Result<()> {
        validate_provider_url(&s.lm_studio_base_url)?;
        let reg = self.registry.lock().unwrap();
        reg.set_setting(SETTING_LM_STUDIO_URL, &s.lm_studio_base_url)?;
        reg.set_setting(SETTING_DEFAULT_AGENT, s.default_agent.as_deref().unwrap_or(""))?;
        reg.set_setting(SETTING_DEFAULT_WORKTREE, if s.default_worktree { "1" } else { "0" })?;
        Ok(())
    }

    pub fn run_title(&self, id: &str) -> Result<Option<String>> {
        let reg = self.registry.lock().unwrap();
        Ok(reg.get_run(id)?.and_then(|r| r.title))
    }

    pub fn store_run_title(&self, id: &str, title: &str) -> Result<()> {
        self.registry.lock().unwrap().set_run_title(id, title)
    }

    pub fn add_project(&self, name: &str, repo_path: &Path) -> Result<Project> {
        validate_project_path(repo_path)?;
        self.registry.lock().unwrap().add_project(name, repo_path)
    }

    // ── workspace (the pinned notes/journal project) ───────────────────────

    /// The pinned workspace project, if the user has created it.
    pub fn get_workspace(&self) -> Result<Option<Project>> {
        self.registry.lock().unwrap().get_workspace()
    }

    /// The folder offered by default for a new workspace: `~/Agency`.
    pub fn default_workspace_location(&self) -> std::path::PathBuf {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/"))
            .join("Agency")
    }

    /// Create (or adopt) the workspace at `path`. Unlike `add_project`, git is
    /// optional: with `use_git` the folder is initialized and given an initial
    /// commit so agents can spawn in it; without, it's just a folder — Docs and
    /// Files work, Source Control and agents stay hidden in the UI. Idempotent:
    /// an existing workspace row is reused (and repointed at `path`).
    pub fn create_workspace(&self, path: &Path, use_git: bool) -> Result<Project> {
        std::fs::create_dir_all(path)
            .with_context(|| format!("creating workspace folder {}", path.display()))?;
        // Seed the starter guide before the initial commit so it's tracked.
        // Only when missing — re-running create on an adopted folder (or the
        // idempotent re-create path) must not clobber user edits.
        agency_core::guide::ensure_guide(path)?;
        if use_git {
            use agency_core::setup::RepoReadiness;
            if matches!(agency_core::setup::repo_readiness(path), RepoReadiness::NotARepo) {
                agency_core::setup::init_repo(path)?;
            }
            if matches!(agency_core::setup::repo_readiness(path), RepoReadiness::NoCommits { .. }) {
                agency_core::setup::initial_commit(path, true)?;
            }
        }
        self.registry.lock().unwrap().ensure_workspace("Workspace", path)
    }

    /// Move the workspace folder on disk and repoint its project row. Refuses
    /// to clobber an existing destination; a cross-volume move surfaces the
    /// rename error rather than silently copying.
    pub fn move_workspace(&self, new_path: &Path) -> Result<Project> {
        let ws = self
            .get_workspace()?
            .ok_or_else(|| anyhow!("no workspace to move; create it first"))?;
        if new_path == ws.repo_path {
            return Ok(ws);
        }
        // Live runs keep worktrees under `<ws>/.agency/worktrees/` whose git
        // metadata records absolute paths (the worktree's `.git` file and the
        // main repo's `.git/worktrees/<id>/gitdir`); renaming the folder
        // would orphan them and strand any unmerged work. Registry list_runs
        // already excludes archived runs.
        let live = self.registry.lock().unwrap().list_runs(&ws.id)?.len();
        if live > 0 {
            bail!(
                "the workspace has {live} active agent run{}; merge or archive them before moving",
                if live == 1 { "" } else { "s" }
            );
        }
        // A folder cannot move into itself; the location picker makes this
        // easy to do by creating the destination inside the open workspace.
        if new_path.starts_with(&ws.repo_path) {
            bail!(
                "the destination is inside the current workspace; choose a location outside {}",
                ws.repo_path.display()
            );
        }
        if new_path.exists() {
            bail!("{} already exists; choose a new location", new_path.display());
        }
        if let Some(parent) = new_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&ws.repo_path, new_path).with_context(|| {
            format!("moving {} to {}", ws.repo_path.display(), new_path.display())
        })?;
        let reg = self.registry.lock().unwrap();
        reg.set_project_repo_path(&ws.id, new_path)?;
        reg.get_project(&ws.id)?.ok_or_else(|| anyhow!("workspace row vanished"))
    }

    /// Whether `project_id` is the pinned workspace (drives the whole-folder
    /// docs vault and the relaxed-git UI gating).
    pub fn project_is_workspace(&self, project_id: &str) -> Result<bool> {
        let reg = self.registry.lock().unwrap();
        Ok(reg
            .get_project(project_id)?
            .map(|p| p.kind.as_deref() == Some(agency_core::registry::PROJECT_KIND_WORKSPACE))
            .unwrap_or(false))
    }

    pub fn inspect_repo(&self, repo_path: &Path) -> agency_core::setup::RepoReadiness {
        agency_core::setup::repo_readiness(repo_path)
    }

    pub fn init_repo(&self, repo_path: &Path) -> Result<()> {
        agency_core::setup::init_repo(repo_path)
    }

    pub fn clone_repo(
        &self,
        url: &str,
        parent_dir: &Path,
        on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<std::path::PathBuf> {
        agency_core::setup::clone_repo_with_progress(url, parent_dir, on_progress)
    }

    /// Stage and commit `repo_path`, registering a cancel token for it first so
    /// [`AppState::cancel_repo_setup`] can stop the staging pass.
    pub fn commit_repo(
        &self,
        repo_path: &Path,
        add_gitignore: bool,
        ignore_paths: Vec<String>,
        on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<()> {
        let cancel = agency_core::setup::CancelToken::new();
        let key = repo_path.to_path_buf();
        self.setup_cancels.lock().unwrap().insert(key.clone(), cancel.clone());
        let opts = agency_core::setup::CommitOptions { add_gitignore, ignore_paths, cancel };
        let out = agency_core::setup::initial_commit_with_progress(repo_path, &opts, on_progress);
        self.setup_cancels.lock().unwrap().remove(&key);
        out
    }

    /// Stop a setup commit running against `repo_path`, if there is one. A no-op
    /// otherwise — the dialog also cancels during steps with nothing to kill.
    ///
    /// A Cancel that somehow beats `commit_repo` to registering its token is
    /// dropped. Closing that window means either a tombstone (which poisons the
    /// next attempt for the folder if no commit follows it) or a separate
    /// registration command, and neither is worth it: reaching the window takes
    /// a stalled async runtime *and* a click landing within milliseconds of the
    /// one that started the commit.
    pub fn cancel_repo_setup(&self, repo_path: &Path) {
        if let Some(cancel) = self.setup_cancels.lock().unwrap().get(repo_path) {
            cancel.cancel();
        }
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.registry.lock().unwrap().list_projects()
    }

    pub fn set_project_color(&self, id: &str, color: &str) -> Result<()> {
        self.registry.lock().unwrap().set_project_color(id, color)
    }

    pub fn close_project(&self, id: &str) -> Result<()> {
        self.close_project_with_progress(id, &mut |_| {})
    }

    /// [`close_project`] reporting which agent it is stopping. Every kill is a
    /// round-trip to the terminal daemon, and a busy project has one per agent
    /// plus its shell and extra tabs, so the count says how far through the
    /// list it is rather than leaving the dialog looking stuck.
    pub fn close_project_with_progress(
        &self,
        id: &str,
        on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress),
    ) -> Result<()> {
        // Kill live terminals, then hide the project from the list. Project +
        // run records (and extra-session rows) and the worktrees on disk are
        // all kept — re-adding the same repo path revives everything.
        let runs = self.registry.lock().unwrap().list_runs(id)?;
        let total = runs.len();
        for (i, run) in runs.iter().enumerate() {
            step(on_progress, "Stopping the agents", &sweep_detail(&run_label(run), i, total));
            self.kill_run_terminals(&run.id);
        }
        step(on_progress, "Closing the project", "");
        self.registry.lock().unwrap().set_project_closed(id, true)?;
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        self.delete_project_with_progress(id, &mut |_| {})
    }

    /// [`delete_project`] reporting where the teardown is. This is the slowest
    /// one in the app: it is [`discard_run_with_progress`] once per agent in
    /// the project, worktree removal and all, so on a dozen agents it runs for
    /// tens of seconds and the caller is a modal the user is staring at.
    pub fn delete_project_with_progress(
        &self,
        id: &str,
        on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress),
    ) -> Result<()> {
        let runs = self.registry.lock().unwrap().list_runs(id)?;
        let repo = self.project_repo(id).ok();
        let total = runs.len();
        for (i, run) in runs.iter().enumerate() {
            // Position in the sweep, not the branch name: how far through the
            // project's agents it is matters more here, and without it the
            // phases look like a single teardown restarting over and over.
            let detail = sweep_detail(&run_label(run), i, total);
            step(on_progress, "Stopping the agents", &detail);
            self.kill_run_terminals(&run.id);
            if let Some(repo) = &repo {
                step(on_progress, "Removing the worktrees", &detail);
                // Same mutual exclusion `create_run` takes: this command is
                // async now (off the main thread), so nothing else serializes
                // it against a concurrent worktree add on the same repo. Held
                // per run, not across the sweep, so one long project deletion
                // doesn't block every spawn for its whole duration.
                let _gate = self.worktree_gate.lock().unwrap();
                let _ = WorktreeManager::new(repo.clone()).remove(&run.id);
            }
            let reg = self.registry.lock().unwrap();
            reg.delete_run_sessions(&run.id)?;
            reg.delete_run(&run.id)?;
        }
        step(on_progress, "Cleaning up", "");
        {
            let reg = self.registry.lock().unwrap();
            reg.delete_project_issues(id)?;
            reg.remove_project(id)?;
        }
        Ok(())
    }

    /// Drop every live terminal a run owns: the agent session, its run scripts,
    /// its shell and any extra tabs. Shared by the two project teardowns, which
    /// differ only in what they do with the worktree afterwards.
    fn kill_run_terminals(&self, run_id: &str) {
        self.attaches.lock().unwrap().remove(run_id);
        let _ = self.term.read().unwrap().kill(&session_name(run_id));
        self.kill_run_sessions(run_id);
        self.shell_attaches.lock().unwrap().remove(run_id);
        let _ = self.term.read().unwrap().kill(&shell_session_name(run_id));
        self.kill_extra_sessions(run_id);
    }

    pub fn project_repo_path(&self, project_id: &str) -> anyhow::Result<std::path::PathBuf> {
        self.project_repo(project_id)
    }

    // ── private helpers ────────────────────────────────────────────────────────

    fn project_repo(&self, project_id: &str) -> Result<std::path::PathBuf> {
        let reg = self.registry.lock().unwrap();
        Ok(reg
            .get_project(project_id)?
            .ok_or_else(|| anyhow!("unknown project: {project_id}"))?
            .repo_path)
    }

    fn run_record(&self, id: &str) -> Result<agency_core::registry::Run> {
        let reg = self.registry.lock().unwrap();
        reg.get_run(id)?.ok_or_else(|| anyhow!("unknown run: {id}"))
    }

    fn run_info(&self, run: &agency_core::registry::Run) -> RunInfo {
        let live = self.term.read().unwrap().list().unwrap_or_default();
        self.run_info_from(run, &live)
    }

    /// `run_info` against a session listing the caller already fetched. Both the
    /// agent's status and the run-script dot come out of it, so listing every
    /// run in a project is one daemon round-trip for the lot — the listing the
    /// daemon returns is the same map its per-session `status` reads from.
    fn run_info_from(
        &self,
        run: &agency_core::registry::Run,
        live: &[(String, SessionStatus)],
    ) -> RunInfo {
        let name = session_name(&run.id);
        let status = live
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, s)| s.clone())
            .unwrap_or(SessionStatus::Gone);
        let wt = self
            .project_repo(&run.project_id)
            .ok()
            .map(|repo| workspace_dir(&repo, run))
            .filter(|p| p.exists());
        // A run on its own branch is measured by what that branch added over its
        // base. A run in the main checkout has no branch of its own, so the
        // honest measure is what is currently uncommitted there.
        let stat = wt
            .as_ref()
            .and_then(|p| {
                if run.worktree {
                    agency_core::git::diff_stat(p, &run.base).ok()
                } else {
                    agency_core::git::uncommitted_stat(p).ok()
                }
            })
            .unwrap_or(agency_core::git::DiffStat { added: 0, deleted: 0, files: 0 });
        // A worktree's branch is fixed for its life, so the stored name is the
        // truth. A run in the main checkout follows whatever the user checks
        // out there, so read it live rather than showing a stale name.
        let branch = match (run.worktree, &wt) {
            (false, Some(p)) => {
                agency_core::merge::current_branch(p).unwrap_or_else(|| run.branch.clone())
            }
            _ => run.branch.clone(),
        };
        RunInfo {
            id: run.id.clone(),
            project_id: run.project_id.clone(),
            agent: run.agent.clone(),
            prompt: run.prompt.clone(),
            title: run.title.clone(),
            branch,
            status,
            activity: self.activity.lock().unwrap().get(&run.id).map(|e| {
                // Loops drive themselves — a quiet attempt isn't waiting on
                // the user, so it classifies as idle at most.
                let turn_driven =
                    run.loop_config.is_none() && self.prompted.lock().unwrap().contains(&run.id);
                crate::activity::classify(e, turn_driven, crate::activity::now_ms())
            }),
            usage: self.usage.lock().unwrap().get(&run.id).map(|(_, u)| u.into()),
            added: stat.added,
            deleted: stat.deleted,
            files: stat.files,
            port: run.port_base,
            kind: run.kind.clone(),
            run_scripts_live: any_run_script_live(&run.id, live),
            worktree: run.worktree,
            race_id: run.race_id.clone(),
            loop_config: run.loop_config.clone(),
            loop_state: run.loop_state.clone(),
            issue_id: run.issue_id.clone(),
            model: run.model.clone(),
            created_at: run.created_at,
            archived_at: run.archived_at,
        }
    }

    // ── run lifecycle ──────────────────────────────────────────────────────────

    fn allocate_port(&self, base: u16, block_size: u16) -> Result<u16> {
        let mut used: std::collections::HashSet<u16> =
            self.registry.lock().unwrap().list_port_bases()?.into_iter().collect();
        // `base` itself belongs to the project's own checkout, whose Run tab
        // hands it to `$AGENCY_PORT` (see `run_target`). Agents start a block
        // above so their dev server never fights the one you started yourself.
        used.insert(base);
        pick_port(&used, base, block_size).ok_or_else(|| anyhow!("no free port block available"))
    }

    pub fn create_run(
        &self,
        project_id: &str,
        prompt: &str,
        agent: &str,
        model: Option<&str>,
        base: &str,
        merge_target: Option<&str>,
    ) -> Result<RunInfo> {
        self.create_run_with_progress(
            project_id,
            prompt,
            agent,
            model,
            base,
            merge_target,
            true,
            |_| {},
        )
    }

    /// Like [`create_run`], but streams workspace-setup progress to `on_progress`
    /// (worktree checkout + essential-file copy). Used by the async `create_run`
    /// Tauri command so the UI shows movement instead of freezing on a large repo.
    ///
    /// `worktree = false` skips the worktree and runs the agent in the project's
    /// main checkout on its current branch; `base`/`merge_target` are then
    /// ignored, since there is no branch to cut or land.
    #[allow(clippy::too_many_arguments)]
    pub fn create_run_with_progress(
        &self,
        project_id: &str,
        prompt: &str,
        agent: &str,
        model: Option<&str>,
        base: &str,
        merge_target: Option<&str>,
        worktree: bool,
        mut on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<RunInfo> {
        self.create_run_spec(
            NewRunSpec {
                project_id,
                prompt,
                agent,
                model,
                base,
                merge_target,
                race_id: None,
                title: None,
                existing_branch: None,
                loop_config: None,
                issue_id: None,
                worktree,
            },
            &mut on_progress,
        )
    }

    fn create_run_spec(
        &self,
        spec: NewRunSpec,
        on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress),
    ) -> Result<RunInfo> {
        // Serialize workspace creation. The sync mutating commands run on the
        // main thread and so are serialized there; `create_run` is now async
        // (off the main thread, so it can't freeze the UI on a big checkout), so
        // this gate stands in for that mutual exclusion against concurrent
        // worktree/branch creation.
        let _gate = self.worktree_gate.lock().unwrap();
        let repo = self.project_repo(spec.project_id)?;
        let config = agency_core::config::load(&repo);
        let port = self.allocate_port(config.ports.base, config.ports.block_size)?;
        // Refuse a model the agent's CLI has no way to be told, rather than
        // dropping it and letting the run come up on a model the user did not
        // pick. The pickers only offer models where a recipe exists, so this is
        // the backstop for the raw command.
        let model = spec.model.map(str::to_string);
        if model.is_some() && !crate::agent_catalog::supports_model(spec.agent) {
            bail!(
                "'{}' has no way to be told a model on the command line — set it in that CLI's \
                 own config instead",
                spec.agent
            );
        }
        let (profile, issue_key) = {
            let reg = self.registry.lock().unwrap();
            let profile = reg
                .get_profile(spec.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {agent}", agent = spec.agent))?;
            let profile = with_model(&profile, model.as_deref());
            // The prefix the worktree's tracker briefing names, so `AGE-14`
            // reads to the agent as this project's key rather than a shape it
            // recognizes from some other tracker.
            let (_, key) = self.issue_root(&reg, spec.project_id)?;
            (profile, key)
        };
        let id = new_task_id(spec.prompt);
        let manager = WorktreeManager::new(repo.clone());
        // Three shapes: cut agent/<id> from the base (the default), check out a
        // PR's existing head branch, or skip the worktree entirely and work in
        // the project's own checkout on whatever branch is there.
        let workspace = if !spec.worktree {
            // A project folder with no repository has no branch to name, so the
            // run records none — the same shape terminals have always had. Only
            // a real checkout that is mid-rebase or otherwise off any branch is
            // an error, since there the branch is missing unexpectedly.
            let branch = if require_gitless_known(&repo)? {
                String::new()
            } else {
                agency_core::merge::current_branch(&repo)
                    .ok_or_else(|| anyhow!("the project checkout is not on a branch, so an agent can't work in it directly; create a worktree instead"))?
            };
            agency_core::worktree::Worktree { task_id: id.clone(), path: repo.clone(), branch }
        } else {
            // Races, loops and issue dispatch always ask for a worktree. Say why
            // it can't happen here rather than surfacing a raw git error; the UI
            // hides these entries for a gitless project, so this is the backstop.
            if require_gitless_known(&repo)? {
                bail!(
                    "{} is not a git repository, so there is no branch to cut a worktree from; \
                     initialize one to give agents isolated branches",
                    repo.display()
                );
            }
            match &spec.existing_branch {
                Some(branch) => manager.create_on_branch_with_progress(&id, branch, on_progress)?,
                None => manager.create_with_progress(&id, spec.base, on_progress)?,
            }
        };
        if spec.worktree {
            // Untracked essentials (.env etc.) don't come with a worktree; copy the
            // configured list plus auto-detected root .env files. Best-effort: a bad
            // entry shouldn't block the run. A run in the main checkout already has
            // them, by definition.
            on_progress(agency_core::setup::CloneProgress {
                phase: "Copying files".into(),
                percent: None,
                detail: String::new(),
            });
            if let Err(e) = manager.copy_essentials(&id, &config.files.copy) {
                log::warn!("copying essentials into worktree {id}: {e}");
            }
            self.emit_mcp(spec.agent, &repo, &workspace.path, &config);
            // Introduce the issue tracker. The prompt does this for an
            // issue-dispatched run, but a plain run carries only the user's
            // text and the default interactive run carries none at all, so
            // without this an agent asked for a follow-up has never been told
            // the tracker exists, let alone that it lives outside the worktree.
            // Best-effort, like the copies above: a briefing that can't be
            // written is not worth failing a run over.
            if let Err(e) =
                agency_core::briefing::emit_agents_md(&workspace.path, &repo, &issue_key)
            {
                log::warn!("writing the tracker briefing into worktree {id}: {e}");
            }
        } else if !self.merged_mcp_servers(&repo, &config).is_empty() {
            // Emitting would rewrite `.mcp.json` (or the agent's equivalent) in
            // the user's own checkout — a tracked file in most repos. Dirtying
            // the working copy is not ours to do, so say why they're missing.
            log::info!(
                "run {id} works in the project checkout: MCP servers not emitted, \
                 since that would modify a file in the checkout"
            );
        }

        // Looping runs are spawned below via spawn_loop_attempt — the same
        // path the driver uses for every respawn — so there is exactly one
        // place that builds a headless attempt.
        if spec.loop_config.is_none() {
            let mut env = self.provider_env()?;
            env.extend(profile.env.iter().cloned());
            env.extend(agency_core::scripts::script_env(&workspace.path, &repo, &id, Some(port)));
            // The default flow passes "" and behaves exactly as before: the user
            // types the real prompt into the live terminal.
            let (command, args) = fresh_agent_argv(
                &profile,
                &workspace.path,
                spec.prompt,
                config.scripts.setup.as_deref(),
            );
            if let Err(e) = self.term.read().unwrap().start_session(
                &session_name(&id),
                &workspace.path,
                &command,
                &args,
                &env,
                220,
                50,
            ) {
                // Roll back the worktree + branch we just cut: the run record is
                // inserted below, so on a spawn failure nothing references them —
                // leaving them would orphan a worktree/branch on every failure.
                // Nothing was cut for a run in the main checkout, and `remove`
                // would delete the branch the user is sitting on.
                if spec.worktree {
                    let _ = manager.remove(&id);
                }
                return Err(e.into());
            }
        }

        // A run created with a real prompt gets a title immediately (word-based;
        // no LLM on this path). The promptless flow still titles via the
        // first-prompt capture.
        let title = spec.title.clone().or_else(|| {
            let t = agency_core::title::fallback_title(spec.prompt);
            (!t.is_empty()).then_some(t)
        });

        let run = agency_core::registry::Run {
            id: id.clone(),
            project_id: spec.project_id.to_string(),
            agent: spec.agent.to_string(),
            prompt: spec.prompt.to_string(),
            // A run in the main checkout has no branch cut off a base; recording
            // the branch it works on keeps `base` meaningful rather than storing
            // a picker value that was never used.
            base: if spec.worktree { spec.base.to_string() } else { workspace.branch.clone() },
            branch: workspace.branch.clone(),
            created_at: now_secs(),
            port_base: Some(port),
            archived_at: None,
            title,
            kind: "agent".to_string(),
            // Nothing to land, so no target to remember.
            merge_target: spec.worktree.then(|| spec.merge_target.map(|s| s.to_string())).flatten(),
            race_id: spec.race_id.clone(),
            loop_state: spec
                .loop_config
                .as_ref()
                .map(|_| agency_core::loops::LoopState::new(now_secs())),
            loop_config: spec.loop_config.clone(),
            issue_id: spec.issue_id.clone(),
            worktree: spec.worktree,
            model: model.clone(),
        };
        {
            let reg = self.registry.lock().unwrap();
            reg.insert_run(&run)?;
            // Remember the agent type so new-task shortcuts default to what
            // this project actually uses. Best-effort bookkeeping.
            let _ = reg.set_project_default_agent(spec.project_id, spec.agent);
            // Same for the model: the picker reopens on the last one used for
            // this agent, so repeating a choice is one click rather than a
            // retyped id. Recorded for a default pick too, so choosing
            // "default" after a run on opus actually sticks.
            let _ = remember_model(&reg, spec.agent, model.as_deref());
        }
        // A run born with a real prompt is already mid-turn: when it goes
        // quiet it's waiting on the user, same as after a typed turn.
        if !spec.prompt.trim().is_empty() {
            self.prompted.lock().unwrap().insert(id.clone());
        }
        if run.loop_config.is_some() {
            // Wake the driver's fast path before spawning: even if this first
            // spawn fails, the driver retries (and stalls loudly if it keeps
            // failing) instead of the loop sitting invisible. Bump the
            // generation and set the flag UNDER loop_gate: drive_loops clears
            // the flag under the same gate and only after re-reading the
            // generation, so the bump+set here and its clear there are mutually
            // exclusive. Without the shared gate the two stores race and a
            // finishing pass can clobber this freshly-set flag, parking the new
            // loop until the next creation.
            let _gate = self.loop_gate.lock().unwrap();
            self.loop_generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.loops_active.store(true, std::sync::atomic::Ordering::SeqCst);
            self.spawn_loop_attempt(&run)?;
        }
        Ok(self.run_info(&run))
    }

    /// Fan one prompt out to several agents in parallel workspaces (racing).
    /// Each attempt is an ordinary run sharing a race_id; the user compares
    /// them and merges the winner. Partial failures leave the already-created
    /// attempts in place (visible and individually discardable).
    pub fn create_race(
        &self,
        project_id: &str,
        prompt: &str,
        agents: &[String],
        models: &HashMap<String, String>,
        base: &str,
        merge_target: Option<&str>,
    ) -> Result<Vec<RunInfo>> {
        self.create_race_inner(project_id, prompt, agents, models, base, merge_target, None, None)
    }

    /// `models` maps an agent id to the model that attempt runs on; an agent
    /// missing from it races on its own default. Keyed by agent because model
    /// ids live in each CLI's own namespace, so there is no one model to give
    /// a whole race.
    #[allow(clippy::too_many_arguments)]
    fn create_race_inner(
        &self,
        project_id: &str,
        prompt: &str,
        agents: &[String],
        models: &HashMap<String, String>,
        base: &str,
        merge_target: Option<&str>,
        title: Option<String>,
        issue_id: Option<String>,
    ) -> Result<Vec<RunInfo>> {
        if prompt.trim().is_empty() {
            bail!("racing needs a prompt — it is sent to every agent at launch");
        }
        if agents.len() < 2 {
            bail!("racing needs at least two agents");
        }
        // Up front, before any workspace is cut: create_run_spec refuses a model
        // the agent can't be told, and finding that out on the third attempt
        // would leave the first two running a race they can no longer win.
        for agent in agents {
            if models.contains_key(agent) && !crate::agent_catalog::supports_model(agent) {
                bail!("'{agent}' has no way to be told a model on the command line");
            }
        }
        let race_id = uuid::Uuid::new_v4().to_string();
        let mut out = Vec::new();
        for agent in agents {
            out.push(self.create_run_spec(
                NewRunSpec {
                    project_id,
                    prompt,
                    agent,
                    model: models.get(agent).map(String::as_str),
                    base,
                    merge_target,
                    race_id: Some(race_id.clone()),
                    title: title.clone(),
                    existing_branch: None,
                    loop_config: None,
                    issue_id: issue_id.clone(),
                    worktree: true,
                },
                &mut |_| {},
            )?);
        }
        Ok(out)
    }

    /// Create a looping run: one workspace whose agent is re-invoked headless
    /// (fresh context every attempt, state on disk) until the check command
    /// exits 0 or the attempt cap is spent. The loop driver in the watcher
    /// thread owns the session from here.
    #[allow(clippy::too_many_arguments)]
    pub fn create_loop(
        &self,
        project_id: &str,
        prompt: &str,
        agent: &str,
        model: Option<&str>,
        base: &str,
        merge_target: Option<&str>,
        check_command: &str,
        max_attempts: u32,
    ) -> Result<RunInfo> {
        self.create_loop_inner(
            project_id,
            prompt,
            agent,
            model,
            base,
            merge_target,
            check_command,
            max_attempts,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_loop_inner(
        &self,
        project_id: &str,
        prompt: &str,
        agent: &str,
        model: Option<&str>,
        base: &str,
        merge_target: Option<&str>,
        check_command: &str,
        max_attempts: u32,
        title: Option<String>,
        issue_id: Option<String>,
    ) -> Result<RunInfo> {
        if prompt.trim().is_empty() {
            bail!("a loop needs a prompt — it is re-sent to the agent on every attempt");
        }
        // Validate the recipe BEFORE create_run_spec cuts the worktree and
        // branch: a loop-incapable agent (the dialog filters these, the raw
        // command does not) must fail cleanly instead of leaking an orphaned
        // worktree that no run record references.
        {
            let profile = self
                .registry
                .lock()
                .unwrap()
                .get_profile(agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {agent}"))?;
            if profile.loop_args.as_ref().is_none_or(|a| a.is_empty()) {
                bail!("agent '{agent}' has no loop recipe — set the profile's loop args first");
            }
        }
        let cfg = agency_core::loops::LoopConfig {
            check_command: check_command.trim().to_string(),
            max_attempts: max_attempts.clamp(1, 100),
            check_timeout_secs: 600,
        };
        self.create_run_spec(
            NewRunSpec {
                project_id,
                prompt,
                agent,
                model,
                base,
                merge_target,
                race_id: None,
                title,
                existing_branch: None,
                loop_config: Some(cfg),
                issue_id,
                worktree: true,
            },
            &mut |_| {},
        )
    }

    /// The prompt, run title, and default base for dispatching a local issue,
    /// labeled with the project's issue key ("AGE-14 Fix login"). One place
    /// composes these so run, race, and loop dispatch can't drift.
    fn issue_dispatch(
        &self,
        issue_id: &str,
    ) -> Result<(agency_core::registry::Issue, String, String)> {
        let (issue, key, root) = {
            let reg = self.registry.lock().unwrap();
            let issue =
                reg.get_issue(issue_id)?.ok_or_else(|| anyhow!("unknown issue: {issue_id}"))?;
            // Dispatch is an issue-touching path: make sure the file exists
            // before an agent is told where to find it.
            self.ensure_issue_files(&reg, &issue.project_id)?;
            let (root, key) = self.issue_root(&reg, &issue.project_id)?;
            (issue, key, root)
        };
        let label = format!("{key}-{}", issue.seq);
        let prompt = issue_prompt(&label, &issue.title, &issue.body, &issue.comments, &root);
        let title = format!("{label} {}", issue.title);
        Ok((issue, prompt, title))
    }

    /// Resolve the base branch an issue-dispatched workspace is cut from:
    /// the caller's explicit pick, else the repo's main/master.
    fn issue_base(&self, project_id: &str, base: Option<&str>) -> Result<String> {
        match base {
            Some(b) if !b.trim().is_empty() => Ok(b.to_string()),
            _ => agency_core::merge::detect_base(&self.project_repo(project_id)?),
        }
    }

    /// Dispatch a local issue to an agent: the issue becomes the run's prompt
    /// and title, the run links back via issue_id, and the issue advances to
    /// in_progress. The agent-native counterpart of `create_run_from_issue`.
    pub fn start_issue_run(
        &self,
        issue_id: &str,
        agent: &str,
        model: Option<&str>,
        base: Option<&str>,
        merge_target: Option<&str>,
    ) -> Result<RunInfo> {
        let (issue, prompt, title) = self.issue_dispatch(issue_id)?;
        // With no repository there is no base to cut from and no branch to land,
        // so the agent takes the issue on in the folder itself; the issue still
        // advances to in_progress, it just gets closed by hand rather than by a
        // merge. Racing or looping an issue stays worktree-only.
        let gitless = require_gitless_known(&self.project_repo(&issue.project_id)?)?;
        let base =
            if gitless { "HEAD".to_string() } else { self.issue_base(&issue.project_id, base)? };
        let info = self.create_run_spec(
            NewRunSpec {
                project_id: &issue.project_id,
                prompt: &prompt,
                agent,
                model,
                base: &base,
                merge_target,
                race_id: None,
                title: Some(title),
                existing_branch: None,
                loop_config: None,
                issue_id: Some(issue.id.clone()),
                worktree: !gitless,
            },
            &mut |_| {},
        )?;
        {
            let reg = self.registry.lock().unwrap();
            self.advance_issue(&reg, &issue.id, IssueStatus::InProgress)?;
        }
        Ok(info)
    }

    /// Race several agents on one issue; every attempt links the issue.
    pub fn start_issue_race(
        &self,
        issue_id: &str,
        agents: &[String],
        models: &HashMap<String, String>,
        base: Option<&str>,
        merge_target: Option<&str>,
    ) -> Result<Vec<RunInfo>> {
        let (issue, prompt, title) = self.issue_dispatch(issue_id)?;
        let base = self.issue_base(&issue.project_id, base)?;
        let out = self.create_race_inner(
            &issue.project_id,
            &prompt,
            agents,
            models,
            &base,
            merge_target,
            Some(title),
            Some(issue.id.clone()),
        )?;
        {
            let reg = self.registry.lock().unwrap();
            self.advance_issue(&reg, &issue.id, IssueStatus::InProgress)?;
        }
        Ok(out)
    }

    /// Loop an agent on one issue until the check command passes.
    #[allow(clippy::too_many_arguments)]
    pub fn start_issue_loop(
        &self,
        issue_id: &str,
        agent: &str,
        model: Option<&str>,
        check_command: &str,
        max_attempts: u32,
        base: Option<&str>,
        merge_target: Option<&str>,
    ) -> Result<RunInfo> {
        let (issue, prompt, title) = self.issue_dispatch(issue_id)?;
        let base = self.issue_base(&issue.project_id, base)?;
        let info = self.create_loop_inner(
            &issue.project_id,
            &prompt,
            agent,
            model,
            &base,
            merge_target,
            check_command,
            max_attempts,
            Some(title),
            Some(issue.id.clone()),
        )?;
        {
            let reg = self.registry.lock().unwrap();
            self.advance_issue(&reg, &issue.id, IssueStatus::InProgress)?;
        }
        Ok(info)
    }

    // Since Phase 5 issues are files (`.agency/issues/AGE-14.md`) and the
    // registry rows are an index over them. Every mutation here is file-first:
    // write the file, then the row, both under the registry lock — a crash
    // between the two leaves the file ahead, and reconcile squares the row up
    // on the next poll. The Tauri surface and api.ts are unchanged.

    /// Root + issue-key prefix for a project's issue files.
    fn issue_root(
        &self,
        reg: &agency_core::registry::Registry,
        project_id: &str,
    ) -> Result<(std::path::PathBuf, String)> {
        let p =
            reg.get_project(project_id)?.ok_or_else(|| anyhow!("unknown project: {project_id}"))?;
        let key = p.issue_key.unwrap_or_else(|| "ISSUE".to_string());
        Ok((p.repo_path, key))
    }

    /// One-shot per project: export the SQLite issues to `.agency/issues/`,
    /// fix the repo's exclude file so those files are commit-able, and flip
    /// `issues_migrated`. Cheap (one flag read) ever after. Runs at the top
    /// of every issue-touching path, so no caller can see a pre-file project.
    fn ensure_issue_files(
        &self,
        reg: &agency_core::registry::Registry,
        project_id: &str,
    ) -> Result<()> {
        self.ensure_issues_untracked(reg, project_id);
        if reg.project_issues_migrated(project_id)? {
            return Ok(());
        }
        let (root, key) = self.issue_root(reg, project_id)?;
        let written = agency_core::issuefs::export_project(reg, project_id, &key, &root)?;
        // Best-effort: a repo whose exclude file can't be rewritten (or a
        // workspace without git) still migrates — the files are what matter.
        if let Err(e) = agency_core::worktree::ensure_agency_excludes(&root) {
            log::warn!("updating git excludes for {}: {e}", root.display());
        }
        reg.mark_issues_migrated(project_id)?;
        if written > 0 {
            log::info!("exported {written} issues to files for project {project_id}");
        }
        Ok(())
    }

    /// One-shot per project: exclude `.agency/issues/` from git and commit the
    /// removal of any issue files a previous version had tracked. Issue files
    /// are local to a checkout now, so the app's constant writes to them can't
    /// dirty the tree a merge needs clean, and agent branches can't conflict on
    /// them. Cheap ever after (one settings read).
    ///
    /// Never fails a caller: a repo that can't be migrated right now (staged
    /// changes, a merge in progress) is simply left unmarked and retried on the
    /// next issue-touching call.
    fn ensure_issues_untracked(&self, reg: &agency_core::registry::Registry, project_id: &str) {
        let flag = format!("issues_untracked:{project_id}");
        match reg.get_setting(&flag) {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(e) => {
                log::warn!("reading {flag}: {e}");
                return;
            }
        }
        let Ok((root, _)) = self.issue_root(reg, project_id) else { return };
        if let Err(e) = agency_core::worktree::ensure_agency_excludes(&root) {
            log::warn!("updating git excludes for {}: {e}", root.display());
            return;
        }
        match agency_core::worktree::untrack_issue_files(&root) {
            Ok(untracked) => {
                if untracked {
                    log::info!("stopped tracking issue files in {}", root.display());
                }
                if let Err(e) = reg.set_setting(&flag, "1") {
                    log::warn!("marking {flag}: {e}");
                }
            }
            Err(e) => log::warn!("cannot untrack issue files in {} yet: {e}", root.display()),
        }
    }

    /// Serialize an issue to its file, carrying over any unknown frontmatter
    /// keys already in the file (a newer schema's fields survive our write).
    fn write_issue_file(
        &self,
        reg: &agency_core::registry::Registry,
        issue: &agency_core::registry::Issue,
    ) -> Result<()> {
        use agency_core::issuefs;
        let (root, prefix) = self.issue_root(reg, &issue.project_id)?;
        let key = format!("{prefix}-{}", issue.seq);
        let path = issuefs::issue_path(&root, &key);
        let current = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| issuefs::parse_issue_file(&key, &t).ok());
        let extra = current.as_ref().map(|f| f.extra.clone()).unwrap_or_default();
        // The thread belongs to the file, not to the index row: an agent can
        // append a comment while the app has the issue open, and every write
        // from here (title, body, status, links) carries over what the file
        // says rather than the row's possibly older copy. Comments are written
        // only by `edit_issue_comments`, which parses the file first.
        let comments = current.map_or_else(|| issue.comments.clone(), |f| f.comments);
        let file = issuefs::IssueFile {
            key,
            seq: issue.seq,
            title: issue.title.clone(),
            body: issue.body.clone(),
            comments,
            status: issue.status,
            priority: issue.priority,
            due: issue.due.clone(),
            scheduled: issue.scheduled.clone(),
            rank: issue.rank,
            links: issue.links.clone(),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            extra,
        };
        issuefs::atomic_write(&path, &issuefs::serialize_issue_file(&file))?;
        issuefs::ensure_readme(&root)?;
        Ok(())
    }

    /// Who a comment written in the app is signed by: the repo's git user,
    /// then the OS user, and "You" for a checkout that answers neither.
    fn comment_author(root: &Path) -> String {
        agency_core::git::user_name(root)
            .or_else(|| std::env::var("USER").ok().filter(|u| !u.trim().is_empty()))
            .unwrap_or_else(|| "You".to_string())
    }

    /// Mutate an issue's comment thread, file first. Unlike every other issue
    /// write, this reads the file rather than the index row: the thread has
    /// more than one writer (an agent working the issue can append to it), and
    /// the row is only as fresh as the last reconcile. The mutation gets the
    /// parsed file and the name to sign new comments with; the row is then
    /// rewritten from what was actually written to disk.
    fn edit_issue_comments<F>(&self, id: &str, mutate: F) -> Result<agency_core::registry::Issue>
    where
        F: FnOnce(&mut Vec<agency_core::registry::IssueComment>, &str) -> Result<()>,
    {
        use agency_core::issuefs;
        let reg = self.registry.lock().unwrap();
        let row = reg.get_issue(id)?.ok_or_else(|| anyhow!("unknown issue: {id}"))?;
        self.ensure_issue_files(&reg, &row.project_id)?;
        let (root, prefix) = self.issue_root(&reg, &row.project_id)?;
        let key = format!("{prefix}-{}", row.seq);
        let path = issuefs::issue_path(&root, &key);
        // A file that is missing or unparseable falls back to the row, which is
        // the same recovery `write_issue_file` performs: better a rewritten
        // file than a comment that cannot be posted at all.
        let mut file = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| issuefs::parse_issue_file(&key, &t).ok())
            .unwrap_or_else(|| issuefs::IssueFile {
                key: key.clone(),
                seq: row.seq,
                title: row.title.clone(),
                body: row.body.clone(),
                comments: row.comments.clone(),
                status: row.status,
                priority: row.priority,
                due: row.due.clone(),
                scheduled: row.scheduled.clone(),
                rank: row.rank,
                links: row.links.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
                extra: Vec::new(),
            });
        mutate(&mut file.comments, &Self::comment_author(&root))?;
        file.updated_at = now_secs();
        issuefs::atomic_write(&path, &issuefs::serialize_issue_file(&file))?;
        issuefs::ensure_readme(&root)?;
        let next = issuefs::issue_from_file(
            &file,
            row.id,
            &row.project_id,
            file.created_at,
            file.updated_at,
        );
        reg.upsert_issue_row(&next)
    }

    /// Post a comment, signed by whoever this checkout says is writing.
    pub fn add_issue_comment(&self, id: &str, body: &str) -> Result<agency_core::registry::Issue> {
        let text = body.trim().to_string();
        if text.is_empty() {
            bail!("a comment needs some text");
        }
        self.edit_issue_comments(id, move |comments, author| {
            // A comment is addressed by its timestamp, so two posted inside the
            // same second are nudged apart rather than made ambiguous.
            let mut at = now_secs();
            while comments.iter().any(|c| c.created_at == at) {
                at += 1;
            }
            comments.push(agency_core::registry::IssueComment {
                author: author.to_string(),
                created_at: at,
                body: text,
            });
            Ok(())
        })
    }

    pub fn update_issue_comment(
        &self,
        id: &str,
        created_at: i64,
        body: &str,
    ) -> Result<agency_core::registry::Issue> {
        let text = body.trim().to_string();
        if text.is_empty() {
            bail!("a comment needs some text");
        }
        self.edit_issue_comments(id, move |comments, _| {
            let c = comments
                .iter_mut()
                .find(|c| c.created_at == created_at)
                .ok_or_else(|| anyhow!("that comment is no longer there"))?;
            c.body = text;
            Ok(())
        })
    }

    pub fn delete_issue_comment(
        &self,
        id: &str,
        created_at: i64,
    ) -> Result<agency_core::registry::Issue> {
        self.edit_issue_comments(id, move |comments, _| {
            let before = comments.len();
            comments.retain(|c| c.created_at != created_at);
            if comments.len() == before {
                bail!("that comment is no longer there");
            }
            Ok(())
        })
    }

    /// Automation path: move the issue strictly forward (see
    /// `IssueStatus::advances_to`), file-first. Returns whether it changed.
    fn advance_issue(
        &self,
        reg: &agency_core::registry::Registry,
        issue_id: &str,
        status: IssueStatus,
    ) -> Result<bool> {
        let Some(current) = reg.get_issue(issue_id)? else { return Ok(false) };
        if !current.status.advances_to(status) {
            return Ok(false);
        }
        self.ensure_issue_files(reg, &current.project_id)?;
        let mut next = current;
        next.status = status;
        next.updated_at = now_secs();
        self.write_issue_file(reg, &next)?;
        reg.upsert_issue_row(&next)?;
        Ok(true)
    }

    pub fn list_issues(&self, project_id: &str) -> Result<Vec<agency_core::registry::Issue>> {
        let reg = self.registry.lock().unwrap();
        self.ensure_issue_files(&reg, project_id)?;
        let (root, key) = self.issue_root(&reg, project_id)?;
        // Reconcile only when the files' stat signature moved.
        let sig = agency_core::issuefs::scan_issue_stats(&root)?
            .iter()
            .map(|s| format!("{}:{}:{}", s.path, s.mtime_ms, s.size))
            .collect::<Vec<_>>()
            .join("|");
        let mut sigs = self.issue_sigs.lock().unwrap();
        if sigs.get(project_id) != Some(&sig) {
            let summary = agency_core::issuefs::reconcile(&reg, project_id, &key, &root)?;
            if summary.imported + summary.updated + summary.dropped > 0 {
                log::debug!(
                    "issues reconciled for {project_id}: +{} ~{} -{}",
                    summary.imported,
                    summary.updated,
                    summary.dropped
                );
            }
            sigs.insert(project_id.to_string(), sig);
        }
        reg.list_issues(project_id)
    }

    pub fn create_issue(
        &self,
        project_id: &str,
        title: &str,
        body: &str,
        status: IssueStatus,
    ) -> Result<agency_core::registry::Issue> {
        if title.trim().is_empty() {
            bail!("an issue needs a title");
        }
        let reg = self.registry.lock().unwrap();
        self.ensure_issue_files(&reg, project_id)?;
        // The counter only learns about hand-authored files at reconcile time
        // (the list_issues poll), so a number it hands out may already be
        // taken on disk — skip past those files rather than clobbering them.
        let (root, prefix) = self.issue_root(&reg, project_id)?;
        let seq = loop {
            let seq = reg.alloc_issue_seq(project_id)?;
            if !agency_core::issuefs::issue_path(&root, &format!("{prefix}-{seq}")).exists() {
                break seq;
            }
        };
        let now = now_secs();
        let issue = agency_core::registry::Issue {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            seq,
            title: normalize_title(title),
            body: body.to_string(),
            status,
            priority: 0,
            due: None,
            scheduled: None,
            rank: None,
            links: Vec::new(),
            comments: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        self.write_issue_file(&reg, &issue)?;
        reg.upsert_issue_row(&issue)
    }

    pub fn update_issue(
        &self,
        id: &str,
        patch: &agency_core::registry::IssuePatch,
    ) -> Result<agency_core::registry::Issue> {
        if patch.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            bail!("an issue needs a title");
        }
        let reg = self.registry.lock().unwrap();
        let mut next = reg.get_issue(id)?.ok_or_else(|| anyhow!("unknown issue: {id}"))?;
        self.ensure_issue_files(&reg, &next.project_id)?;
        if let Some(t) = &patch.title {
            next.title = normalize_title(t);
        }
        if let Some(b) = &patch.body {
            next.body = b.clone();
        }
        if let Some(s) = patch.status {
            next.status = s;
        }
        // Validate what the file parser would reject — the API must never
        // write a file that reconcile then skips as corrupt.
        if let Some(p) = patch.priority {
            if p > 4 {
                bail!("invalid priority: {p}");
            }
            next.priority = p;
        }
        if let Some(d) = &patch.due {
            next.due = d.as_deref().map(agency_core::issuefs::parse_date).transpose()?;
        }
        if let Some(d) = &patch.scheduled {
            next.scheduled = d.as_deref().map(agency_core::issuefs::parse_date).transpose()?;
        }
        if let Some(r) = patch.rank {
            if r.is_some_and(|r| !r.is_finite()) {
                bail!("invalid rank");
            }
            next.rank = r;
        }
        if let Some(links) = &patch.links {
            // Through the file parser's own normalization (upper-case, deduped,
            // self-link dropped), so what the API accepts is exactly what the
            // frontmatter can hold.
            let (_, prefix) = self.issue_root(&reg, &next.project_id)?;
            let own = format!("{prefix}-{}", next.seq);
            next.links = agency_core::issuefs::parse_links(&links.join(","), &own)?;
        }
        next.updated_at = now_secs();
        self.write_issue_file(&reg, &next)?;
        reg.upsert_issue_row(&next)
    }

    pub fn delete_issue(&self, id: &str) -> Result<()> {
        let reg = self.registry.lock().unwrap();
        let Some(issue) = reg.get_issue(id)? else { return Ok(()) };
        self.ensure_issue_files(&reg, &issue.project_id)?;
        let (root, prefix) = self.issue_root(&reg, &issue.project_id)?;
        let path = agency_core::issuefs::issue_path(&root, &format!("{prefix}-{}", issue.seq));
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| anyhow!("cannot delete {}: {e}", path.display()))?;
        }
        reg.delete_issue(id)
    }

    /// The abandonment rule: when a linked run is discarded or archived and
    /// it was the issue's last active run, the issue falls back to `todo`
    /// (only from in_progress/in_review — manual done/cancelled stay put).
    fn maybe_rollback_issue(&self, issue_id: &str) {
        let reg = self.registry.lock().unwrap();
        match reg.runs_for_issue(issue_id) {
            Ok(runs) if runs.is_empty() => {
                if let Err(e) = self.rollback_issue_to_todo(&reg, issue_id) {
                    log::warn!("rolling back issue {issue_id}: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("checking linked runs of issue {issue_id}: {e}"),
        }
    }

    /// File-first counterpart of the old registry-only rollback.
    fn rollback_issue_to_todo(
        &self,
        reg: &agency_core::registry::Registry,
        issue_id: &str,
    ) -> Result<bool> {
        let Some(current) = reg.get_issue(issue_id)? else { return Ok(false) };
        if !matches!(current.status, IssueStatus::InProgress | IssueStatus::InReview) {
            return Ok(false);
        }
        self.ensure_issue_files(reg, &current.project_id)?;
        let mut next = current;
        next.status = IssueStatus::Todo;
        next.updated_at = now_secs();
        self.write_issue_file(reg, &next)?;
        reg.upsert_issue_row(&next)?;
        Ok(true)
    }

    /// Spawn a workspace for a GitHub issue: the issue becomes the run's
    /// prompt (delivered to the agent at launch) and its title.
    pub fn create_run_from_issue(
        &self,
        project_id: &str,
        number: u64,
        agent: &str,
        model: Option<&str>,
    ) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let issue = agency_core::gh::GhCli::default().view_issue(&repo, number)?;
        let base = agency_core::merge::detect_base(&repo)?;
        let prompt = format!(
            "Work on GitHub issue #{number}: {title}\n\n{body}\n\nIssue link: {url}",
            title = issue.title,
            body = issue.body,
            url = issue.url,
        );
        self.create_run_spec(
            NewRunSpec {
                project_id,
                prompt: &prompt,
                agent,
                model,
                base: &base,
                merge_target: None,
                race_id: None,
                title: Some(format!("#{number} {}", issue.title)),
                existing_branch: None,
                loop_config: None,
                issue_id: None,
                worktree: true,
            },
            &mut |_| {},
        )
    }

    /// Check an existing PR's head branch out into a workspace for review.
    /// The local branch is fast-forwarded from origin first; a diverged local
    /// branch fails loudly rather than being clobbered.
    pub fn create_run_from_pr(
        &self,
        project_id: &str,
        number: u64,
        agent: &str,
        model: Option<&str>,
    ) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let pr = agency_core::gh::GhCli::default()
            .view_pr_by_number(&repo, number)?
            .ok_or_else(|| anyhow!("PR #{number} not found"))?;
        if pr.head_ref_name.is_empty() {
            bail!("PR #{number} has no local head branch (cross-fork PRs aren't supported yet)");
        }
        // The PR's branch may already be an active run here (e.g. the PR was
        // opened from an Agency agent). git only allows a branch to be checked
        // out in one worktree, so re-checking it out into a review worktree
        // fails — reuse the existing run instead of erroring.
        if let Some(run) = self
            .registry
            .lock()
            .unwrap()
            .list_runs(project_id)?
            .into_iter()
            .find(|r| r.branch == pr.head_ref_name)
        {
            return Ok(self.run_info(&run));
        }
        agency_core::git::fetch_branch(&repo, &pr.head_ref_name)?;
        let prompt = format!(
            "Review GitHub pull request #{number}: {title}. Its branch is checked out in this workspace. PR link: {url}",
            title = pr.title,
            url = pr.url,
        );
        self.create_run_spec(
            NewRunSpec {
                project_id,
                prompt: &prompt,
                agent,
                model,
                base: &pr.base_ref_name,
                merge_target: Some(&pr.base_ref_name),
                race_id: None,
                title: Some(format!("PR #{number} {}", pr.title)),
                existing_branch: Some(pr.head_ref_name.clone()),
                loop_config: None,
                issue_id: None,
                worktree: true,
            },
            &mut |_| {},
        )
    }

    /// Start an agent that reviews an existing PR and then sticks around to fix
    /// what it found. The agent works in the PR's head branch, so its fixes
    /// commit and push straight onto the PR.
    ///
    /// When that branch is already checked out by another run (the usual case
    /// for a PR an Agency agent opened from the Approve window), the review runs
    /// as an extra agent tab inside that run: git allows a branch in only one
    /// worktree, and fixes have to land on that branch anyway. The tab is still
    /// a fresh agent with no memory of writing the code.
    pub fn create_pr_review_run(
        &self,
        project_id: &str,
        number: u64,
        agent: &str,
        model: Option<&str>,
        post_comments: bool,
    ) -> Result<PrReviewRun> {
        let repo = self.project_repo(project_id)?;
        let pr = agency_core::gh::GhCli::default()
            .view_pr_by_number(&repo, number)?
            .ok_or_else(|| anyhow!("PR #{number} not found"))?;
        if pr.head_ref_name.is_empty() {
            bail!("PR #{number} has no local head branch (cross-fork PRs aren't supported yet)");
        }
        let prompt = pr_review_prompt(number, &pr.title, &pr.url, &pr.base_ref_name, post_comments);
        // Only an unarchived agent run can host an extra tab; anything else
        // holding the branch falls through and git reports the conflict.
        let holder = self
            .registry
            .lock()
            .unwrap()
            .list_runs(project_id)?
            .into_iter()
            .find(|r| r.branch == pr.head_ref_name && r.kind == "agent");
        if let Some(run) = holder {
            let session = self.start_run_session(&run.id, Some(agent), &prompt)?;
            return Ok(PrReviewRun { run: self.run_info(&run), session_id: Some(session.id) });
        }
        agency_core::git::fetch_branch(&repo, &pr.head_ref_name)?;
        let run = self.create_run_spec(
            NewRunSpec {
                project_id,
                prompt: &prompt,
                agent,
                model,
                base: &pr.base_ref_name,
                merge_target: Some(&pr.base_ref_name),
                race_id: None,
                title: Some(format!("Review PR #{number}")),
                existing_branch: Some(pr.head_ref_name.clone()),
                loop_config: None,
                issue_id: None,
                worktree: true,
            },
            &mut |_| {},
        )?;
        Ok(PrReviewRun { run, session_id: None })
    }

    /// Store the first prompt the user typed into the agent terminal as the
    /// run's prompt (runs are created promptless). First capture wins.
    pub fn store_run_prompt(&self, id: &str, prompt: &str) -> Result<()> {
        self.registry.lock().unwrap().set_run_prompt_if_empty(id, prompt)
    }

    // ── MCP servers ────────────────────────────────────────────────────────────

    /// App-global MCP servers, configured in Settings.
    pub fn list_mcp_servers(&self) -> Result<Vec<agency_core::mcp::McpServer>> {
        let raw = self.registry.lock().unwrap().get_setting(SETTING_MCP)?;
        let mut servers: Vec<agency_core::mcp::McpServer> =
            raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        // Fold the pre-per-agent `userScope` flag into `userScopeAgents` so
        // callers never have to know the legacy shape.
        for s in &mut servers {
            s.normalize();
        }
        Ok(servers)
    }

    /// Replace the app-global MCP server list. Every entry must validate.
    pub fn save_mcp_servers(&self, servers: &[agency_core::mcp::McpServer]) -> Result<()> {
        for s in servers {
            s.validate()?;
        }
        let json = serde_json::to_string(servers)?;
        self.registry.lock().unwrap().set_setting(SETTING_MCP, &json)
    }

    /// The effective knowledge-graph config for a project, plus whether the
    /// resolved serve/build tooling is actually on PATH (the feature silently
    /// no-ops when it isn't, so the UI surfaces it as a warning).
    pub fn knowledge_config(&self, project_id: &str) -> Result<KnowledgeConfigDto> {
        let repo = self.project_repo(project_id)?;
        let k = agency_core::config::load(&repo).knowledge;
        let serve_default = agency_core::config::default_serve_command(&repo);
        let build_default = agency_core::config::default_build_command().to_string();
        let serve_effective = k.serve_command.clone().unwrap_or_else(|| serve_default.clone());
        let build_effective = k.build_command.clone().unwrap_or_else(|| build_default.clone());
        let graph = agency_core::config::graph_path(&repo);
        let build_state = self.kg_builds.lock().unwrap().get(&repo).cloned().unwrap_or_default();
        Ok(KnowledgeConfigDto {
            graph: k.graph,
            serve_installed: first_token_on_path(&serve_effective),
            build_installed: first_token_on_path(&build_effective),
            serve_command: k.serve_command,
            build_command: k.build_command,
            serve_default,
            build_default,
            graph_built: graph.is_file(),
            graph_path: graph.display().to_string(),
            building: build_state.running,
            last_build_error: build_state.error,
            install_command: graphify_install_script(),
        })
    }

    /// Persist a project's knowledge-graph config into its (gitignored) local
    /// override file. Empty command strings clear the override (runtime default
    /// applies) rather than persisting a blank command.
    pub fn save_knowledge_config(
        &self,
        project_id: &str,
        graph: bool,
        serve_command: Option<String>,
        build_command: Option<String>,
    ) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        let clean = |s: Option<String>| s.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
        let k = agency_core::config::KnowledgeConfig {
            graph,
            serve_command: clean(serve_command),
            build_command: clean(build_command),
        };
        agency_core::config::save_knowledge(&repo, &k)?;
        // Enabling the graph is a request for a graph. Nothing else builds one
        // until a merge lands, so without this the feature stays inert: the
        // serve command would point at a graph.json that never appears.
        if graph && !agency_core::config::graph_path(&repo).is_file() {
            if let Err(e) = self.start_knowledge_build(&repo) {
                log::info!("not building knowledge graph for {}: {e}", repo.display());
            }
        }
        Ok(())
    }

    /// Build (or rebuild) a project's knowledge graph now, from the UI.
    pub fn build_knowledge_graph(&self, project_id: &str) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        self.start_knowledge_build(&repo)
    }

    /// Install the graphify tooling in a visible Agency terminal, the same way
    /// a missing agent CLI is installed: the user watches it run and can answer
    /// anything it asks, rather than Agency mutating their machine silently.
    /// Returns the terminal run to jump into.
    pub fn install_knowledge_tooling(&self, project_id: &str) -> Result<RunInfo> {
        self.create_install_terminal(project_id, "graphify", &graphify_install_script())
    }

    /// Spawn the project's build command in the primary checkout, tracking it in
    /// `kg_builds` so the UI can show progress and failures. Returns an error
    /// (without spawning) when the tooling is missing or a build is already
    /// running for this repo — both are states the caller reports, not retries.
    fn start_knowledge_build(&self, repo: &Path) -> Result<()> {
        let config = agency_core::config::load(repo);
        let build = config
            .knowledge
            .build_command
            .clone()
            .unwrap_or_else(|| agency_core::config::default_build_command().to_string());
        let cmd = build
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("the build command is empty"))?
            .to_string();
        if !command_on_path(&cmd) {
            return Err(anyhow!(
                "'{cmd}' is not installed. Install the graphify tooling and try again."
            ));
        }
        {
            let mut builds = self.kg_builds.lock().unwrap();
            let entry = builds.entry(repo.to_path_buf()).or_default();
            if entry.running {
                return Err(anyhow!("a graph build is already running for this project"));
            }
            entry.running = true;
            entry.error = None;
        }
        log::info!("building knowledge graph in {}: {build}", repo.display());
        let repo = repo.to_path_buf();
        let builds_handle = self.kg_builds.clone();
        std::thread::spawn(move || {
            // A login shell so the build resolves the same tooling the user's
            // terminal does, and captured output so a failure has a reason to
            // show instead of a bare exit code.
            let out = std::process::Command::new("sh")
                .args(["-lc", &build])
                .current_dir(&repo)
                .stdin(std::process::Stdio::null())
                .output();
            let error = match out {
                Err(e) => Some(format!("couldn't run the build command: {e}")),
                Ok(o) if o.status.success() => None,
                Ok(o) => Some(match failure_tail(&o.stderr, &o.stdout) {
                    Some(tail) => tail,
                    None => format!("the build command exited with {}", o.status),
                }),
            };
            if let Some(e) = &error {
                log::warn!("knowledge graph build failed in {}: {e}", repo.display());
            }
            let mut builds = builds_handle.lock().unwrap();
            let entry = builds.entry(repo).or_default();
            entry.running = false;
            entry.error = error;
        });
        Ok(())
    }

    /// The files-copied-into-worktrees config for a project: the configured
    /// `[files] copy` list plus the auto-detected untracked root `.env*` files.
    pub fn files_config(&self, project_id: &str) -> Result<FilesConfigDto> {
        let repo = self.project_repo(project_id)?;
        let copy = agency_core::config::load(&repo).files.copy;
        let detected_env = WorktreeManager::new(repo).default_env_files();
        Ok(FilesConfigDto { copy, detected_env })
    }

    /// Persist a project's worktree copy list into its (gitignored) local
    /// override file. Entries are trimmed and blanks dropped by `save_files`.
    pub fn save_files_config(&self, project_id: &str, copy: Vec<String>) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        agency_core::config::save_files(&repo, &agency_core::config::FilesConfig { copy })?;
        Ok(())
    }

    /// The full MCP server list for a workspace: app-global servers, overlaid
    /// by the project's `[mcp.servers.*]`, plus the graphify knowledge-graph
    /// server when the project opted in and the tooling is installed.
    ///
    /// graphify's MCP server has no console-script entry point — it runs as
    /// `python -m graphify.serve <graph.json>` inside the uv tool venv, so the
    /// default goes through `uv tool run --from graphifyy`. The graph.json is
    /// addressed absolutely in the PRIMARY repo (worktrees don't carry the
    /// untracked graphify-out/, and the post-merge rebuild runs there).
    fn merged_mcp_servers(
        &self,
        repo: &Path,
        config: &agency_core::config::AgencyConfig,
    ) -> Vec<agency_core::mcp::McpServer> {
        let global = self.list_mcp_servers().unwrap_or_default();
        let project = agency_core::mcp::from_config(&config.mcp);
        let mut auto = Vec::new();
        if config.knowledge.graph {
            match graphify_server(repo, &config.knowledge, |p| p.is_file(), command_on_path) {
                Ok(server) => auto.push(server),
                Err(reason) => {
                    log::warn!("knowledge graph enabled but {reason}; skipping MCP injection")
                }
            }
        }
        agency_core::mcp::merge(&[global, project, auto])
    }

    /// Emit MCP config into a workspace in the agent's native format.
    /// Best-effort: a bad server entry must not block the run.
    fn emit_mcp(
        &self,
        agent: &str,
        repo: &Path,
        worktree: &Path,
        config: &agency_core::config::AgencyConfig,
    ) {
        let servers = self.merged_mcp_servers(repo, config);
        if servers.is_empty() {
            return;
        }
        match agency_core::mcp::emit_for_agent(agent, worktree, &servers) {
            Err(e) => {
                log::warn!("emitting MCP config for {agent} into {}: {e}", worktree.display());
            }
            Ok(false) => {
                log::info!(
                    "MCP servers not emitted for {agent}: no per-workspace config format \
                     (or all entries user-scope/invalid)"
                );
            }
            Ok(true) => {}
        }
    }

    /// After a clean merge, rebuild the project's knowledge graph in the
    /// background so the next agent workspace starts with a fresh graph.
    fn maybe_rebuild_knowledge_graph(&self, repo: &Path) {
        if !agency_core::config::load(repo).knowledge.graph {
            return;
        }
        if let Err(e) = self.start_knowledge_build(repo) {
            log::warn!("not rebuilding knowledge graph after merge: {e}");
        }
    }

    /// Best-effort `git fetch` for a project's `origin`, so ahead/behind stops
    /// going stale. Returns `Ok(false)` when the project has no remote (nothing
    /// to fetch). Runs no git under any lock — the network call happens after
    /// the repo path is resolved and the registry lock released.
    ///
    /// Unthrottled: everything that fetches on its own goes through
    /// [`fetch_project_if_due`] instead, so the sweep and the UI share one
    /// cadence per origin.
    pub fn fetch_project(&self, project_id: &str) -> Result<bool> {
        let repo = self.project_repo(project_id)?;
        if !agency_core::git::has_origin(&repo) {
            return Ok(false);
        }
        agency_core::git::fetch(&repo)?;
        Ok(true)
    }

    /// [`fetch_project`], but only if this project's origin hasn't been
    /// contacted in the last `min_age` and isn't being fetched right now.
    /// Returns whether a fetch actually ran.
    ///
    /// Every automatic fetch shares this bookkeeping, which is what lets the
    /// slow background sweep and the UI's on-open/on-focus requests coexist:
    /// two triggers a second apart cost one fetch, and a remote that is offline
    /// or unauthenticated backs off for both at once instead of being retried
    /// on every panel open. Holds no lock across the network call.
    pub fn fetch_project_if_due(&self, project_id: &str, min_age: Duration) -> Result<bool> {
        let start = Instant::now();
        {
            let mut sched = self.fetches.lock().unwrap();
            let entry = sched.entry(project_id.to_string()).or_default();
            let too_soon = entry.last_attempt.is_some_and(|t| start.duration_since(t) < min_age);
            if entry.in_flight || too_soon || entry.retry_after.is_some_and(|t| start < t) {
                return Ok(false);
            }
            entry.in_flight = true;
            entry.last_attempt = Some(start);
        }

        let result = self.fetch_project(project_id);

        let mut sched = self.fetches.lock().unwrap();
        // The entry can be gone if the project was removed mid-fetch; re-inserting
        // it would leak, and there is nothing left to schedule for.
        if let Some(entry) = sched.get_mut(project_id) {
            entry.in_flight = false;
            match result {
                Ok(_) => {
                    entry.retry_after = None;
                    entry.backoff = FETCH_BACKOFF_BASE;
                }
                Err(_) => {
                    entry.retry_after = Some(Instant::now() + entry.backoff);
                    entry.backoff = (entry.backoff * 2).min(FETCH_BACKOFF_MAX);
                }
            }
        }
        result
    }

    /// One pass of the background fetch sweep: fetch every project whose origin
    /// hasn't been contacted in `min_age`, and forget the bookkeeping for
    /// projects that no longer exist. Failures are logged rather than
    /// propagated — one unreachable remote must not stop the others, and
    /// [`fetch_project_if_due`] has already backed that project off.
    pub fn sweep_project_fetches(&self, min_age: Duration) {
        let projects = self.list_projects().unwrap_or_default();
        let live: HashSet<&str> = projects.iter().map(|p| p.id.as_str()).collect();
        self.fetches.lock().unwrap().retain(|id, _| live.contains(id.as_str()));
        for p in &projects {
            if let Err(e) = self.fetch_project_if_due(&p.id, min_age) {
                log::warn!("background fetch for project {}: {e}", p.id);
            }
        }
    }

    /// The project a git token belongs to — `project:<id>` names it outright,
    /// any other token is a run id. Used by the auto-fetch command, which is
    /// handed whatever token the Source Control panel is showing.
    pub fn project_of(&self, token: &str) -> Result<String> {
        if let Some(pid) = token.strip_prefix("project:") {
            return Ok(pid.to_string());
        }
        Ok(self.run_record(token)?.project_id)
    }

    pub fn list_project_branches(
        &self,
        project_id: &str,
    ) -> Result<agency_core::git::ProjectBranches> {
        let repo = self.project_repo(project_id)?;
        agency_core::git::list_branches(&repo)
    }

    /// Create a standalone shell terminal session in the project repo root.
    /// Unlike `create_run` it has no worktree, branch, or agent profile — it just
    /// runs the user's login shell, reusing the daemon attach/resize pipeline.
    pub fn create_terminal(&self, project_id: &str) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let id = new_task_id("terminal");
        let shell = login_shell();
        // Login shell so the user's prompt/profile loads.
        let args = vec!["-l".to_string()];
        pty_debug(&format!("create_terminal id={id} shell={shell} args={args:?}"));
        self.term.read().unwrap().start_session(
            &session_name(&id),
            &repo,
            &shell,
            &args,
            &[],
            220,
            50,
        )?;

        let run = agency_core::registry::Run {
            id: id.clone(),
            project_id: project_id.to_string(),
            agent: "terminal".to_string(),
            prompt: String::new(),
            base: String::new(),
            branch: String::new(),
            created_at: now_secs(),
            port_base: None,
            archived_at: None,
            title: Some("terminal".to_string()),
            kind: "terminal".to_string(),
            merge_target: None,
            race_id: None,
            loop_config: None,
            loop_state: None,
            issue_id: None,
            worktree: false,
            model: None,
        };
        self.registry.lock().unwrap().insert_run(&run)?;
        Ok(self.run_info(&run))
    }

    /// Every live run in the project, as the board polls it (every 1.5s). One
    /// session listing serves the whole list: per-run daemon calls would scale
    /// with the size of the board on every tick.
    pub fn list_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_runs(project_id)?;
        let live = self.term.read().unwrap().list().unwrap_or_default();
        Ok(runs.iter().map(|r| self.run_info_from(r, &live)).collect())
    }

    /// Whether any run script is live in a workspace — for the project's own
    /// checkout (`project:<id>`), which has no `RunInfo` to carry the flag.
    pub fn run_scripts_live(&self, target: &str) -> Result<bool> {
        let live = self.term.read().unwrap().list().unwrap_or_default();
        Ok(any_run_script_live(target, &live))
    }

    /// Every non-archived run across all projects, grouped by project in
    /// project order, for the tray menu. Cheap by design: only the session
    /// status is fetched per run (no diff stats, no pane capture).
    pub fn tray_runs(&self) -> Result<Vec<crate::tray::TrayRun>> {
        let projects = self.registry.lock().unwrap().list_projects()?;
        let mut out = Vec::new();
        for proj in projects {
            let runs = self.registry.lock().unwrap().list_runs(&proj.id)?;
            for run in runs {
                let status = self
                    .term
                    .read()
                    .unwrap()
                    .status(&session_name(&run.id))
                    .unwrap_or(SessionStatus::Gone);
                let name = run.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| {
                    if run.prompt.is_empty() {
                        run.branch.clone()
                    } else {
                        run.prompt.clone()
                    }
                });
                out.push(crate::tray::TrayRun {
                    run_id: run.id,
                    project_id: proj.id.clone(),
                    project: proj.name.clone(),
                    label: format!("{}: {}", run.agent, name),
                    running: matches!(status, SessionStatus::Running),
                });
            }
        }
        Ok(out)
    }

    /// Whether the agent profile's command resolves to something executable —
    /// an explicit path, or a name found on PATH (which pathenv::repair() has
    /// already fixed up for Finder launches). Falls back to the catalog command
    /// when the profile isn't enabled yet (onboarding / catalog install checks).
    pub fn agent_installed(&self, agent: &str) -> Result<bool> {
        let command = {
            let reg = self.registry.lock().unwrap();
            if let Some(profile) = reg.get_profile(agent)? {
                profile.command
            } else if let Some(entry) = crate::agent_catalog::find(agent) {
                entry.command.to_string()
            } else {
                bail!("unknown agent profile: {agent}");
            }
        };
        Ok(command_on_path(&command))
    }

    /// Spawn a terminal session in the project repo that first runs `command`
    /// (an agent install line), then execs the user's login shell so they can
    /// verify the result — and immediately use the freshly installed CLI.
    pub fn create_install_terminal(
        &self,
        project_id: &str,
        agent: &str,
        command: &str,
    ) -> Result<RunInfo> {
        let script = format!("{command}\nexec \"$SHELL\" -l");
        self.spawn_terminal(project_id, &format!("install {agent}"), script)
    }

    /// Register a remote MCP `server` with `agent`'s own CLI at that CLI's user
    /// scope, then open a terminal so the user can complete the interactive OAuth
    /// handshake (which needs a browser Agency can't drive headlessly). A
    /// user-scope session persists across every worktree, so the agent is recorded
    /// in the server's `user_scope_agents` and Agency stops emitting a
    /// project-scoped copy *for that agent only* — every other agent keeps
    /// getting the server written into its workspace config.
    ///
    /// The `mcp add` command is run synchronously and its exit status checked
    /// *before* recording: a failed registration (e.g. the CLI isn't installed)
    /// must not silently mark the server, which would remove it from that agent's
    /// worktrees while never actually registering it.
    pub fn authenticate_mcp_server(
        &self,
        project_id: &str,
        agent: &str,
        name: &str,
    ) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let mut servers = self.list_mcp_servers()?;
        let idx = servers
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| anyhow!("unknown MCP server: {name}"))?;
        let server = servers[idx].clone();
        // Builds the argv and rejects unsupported agents / stdio servers.
        let (command, args) = agency_core::mcp::auth_argv(agent, &server)?;

        // Register unless it already is — `mcp add` errors on a duplicate name,
        // and re-recording an already-recorded agent is a no-op.
        if !server.is_user_scope_for(agent) {
            // Pass argv directly (no shell) so user-supplied values need no quoting.
            let out = std::process::Command::new(&command)
                .args(&args)
                .current_dir(&repo)
                .output()
                .map_err(|e| {
                    anyhow!("running `{command} mcp add` (is the {command} CLI installed and on PATH?): {e}")
                })?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                bail!("`{command} mcp add` failed: {}", stderr.trim());
            }
            // Registration succeeded — now it's safe to record.
            servers[idx].user_scope_agents.push(agent.to_string());
            self.save_mcp_servers(&servers)?;
        }

        // Drop the user into a terminal to finish the interactive OAuth step.
        let hint = format!(
            "echo; echo 'Registered \"{name}\" with {agent} at user scope. To finish OAuth: run  {command}  then  /mcp  and choose Authenticate.'; echo"
        );
        let script = format!("{hint}\nexec \"$SHELL\" -l");
        self.spawn_terminal(project_id, &format!("authenticate {name}"), script)
    }

    /// Drop `agent` from a server's `user_scope_agents` so Agency resumes emitting
    /// it into that agent's per-worktree config. The recovery path when a
    /// registration failed (or the user wants Agency to manage the server again);
    /// the agent's own user-scope registration, if any, is left in place — remove
    /// it with `<agent> mcp remove`.
    pub fn deauthenticate_mcp_server(&self, agent: &str, name: &str) -> Result<()> {
        let mut servers = self.list_mcp_servers()?;
        let s = servers
            .iter_mut()
            .find(|s| s.name == name)
            .ok_or_else(|| anyhow!("unknown MCP server: {name}"))?;
        if s.is_user_scope_for(agent) {
            s.user_scope_agents.retain(|a| a != agent);
            self.save_mcp_servers(&servers)?;
        }
        Ok(())
    }

    /// Import MCP servers from a standard / VS Code `mcp.json` document, merging
    /// them into the app-global list (imported entries win on name conflict).
    /// Returns the resulting full list plus how many servers were imported, so
    /// the UI can confirm the count (entries with neither command nor url are
    /// silently skipped by the parser).
    pub fn import_mcp_json(&self, text: &str) -> Result<McpImportResult> {
        let imported = agency_core::mcp::import_json(text)?;
        if imported.is_empty() {
            bail!("no MCP servers found in that file");
        }
        let count = imported.len();
        let existing = self.list_mcp_servers()?;
        let servers = agency_core::mcp::merge(&[existing, imported]);
        self.save_mcp_servers(&servers)?;
        Ok(McpImportResult { servers, imported: count })
    }

    /// Spawn a `terminal`-kind run whose shell runs `script` (via `$SHELL -lc`).
    /// Shared by the install and MCP-authenticate flows.
    fn spawn_terminal(&self, project_id: &str, title: &str, script: String) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let id = new_task_id(title);
        let shell = login_shell();
        // Login shell (-l) so the user's profile (PATH etc.) is loaded first.
        let args = vec!["-lc".to_string(), script];
        let env = vec![("SHELL".to_string(), shell.clone())];
        self.term.read().unwrap().start_session(
            &session_name(&id),
            &repo,
            &shell,
            &args,
            &env,
            220,
            50,
        )?;

        let run = agency_core::registry::Run {
            id: id.clone(),
            project_id: project_id.to_string(),
            agent: "terminal".to_string(),
            prompt: String::new(),
            base: String::new(),
            branch: String::new(),
            created_at: now_secs(),
            port_base: None,
            archived_at: None,
            title: Some(title.to_string()),
            kind: "terminal".to_string(),
            merge_target: None,
            race_id: None,
            loop_config: None,
            loop_state: None,
            issue_id: None,
            worktree: false,
            model: None,
        };
        self.registry.lock().unwrap().insert_run(&run)?;
        Ok(self.run_info(&run))
    }

    pub fn run_status(&self, id: &str) -> Result<SessionStatus> {
        Ok(self.term.read().unwrap().status(&session_name(id)).unwrap_or(SessionStatus::Gone))
    }

    pub fn attach_run<F>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<()>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        pty_debug(&format!("attach_run id={id}"));
        let sub = self.term.read().unwrap().subscribe(&session_name(id), cols, rows, on_output)?;
        self.attaches.lock().unwrap().insert(id.to_string(), sub);
        Ok(())
    }

    pub fn detach_run(&self, id: &str) {
        pty_debug(&format!("detach_run id={id}"));
        // Dropping the Subscription sends Unsubscribe to the daemon; the session
        // itself keeps running server-side, so navigating away/back no longer
        // risks taking the shell down with the client.
        self.attaches.lock().unwrap().remove(id);
    }

    pub fn run_input(&self, id: &str, data: &[u8]) -> Result<()> {
        pty_debug(&format!("run_input id={id} len={} {}", data.len(), fmt_bytes(data)));
        self.term.read().unwrap().input(&session_name(id), data)?;
        // Arm the turn-finished notification only on Enter — the signal that the
        // user actually submitted a turn. This handler also receives xterm
        // mouse-tracking and focus escape sequences (hovering, scrolling, or
        // clicking the pane), and arming on those made the notification fire at
        // seemingly random times for runs the user never prompted.
        if data.contains(&b'\r') || data.contains(&b'\n') {
            self.input_seen.lock().unwrap().insert(id.to_string());
            self.prompted.lock().unwrap().insert(id.to_string());
        }
        Ok(())
    }

    /// Whether the run has unconsumed user input for idle-notification gating.
    pub fn has_input_pending(&self, id: &str) -> bool {
        self.input_seen.lock().unwrap().contains(id)
    }

    /// Consume the idle gate after a "waiting for input" notification fires so the
    /// run stays quiet until the user drives another turn.
    pub fn clear_input_seen(&self, id: &str) {
        self.input_seen.lock().unwrap().remove(id);
    }

    /// Advance a run's busy/idle state; called by the notifier tick with its
    /// pane-changed observation.
    pub fn update_activity(&self, id: &str, pane_changed: bool, now_ms: i64) {
        let mut map = self.activity.lock().unwrap();
        let next = crate::activity::update(map.get(id).copied(), pane_changed, now_ms);
        map.insert(id.to_string(), next);
    }

    /// Drop activity entries for runs no longer in the watch snapshot
    /// (archived or discarded), mirroring the notifier's own watch pruning.
    pub fn retain_activity(&self, keep: &HashSet<String>) {
        self.activity.lock().unwrap().retain(|id, _| keep.contains(id));
        self.usage.lock().unwrap().retain(|id, _| keep.contains(id));
    }

    /// Re-read every run's token usage from its agent's transcript, and append
    /// any growth to the cost ledger. Called once per notifier tick.
    ///
    /// Cheap by construction: `UsageCache` stats each transcript and re-parses
    /// only files whose length or mtime moved, so a board of idle runs costs
    /// one readdir apiece.
    pub fn refresh_usage(&self) -> Result<()> {
        let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
            return Ok(());
        };
        // Every registry read binds to a local before the loop that uses it. A
        // `self.registry.lock()` left in a `for` iterator expression lives for
        // the whole loop body, so the next lock inside it deadlocks the
        // notifier thread outright. `watch_snapshot` binds for the same reason.
        let projects = self.registry.lock().unwrap().list_projects()?;

        // The profile carries the launch command; the agent id alone does not,
        // since a custom profile may name any binary. Resolved once per pass
        // rather than per run: this runs every 2 seconds against a board that
        // can hold dozens of runs sharing a handful of agents.
        let commands: HashMap<String, String> = {
            let reg = self.registry.lock().unwrap();
            reg.list_profiles()?.into_iter().map(|p| (p.name, p.command)).collect()
        };

        for proj in projects {
            let repo = proj.repo_path.clone();
            let runs = self.registry.lock().unwrap().list_runs(&proj.id)?;
            for run in runs {
                let Some(command) = commands.get(&run.agent) else { continue };
                if !agency_core::usage::agent_supported(command) {
                    continue;
                }
                // One worktree per run means this directory is already scoped
                // to the run, and extra agent tabs sharing the worktree belong
                // to the same run, so per-directory totals are per-run totals.
                //
                // The exception is a `worktree: false` run, which works in the
                // project checkout: several of those share one cwd and so
                // report the same figure, since the transcripts give us no way
                // to tell their turns apart. Over-attributing to each is the
                // lesser wrong against silently splitting it.
                let worktree = workspace_dir(&repo, &run);
                let Some(dir) = agency_core::usage::session_dir(&home, command, &worktree) else {
                    continue;
                };

                let mut map = self.usage.lock().unwrap();
                let entry = map.entry(run.id.clone()).or_default();
                let next = entry.0.refresh(&dir);
                let prev = std::mem::replace(&mut entry.1, next.clone());
                drop(map);

                if next != prev {
                    self.append_usage_ledger(&proj.id, &run.id, &run.agent, &next);
                }
            }
        }
        Ok(())
    }

    /// Append one line to the cost ledger, a plain JSONL file.
    ///
    /// It lives in the app data directory rather than the project's `.agency/`
    /// on purpose. `.agency/` sits inside the user's repository, agents run
    /// `git add -A` constantly, and a spend record committed into someone's
    /// project is a leak we would have shipped on their behalf. This is
    /// per-machine state, so it belongs with the per-machine state.
    ///
    /// Best-effort: a ledger write that fails must never disturb the poll.
    fn append_usage_ledger(
        &self,
        project_id: &str,
        run_id: &str,
        agent: &str,
        usage: &agency_core::usage::Usage,
    ) {
        use std::io::Write;
        let info: agency_core::usage::UsageInfo = usage.into();
        let line = serde_json::json!({
            "atMs": crate::activity::now_ms(),
            "projectId": project_id,
            "runId": run_id,
            "agent": agent,
            "records": usage.records,
            "usage": info,
        });
        let path = self.data_dir.join("usage.jsonl");
        let write = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = write {
            log::warn!("usage ledger write failed ({}): {e}", path.display());
        }
    }

    /// Resize the session's PTY so the emulator reflows to the visible terminal.
    /// Sent straight to the daemon by id; harmless if the session isn't live yet
    /// (resize events can race ahead of the session coming up).
    pub fn resize_run(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        self.term.read().unwrap().resize(&session_name(id), cols, rows)
    }

    pub fn run_preview(&self, id: &str, lines: usize) -> Result<String> {
        Ok(self.term.read().unwrap().capture(&session_name(id), lines).unwrap_or_default())
    }

    pub fn discard_run(&self, id: &str) -> Result<()> {
        self.discard_run_with_progress(id, &mut |_| {})
    }

    /// [`discard_run`] reporting each teardown step. Deleting an agent is not
    /// one quick write: it stops a live session, waits on the terminal daemon,
    /// and hands git a worktree that can hold a whole `node_modules` to unlink.
    /// Seconds to tens of seconds, and the caller is a modal the user is
    /// staring at, so it says which step it is on rather than nothing at all.
    pub fn discard_run_with_progress(
        &self,
        id: &str,
        on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress),
    ) -> Result<()> {
        self.attaches.lock().unwrap().remove(id);
        self.input_seen.lock().unwrap().remove(id);
        self.prompted.lock().unwrap().remove(id);
        let run = self.run_record(id)?;
        step(on_progress, "Stopping the agent", &run.branch);
        // End an active loop first (best-effort): once delete_run removes the
        // row, nothing could ever stop a session the driver respawned into
        // the deleted worktree.
        if has_active_loop(&run) {
            if let Err(e) = self.stop_loop(id) {
                log::warn!("discard_run {id}: couldn't end loop: {e}");
            }
        }
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.kill_run_sessions(id);
        self.shell_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&shell_session_name(id));
        self.kill_extra_sessions(id);
        // Only a run that owns a worktree has one to remove. For a run in the
        // main checkout `remove` would try to delete the branch the user is
        // standing on, so discarding is purely dropping the record.
        if run.kind == "agent" && run.worktree {
            if let Ok(repo) = self.project_repo(&run.project_id) {
                step(on_progress, "Removing the worktree", &run.branch);
                // Same mutual exclusion `create_run` takes: this command is
                // async now (off the main thread), so nothing else serializes
                // it against a concurrent worktree add on the same repo.
                let _gate = self.worktree_gate.lock().unwrap();
                let _ = WorktreeManager::new(repo).remove(id);
            }
        }
        step(on_progress, "Cleaning up", &run.branch);
        {
            let reg = self.registry.lock().unwrap();
            reg.delete_run_sessions(id)?;
            reg.delete_run(id)?;
        }
        if let Some(issue_id) = &run.issue_id {
            self.maybe_rollback_issue(issue_id);
        }
        Ok(())
    }

    /// Archive a run: stop its sessions, run the optional archive cleanup script,
    /// remove the worktree but KEEP the branch, and stamp `archived_at`. The run
    /// record is kept so it can be restored.
    ///
    /// A run without a worktree owns nothing on disk, so archiving it is only
    /// the session teardown and the stamp: its changes stay in the checkout,
    /// uncommitted, exactly as the user left them.
    pub fn archive_run(&self, id: &str) -> Result<()> {
        self.archive_run_with_progress(id, &mut |_| {})
    }

    /// [`archive_run`] reporting each teardown step, for the same reason
    /// [`discard_run_with_progress`] does: the auto-commit, the cleanup script
    /// and the worktree removal are each long enough to look like a hang.
    pub fn archive_run_with_progress(
        &self,
        id: &str,
        on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress),
    ) -> Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;

        // End an active loop first (best-effort): archiving spends seconds
        // between the session kills and set_archived, and a driver tick in
        // that window would respawn an attempt into the worktree being
        // removed. Terminal loop state also keeps restore from auto-resuming.
        if has_active_loop(&run) {
            if let Err(e) = self.stop_loop(id) {
                log::warn!("archive_run {id}: couldn't end loop: {e}");
            }
        }

        // Preserve uncommitted agent work FIRST, before any teardown: commit it
        // onto the kept agent branch so restore brings it back. A failure here
        // must abort the archive — proceeding would destroy work. Doing this
        // before killing sessions / deleting session rows means a failed commit
        // leaves the run fully intact instead of half-archived (sessions gone
        // but archived_at still null).
        // A worktree-less run's changes live in the user's own checkout on their
        // own branch. Nothing is about to be removed, so there is nothing to
        // preserve — and auto-committing there would sweep up their work.
        if run.kind == "agent" && run.worktree {
            step(on_progress, "Saving uncommitted changes", &run.branch);
            WorktreeManager::new(repo.clone())
                .commit_all_if_dirty(
                    id,
                    "WIP: uncommitted changes auto-committed by Agency on archive",
                )
                .map_err(|e| {
                    anyhow!("couldn't preserve uncommitted changes before archiving: {e}")
                })?;
        }

        step(on_progress, "Stopping the agent", &run.branch);
        // Stop all sessions and drop attach handles. Extra tabs are purged for
        // good: the worktree they live in is about to disappear.
        self.attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.kill_run_sessions(id);
        self.shell_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&shell_session_name(id));
        self.kill_extra_sessions(id);
        self.registry.lock().unwrap().delete_run_sessions(id)?;

        // Best-effort archive cleanup script, before the worktree disappears.
        // It tears down a workspace; a run that never had one has nothing to
        // tear down, and the script would run against the live checkout.
        let config = agency_core::config::load(&repo);
        if run.worktree {
            if let Some(script) = config.scripts.archive.as_deref() {
                let worktree = workspace_dir(&repo, &run);
                if worktree.exists() {
                    step(on_progress, "Running the archive script", script);
                    let env =
                        agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base);
                    let _ = agency_core::scripts::run_blocking(script, &worktree, &env);
                }
            }
            step(on_progress, "Removing the worktree", &run.branch);
            // See discard_run_with_progress: async command, so the gate stands
            // in for the main-thread serialization this used to get for free.
            let _gate = self.worktree_gate.lock().unwrap();
            WorktreeManager::new(repo).remove_keep_branch(id)?;
        }
        step(on_progress, "Cleaning up", &run.branch);
        self.registry.lock().unwrap().set_archived(id, Some(now_secs()))?;
        // Archiving an unmerged run abandons it from the issue's point of
        // view. A merged run's issue is already done, which rollback skips.
        if let Some(issue_id) = &run.issue_id {
            self.maybe_rollback_issue(issue_id);
        }
        Ok(())
    }

    /// Restore an archived run: re-create its worktree on the kept branch and
    /// clear `archived_at`. The agent is not auto-started.
    pub fn restore_run(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        // Belt and braces: archive_run now ends loops, but rows archived
        // before that fix (or by a crash mid-archive) may still carry a live
        // loop state — which would make the driver auto-resume headless
        // attempts the moment archived_at clears, violating this function's
        // no-auto-start contract.
        if has_active_loop(&run) {
            if let Some(mut st) = run.loop_state.clone() {
                st.status = agency_core::loops::LoopStatus::Stopped;
                st.updated_at = now_secs();
                self.registry.lock().unwrap().set_loop_state(id, &st)?;
            }
        }
        let manager = WorktreeManager::new(repo.clone());
        let config = agency_core::config::load(&repo);
        // Nothing was removed for a worktree-less run, so nothing is re-created:
        // restoring it just makes the row live again.
        if run.worktree {
            manager.restore(id)?;
            if let Err(e) = manager.copy_essentials(id, &config.files.copy) {
                log::warn!("copying essentials into restored worktree {id}: {e}");
            }
        }
        // Reallocate the port block if another active run claimed it while this
        // one was archived (list_port_bases excludes archived rows, so a live
        // collision means a real conflict) — otherwise both export the same
        // AGENCY_PORT into their scripts.
        if let Some(existing) = run.port_base {
            let taken: std::collections::HashSet<u16> =
                self.registry.lock().unwrap().list_port_bases()?.into_iter().collect();
            if taken.contains(&existing) {
                let fresh = self.allocate_port(config.ports.base, config.ports.block_size)?;
                self.registry.lock().unwrap().set_port_base(id, Some(fresh))?;
            }
        }
        if run.worktree {
            self.emit_mcp(&run.agent, &repo, &workspace_dir(&repo, &run), &config);
        }
        self.registry.lock().unwrap().set_archived(id, None)?;
        let refreshed = self.run_record(id)?;
        Ok(self.run_info(&refreshed))
    }

    pub fn list_archived_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_archived_runs(project_id)?;
        let live = self.term.read().unwrap().list().unwrap_or_default();
        Ok(runs.iter().map(|r| self.run_info_from(r, &live)).collect())
    }

    /// Discard every archived run in a project in one sweep: each one's kept
    /// branch, whatever is left of its worktree, and its record go for good.
    ///
    /// Best-effort per run — one run that won't go (a branch git refuses to
    /// delete, say) must not strand the rest of the sweep, so failures are
    /// collected and reported alongside the count that did go.
    pub fn discard_archived_runs(&self, project_id: &str) -> Result<DiscardSummary> {
        self.discard_archived_runs_with_progress(project_id, &mut |_| {})
    }

    /// [`discard_archived_runs`] reporting where the sweep is. A whole
    /// project's worth of archived worktrees is the slowest teardown of the
    /// lot, so the count carries the position and each run's own step names
    /// what it is waiting on.
    pub fn discard_archived_runs_with_progress(
        &self,
        project_id: &str,
        on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress),
    ) -> Result<DiscardSummary> {
        let mut summary = DiscardSummary { discarded: 0, failed: Vec::new() };
        let archived = self.list_archived_runs(project_id)?;
        let total = archived.len();
        for (i, run) in archived.iter().enumerate() {
            let label = run.title.clone().unwrap_or_else(|| run.branch.clone());
            // The sweep's position replaces each run's own detail line: how far
            // through the list it is matters more here than one branch's name.
            let detail = sweep_detail(&label, i, total);
            let mut relay = |p: agency_core::setup::CloneProgress| {
                on_progress(agency_core::setup::CloneProgress { detail: detail.clone(), ..p });
            };
            match self.discard_run_with_progress(&run.id, &mut relay) {
                Ok(()) => summary.discarded += 1,
                Err(e) => {
                    log::warn!("discard_archived_runs: couldn't discard {}: {e}", run.id);
                    summary.failed.push(format!("{label}: {e}"));
                }
            }
        }
        Ok(summary)
    }

    pub fn stop_run(&self, id: &str) -> Result<()> {
        // Under the spawn gate so a Stop can't land between a rerun's kill and
        // its spawn, which would leave the session the user just stopped alive.
        self.spawn_gates.with(id, || self.stop_run_locked(id))
    }

    fn stop_run_locked(&self, id: &str) -> Result<()> {
        // Stopping a looping run ends the loop too, or the driver would just
        // respawn the session on the next tick. Best-effort throughout: a
        // failed loop-state write must not abort the teardown below (the
        // kills are this function's contract), and extra-tab ids and
        // already-deleted runs have no record at all.
        if let Ok(run) = self.run_record(id) {
            if has_active_loop(&run) {
                if let Err(e) = self.stop_loop(id) {
                    log::warn!("stop_run {id}: couldn't end loop: {e}");
                }
            }
        }
        self.attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.kill_run_sessions(id);
        Ok(())
    }

    /// Resolve a run-script target to the workspace its scripts run in. A
    /// `project:<id>` token means the project's own checkout — the same token
    /// [`git_root`] takes, so "run it at project level" needs no separate set
    /// of commands. Anything else is a run id.
    fn run_target(&self, target: &str) -> Result<RunTarget> {
        if let Some(pid) = target.strip_prefix("project:") {
            let repo = self.project_repo(pid)?;
            let config = agency_core::config::load(&repo);
            return Ok(RunTarget {
                project_id: pid.to_string(),
                name: pid.to_string(),
                cwd: repo.clone(),
                repo,
                // The checkout's own block. `allocate_port` skips it, so an
                // agent's dev server never lands on the same port as yours.
                port: Some(config.ports.base),
            });
        }
        let run = self.run_record(target)?;
        let repo = self.project_repo(&run.project_id)?;
        let cwd = workspace_dir(&repo, &run);
        Ok(RunTarget {
            project_id: run.project_id.clone(),
            name: run.id.clone(),
            repo,
            cwd,
            port: run.port_base,
        })
    }

    /// Every live run-script session name for `target`, newest daemon view.
    fn live_run_sessions(&self, target: &str) -> Vec<String> {
        let live: Vec<String> = self
            .term
            .read()
            .unwrap()
            .list()
            .map(|v| v.into_iter().map(|(name, _)| name).collect())
            .unwrap_or_default();
        run_session_names_for(target, &live)
    }

    /// Stop every run script running in `target`'s workspace. Used wherever a
    /// workspace goes away (archive, discard, project close) and by the
    /// "one app at a time" mode.
    fn kill_run_sessions(&self, target: &str) {
        for name in self.live_run_sessions(target) {
            self.run_attaches.lock().unwrap().remove(&name);
            let _ = self.term.read().unwrap().kill(&name);
        }
    }

    /// The project's run scripts as the Run tab sees them, including detected
    /// candidates so an unconfigured project can be set up from the panel.
    pub fn run_script_config(&self, target: &str) -> Result<RunScriptConfigDto> {
        let t = self.run_target(target)?;
        let config = agency_core::config::load(&t.repo);
        Ok(RunScriptConfigDto {
            scripts: config.scripts.run_list(),
            shared: agency_core::config::run_scripts_are_shared(&t.repo),
            // Detection reads the project checkout, not an agent's worktree:
            // the config it prefills is project-wide, and a brand-new worktree
            // may not have installed anything yet.
            suggestions: agency_core::runsetup::suggest_run_commands(&t.repo),
            workspace: t.cwd.display().to_string(),
            port: t.port,
        })
    }

    /// Persist the project's run list from the Run tab. The list is
    /// project-wide however it was reached: editing it from an agent's Run tab
    /// and from the project's own are the same edit.
    pub fn save_run_scripts(
        &self,
        target: &str,
        scripts: Vec<agency_core::config::RunScript>,
    ) -> Result<()> {
        let t = self.run_target(target)?;
        agency_core::config::save_run_scripts(&t.repo, &scripts)?;
        Ok(())
    }

    pub fn start_run_script(&self, target: &str, script_name: &str) -> Result<()> {
        let t = self.run_target(target)?;
        let config = agency_core::config::load(&t.repo);
        let script =
            config.scripts.run_list().into_iter().find(|s| s.name == script_name).ok_or_else(
                || anyhow!("this project has no run script called \"{script_name}\""),
            )?;

        // "One app at a time": every other run script in the project stops,
        // including the other scripts in this very workspace. The command binds
        // a fixed port, so a second copy of anything would just fail to bind.
        if script.nonconcurrent {
            let mut targets: Vec<String> = self
                .registry
                .lock()
                .unwrap()
                .list_runs(&t.project_id)?
                .into_iter()
                .map(|r| r.id)
                .collect();
            targets.push(format!("project:{}", t.project_id));
            for other in targets {
                self.kill_run_sessions(&other);
            }
        }

        let mut env = self.provider_env()?;
        env.extend(agency_core::scripts::script_env(&t.cwd, &t.repo, &t.name, t.port));

        // Restart cleanly if this script's previous session is still around.
        let session = run_session_name(target, &script.name);
        self.run_attaches.lock().unwrap().remove(&session);
        let _ = self.term.read().unwrap().kill(&session);
        self.term.read().unwrap().start_session(
            &session,
            &t.cwd,
            "sh",
            &["-lc".to_string(), script.command],
            &env,
            220,
            50,
        )
    }

    pub fn stop_run_script(&self, target: &str, script_name: &str) -> Result<()> {
        let session = run_session_name(target, script_name);
        self.run_attaches.lock().unwrap().remove(&session);
        let _ = self.term.read().unwrap().kill(&session);
        Ok(())
    }

    /// One status per configured script, so the Run tab polls once however many
    /// scripts a project has.
    pub fn run_scripts_status(&self, target: &str) -> Result<Vec<RunScriptStatusDto>> {
        let t = self.run_target(target)?;
        let config = agency_core::config::load(&t.repo);
        let term = self.term.read().unwrap();
        Ok(config
            .scripts
            .run_list()
            .into_iter()
            .map(|s| {
                let status =
                    term.status(&run_session_name(target, &s.name)).unwrap_or(SessionStatus::Gone);
                RunScriptStatusDto { name: s.name, status }
            })
            .collect())
    }

    pub fn run_script_preview(&self, target: &str, script: &str, lines: usize) -> Result<String> {
        Ok(self
            .term
            .read()
            .unwrap()
            .capture(&run_session_name(target, script), lines)
            .unwrap_or_default())
    }

    pub fn attach_run_script<F>(
        &self,
        target: &str,
        script: &str,
        cols: u16,
        rows: u16,
        on_output: F,
    ) -> Result<()>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        let session = run_session_name(target, script);
        let sub = self.term.read().unwrap().subscribe(&session, cols, rows, on_output)?;
        self.run_attaches.lock().unwrap().insert(session, sub);
        Ok(())
    }

    pub fn detach_run_script(&self, target: &str, script: &str) {
        // Dropping the Subscription sends Unsubscribe; the run-script session keeps
        // running server-side. See detach_run.
        self.run_attaches.lock().unwrap().remove(&run_session_name(target, script));
    }

    pub fn run_script_input(&self, target: &str, script: &str, data: &[u8]) -> Result<()> {
        self.term.read().unwrap().input(&run_session_name(target, script), data)
    }

    pub fn resize_run_script(
        &self,
        target: &str,
        script: &str,
        cols: u16,
        rows: u16,
    ) -> Result<()> {
        self.term.read().unwrap().resize(&run_session_name(target, script), cols, rows)
    }

    /// Start (or reuse) the run's companion shell: an interactive login shell
    /// rooted in the run's worktree. Idempotent — if the session is already
    /// running it is left untouched so scrollback and any in-flight command
    /// survive the UI panel being toggled or the terminal remounting.
    pub fn start_shell(&self, id: &str) -> Result<()> {
        // Already live? Leave it alone.
        if matches!(
            self.term.read().unwrap().status(&shell_session_name(id)),
            Ok(SessionStatus::Running)
        ) {
            return Ok(());
        }
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        // A run with a worktree gets it; everything else the repo root.
        let cwd = self.worktree_path(id)?;
        let shell = login_shell();
        let args = vec!["-l".to_string()];
        let mut env = self.provider_env()?;
        env.extend(agency_core::scripts::script_env(&cwd, &repo, &run.id, run.port_base));

        // Restart cleanly if a dead session lingers.
        let _ = self.term.read().unwrap().kill(&shell_session_name(id));
        self.term.read().unwrap().start_session(
            &shell_session_name(id),
            &cwd,
            &shell,
            &args,
            &env,
            220,
            50,
        )
    }

    pub fn stop_shell(&self, id: &str) -> Result<()> {
        self.shell_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&shell_session_name(id));
        Ok(())
    }

    pub fn shell_status(&self, id: &str) -> Result<SessionStatus> {
        Ok(self.term.read().unwrap().status(&shell_session_name(id)).unwrap_or(SessionStatus::Gone))
    }

    pub fn shell_preview(&self, id: &str, lines: usize) -> Result<String> {
        Ok(self.term.read().unwrap().capture(&shell_session_name(id), lines).unwrap_or_default())
    }

    pub fn attach_shell<F>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<()>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        let sub =
            self.term.read().unwrap().subscribe(&shell_session_name(id), cols, rows, on_output)?;
        self.shell_attaches.lock().unwrap().insert(id.to_string(), sub);
        Ok(())
    }

    pub fn detach_shell(&self, id: &str) {
        // Dropping the Subscription unsubscribes but leaves the shell running
        // server-side, so re-opening the panel resumes the same session.
        self.shell_attaches.lock().unwrap().remove(id);
    }

    pub fn shell_input(&self, id: &str, data: &[u8]) -> Result<()> {
        self.term.read().unwrap().input(&shell_session_name(id), data)
    }

    pub fn resize_shell(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        self.term.read().unwrap().resize(&shell_session_name(id), cols, rows)
    }

    // ── Extra agent sessions (additional agent tabs sharing a run's worktree) ──

    /// Launch (or relaunch) an extra session's agent in the parent run's
    /// worktree, opening with `prompt` (empty for the usual promptless tab).
    /// Always a fresh launch — resume recipes pick the cwd's most recent
    /// conversation, which in a shared worktree may belong to a sibling tab,
    /// so extras never resume.
    fn launch_run_session(
        &self,
        sid: &str,
        run: &agency_core::registry::Run,
        agent: &str,
        prompt: &str,
    ) -> Result<()> {
        let repo = self.project_repo(&run.project_id)?;
        let config = agency_core::config::load(&repo);
        let worktree = workspace_dir(&repo, &run);
        // A terminal tab is not an agent: no profile, no MCP config, no argv
        // recipe — just the user's login shell in the worktree, matching what
        // `start_shell` and `create_terminal` do.
        if agent == SHELL_AGENT {
            let mut env = self.provider_env()?;
            env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
            return self.term.read().unwrap().start_session(
                &session_name(sid),
                &worktree,
                &login_shell(),
                &["-l".to_string()],
                &env,
                220,
                50,
            );
        }
        let profile = {
            let reg = self.registry.lock().unwrap();
            let profile =
                reg.get_profile(agent)?.ok_or_else(|| anyhow!("unknown agent profile: {agent}"))?;
            // An extra tab may run a different agent than the run does, and the
            // run's model belongs to that agent's namespace — "opus" means
            // nothing to Codex. Only carry it when the tab is the same agent.
            let model = (agent == run.agent).then_some(run.model.as_deref()).flatten();
            with_model(&profile, model)
        };
        // The extra tab may run a different agent than the one the worktree
        // was created for; make sure MCP config exists in its native format.
        // Skipped without a worktree, for the same reason as at creation: the
        // target file would be one in the user's own checkout.
        if run.worktree {
            self.emit_mcp(agent, &repo, &worktree, &config);
        }
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        // Same env recipe as the run itself, ports included: extra sessions
        // are collaborators in the same workspace, not new workspaces.
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
        let (command, args) =
            fresh_agent_argv(&profile, &worktree, prompt, config.scripts.setup.as_deref());
        self.term.read().unwrap().start_session(
            &session_name(sid),
            &worktree,
            &command,
            &args,
            &env,
            220,
            50,
        )
    }

    /// Open an additional agent tab in an existing run's worktree. `agent`
    /// defaults to the run's own agent profile; `prompt` is empty for a plain
    /// tab and set when the tab is opened for a specific job (a PR review).
    pub fn start_run_session(
        &self,
        run_id: &str,
        agent: Option<&str>,
        prompt: &str,
    ) -> Result<RunSessionInfo> {
        let run = self.run_record(run_id)?;
        if run.kind != "agent" {
            bail!("only agent runs can host extra sessions");
        }
        if run.archived_at.is_some() {
            bail!("run is archived — restore it before adding sessions");
        }
        let agent = agent.unwrap_or(&run.agent).to_string();
        // Next tab number: the primary is implicitly 1, extras start at --2.
        // Gaps left by closed tabs are fine; only uniqueness matters.
        let next = {
            let reg = self.registry.lock().unwrap();
            reg.list_run_sessions(run_id)?
                .iter()
                .filter_map(|s| split_session_id(&s.id).1)
                .max()
                .unwrap_or(1)
                + 1
        };
        let session = agency_core::registry::RunSession {
            id: format!("{run_id}--{next}"),
            run_id: run_id.to_string(),
            agent,
            created_at: now_secs(),
        };
        self.launch_run_session(&session.id, &run, &session.agent, prompt)?;
        self.registry.lock().unwrap().insert_run_session(&session)?;
        Ok(self.run_session_info(&session))
    }

    pub fn run_sessions(&self, run_id: &str) -> Result<Vec<RunSessionInfo>> {
        let rows = self.registry.lock().unwrap().list_run_sessions(run_id)?;
        Ok(rows.iter().map(|s| self.run_session_info(s)).collect())
    }

    /// Close an extra agent tab: kill its daemon session and forget it. The
    /// worktree, branch and every sibling session are untouched.
    pub fn close_run_session(&self, id: &str) -> Result<()> {
        let (_, seq) = split_session_id(id);
        if seq.is_none() {
            bail!("not an extra session id: {id}");
        }
        self.attaches.lock().unwrap().remove(id);
        self.input_seen.lock().unwrap().remove(id);
        self.prompted.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.registry.lock().unwrap().delete_run_session(id)?;
        Ok(())
    }

    fn run_session_info(&self, s: &agency_core::registry::RunSession) -> RunSessionInfo {
        let status =
            self.term.read().unwrap().status(&session_name(&s.id)).unwrap_or(SessionStatus::Gone);
        RunSessionInfo {
            id: s.id.clone(),
            run_id: s.run_id.clone(),
            agent: s.agent.clone(),
            status,
        }
    }

    /// Kill every extra session of a run and drop their attach handles. The
    /// registry rows stay unless the caller removes them: discard/archive
    /// purge them (the worktree is going away), close_project keeps them so
    /// reopening the project revives the tabs.
    fn kill_extra_sessions(&self, run_id: &str) {
        let rows = self.registry.lock().unwrap().list_run_sessions(run_id).unwrap_or_default();
        for s in rows {
            self.attaches.lock().unwrap().remove(&s.id);
            self.input_seen.lock().unwrap().remove(&s.id);
            self.prompted.lock().unwrap().remove(&s.id);
            let _ = self.term.read().unwrap().kill(&session_name(&s.id));
        }
    }

    /// Ensure the run has a live daemon session, transparently respawning it if the
    /// previous session is gone (e.g. after the app was quit). Prefers resuming the
    /// agent's prior context; falls back to a fresh start; terminals get a fresh
    /// shell. No-op if a session already exists.
    pub fn ensure_run_active(&self, id: &str) -> Result<()> {
        // Under the spawn gate, with the Gone check inside it: the check and
        // the spawn are not one step, and two attach paths (or one of them and
        // a rerun) could otherwise both read Gone and both spawn. See
        // `spawn_gates` for why a second process is worse than a slow one.
        self.spawn_gates.with(id, || self.ensure_run_active_locked(id))
    }

    fn ensure_run_active_locked(&self, id: &str) -> Result<()> {
        if !matches!(self.run_status(id)?, SessionStatus::Gone) {
            return Ok(());
        }
        // Extra tab (`<run>--<n>`): relaunch its agent fresh in the parent
        // run's worktree (see launch_run_session for why extras never resume).
        if let (run_id, Some(_)) = split_session_id(id) {
            let session = {
                let reg = self.registry.lock().unwrap();
                reg.get_run_session(id)?.ok_or_else(|| anyhow!("unknown session: {id}"))?
            };
            let run = self.run_record(run_id)?;
            // A relaunch is a blank slate, not a re-run of whatever job first
            // opened the tab: the original prompt is not replayed.
            return self.launch_run_session(id, &run, &session.agent, "");
        }
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;

        // The loop driver owns an active loop's session — never spawn a rival
        // interactive agent. But an AwaitingAgent loop with no session (the
        // gap between attempts, or right after an app restart) would leave the
        // pane dead until the driver's next tick, so bring the next headless
        // attempt up now; the driver treats a Running session as a no-op.
        // During Checking the session stays down by design (the check result
        // decides what happens next) — the loop strip shows the state.
        if has_active_loop(&run) {
            let _gate = self.loop_gate.lock().unwrap();
            let Some(fresh) = self.registry.lock().unwrap().get_run(id)? else {
                return Ok(());
            };
            let awaiting = fresh
                .loop_state
                .as_ref()
                .map(|s| s.status == agency_core::loops::LoopStatus::AwaitingAgent)
                .unwrap_or(false);
            if awaiting && matches!(self.run_status(id)?, SessionStatus::Gone) {
                self.spawn_loop_attempt(&fresh)?;
            }
            return Ok(());
        }

        if run.kind == "terminal" {
            let shell = login_shell();
            self.term.read().unwrap().start_session(
                &session_name(id),
                &repo,
                &shell,
                &["-l".to_string()],
                &[],
                220,
                50,
            )?;
            return Ok(());
        }

        let config = agency_core::config::load(&repo);
        let worktree = workspace_dir(&repo, &run);
        let profile = {
            let reg = self.registry.lock().unwrap();
            let profile = reg
                .get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?;
            // Whatever model the run started on, it comes back on: a resume
            // that quietly changed model would rewrite the session's terms
            // halfway through the work.
            with_model(&profile, run.model.as_deref())
        };
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
        let setup = config.scripts.setup.as_deref();
        // Decide resume-vs-fresh up front. For claude/pi we can prove whether a
        // session exists (they don't exit on resume-failure, so the daemon
        // fallback can't save them); other resume-capable agents fall through to
        // the resume-with-fallback path (the fallback catches their fast exits).
        let probe = std::env::var_os("HOME")
            .map(|h| {
                crate::resume_probe::resume_probe(
                    std::path::Path::new(&h),
                    &profile.command,
                    &worktree,
                )
            })
            .unwrap_or(crate::resume_probe::ResumeProbe::Unknown);
        let use_resume =
            profile.resume_args.is_some() && probe != crate::resume_probe::ResumeProbe::None;
        let (command, args) = agent_argv(&profile, &worktree, &run.prompt, use_resume, setup);
        let fallback = if use_resume {
            let (fresh_cmd, fresh_args) =
                agent_argv(&profile, &worktree, &run.prompt, false, setup);
            Some(agency_core::term::protocol::FallbackSpec {
                command: fresh_cmd,
                args: fresh_args,
                grace_ms: 3000,
            })
        } else {
            None
        };
        self.term.read().unwrap().start_session_with_fallback(
            &session_name(id),
            &worktree,
            &command,
            &args,
            &env,
            220,
            50,
            fallback,
        )?;
        Ok(())
    }

    pub fn rerun(&self, id: &str) -> Result<RunInfo> {
        // Under the spawn gate: the kill and the spawn below are two daemon
        // round-trips, and the registry inserts by session id, so two
        // overlapping reruns would start two agent processes and register only
        // the second. Waiting (rather than refusing) keeps a double-click doing
        // what it did on the main thread: two reruns, one after the other.
        self.spawn_gates.with(id, || self.rerun_locked(id))
    }

    fn rerun_locked(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
        if has_active_loop(&run) {
            bail!("this run is looping — stop the loop before rerunning it manually");
        }
        let repo = self.project_repo(&run.project_id)?;
        let config = agency_core::config::load(&repo);
        let worktree = workspace_dir(&repo, &run);
        let profile = {
            let reg = self.registry.lock().unwrap();
            let profile = reg
                .get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?;
            with_model(&profile, run.model.as_deref())
        };
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
        // A rerun is a fresh launch by definition, so it takes the same argv the
        // fresh side of `agent_argv` builds (never the resume recipe).
        let (command, args) =
            agent_argv(&profile, &worktree, &run.prompt, false, config.scripts.setup.as_deref());
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.term.read().unwrap().start_session(
            &session_name(id),
            &worktree,
            &command,
            &args,
            &env,
            220,
            50,
        )?;
        Ok(self.run_info(&run))
    }

    // ── agentic loops ──────────────────────────────────────────────────────────

    /// Kill any leftover session and launch the next headless attempt in the
    /// run's existing worktree. Uncommitted changes a previous attempt left
    /// behind are committed first so every attempt has an auditable boundary
    /// in `git log` and nothing is invisibly carried or clobbered.
    fn spawn_loop_attempt(&self, run: &agency_core::registry::Run) -> Result<()> {
        let repo = self.project_repo(&run.project_id)?;
        let config = agency_core::config::load(&repo);
        let worktree = workspace_dir(&repo, &run);
        let attempt = run.loop_state.as_ref().map(|s| s.attempt).unwrap_or(1);
        match agency_core::git::status(&worktree) {
            Ok(changes) if !changes.is_empty() => {
                agency_core::git::stage_all(&worktree)?;
                agency_core::git::commit(&worktree, &format!("wip: loop attempt {attempt}"))?;
            }
            _ => {}
        }
        let profile = {
            let reg = self.registry.lock().unwrap();
            let profile = reg
                .get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?;
            with_model(&profile, run.model.as_deref())
        };
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
        let (command, args) =
            loop_argv(&profile, &worktree, &run.prompt, config.scripts.setup.as_deref())?;
        let _ = self.term.read().unwrap().kill(&session_name(&run.id));
        self.term.read().unwrap().start_session(
            &session_name(&run.id),
            &worktree,
            &command,
            &args,
            &env,
            220,
            50,
        )
    }

    /// Run the loop's check command in the worktree on its own thread; the
    /// result lands in the run's check slot for `drive_loops` to drain. The
    /// child is killed at the config's timeout (counted as a failed check).
    fn start_loop_check(
        &self,
        run: &agency_core::registry::Run,
        cfg: &agency_core::loops::LoopConfig,
    ) -> Result<()> {
        let repo = self.project_repo(&run.project_id)?;
        let worktree = workspace_dir(&repo, &run);
        let slot = std::sync::Arc::new(CheckSlot {
            status: Mutex::new(CheckStatus::Running),
            cancelled: std::sync::atomic::AtomicBool::new(false),
        });
        self.checks.lock().unwrap().insert(run.id.clone(), slot.clone());
        let command = cfg.check_command.clone();
        let timeout = std::time::Duration::from_secs(cfg.check_timeout_secs.max(1));
        std::thread::spawn(move || {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            let spawned = std::process::Command::new(shell)
                .arg("-lc")
                .arg(&command)
                .current_dir(&worktree)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            let mut child = match spawned {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("loop check failed to spawn: {e}");
                    *slot.status.lock().unwrap() = CheckStatus::Done(Some(127));
                    return;
                }
            };
            let deadline = std::time::Instant::now() + timeout;
            let code = loop {
                // Loop stopped/discarded: kill the child now rather than let a
                // full test run keep writing into a worktree that may be
                // removed out from under it.
                if slot.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                match child.try_wait() {
                    // `code()` is None when the check died on a signal; both
                    // that and timeout report as None (a failed check).
                    Ok(Some(status)) => break status.code(),
                    Ok(None) if std::time::Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(500)),
                    Err(e) => {
                        log::warn!("loop check wait failed: {e}");
                        break Some(127);
                    }
                }
            };
            *slot.status.lock().unwrap() = CheckStatus::Done(code);
        });
        Ok(())
    }

    /// Halt a loop: mark it Stopped (terminal) and kill the live attempt.
    /// The worktree and its commits stay for the normal finish flow.
    pub fn stop_loop(&self, id: &str) -> Result<()> {
        // Under the loop gate: an in-flight drive_loops tick that read the run
        // before this Stop must not see the killed session as Gone and respawn
        // it — the gate makes it re-read the Stopped state instead.
        let _gate = self.loop_gate.lock().unwrap();
        let run = self.run_record(id)?;
        if let Some(mut st) = run.loop_state {
            if !st.status.is_terminal() {
                st.status = agency_core::loops::LoopStatus::Stopped;
                st.updated_at = now_secs();
                self.registry.lock().unwrap().set_loop_state(id, &st)?;
            }
        }
        // Cancel (not just forget) any in-flight check so its thread kills the
        // child instead of leaving it running against this worktree.
        if let Some(slot) = self.checks.lock().unwrap().remove(id) {
            slot.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&session_name(id));
        Ok(())
    }

    /// Advance every active loop one step: read the session/check snapshot,
    /// run the pure `looper::step`, persist the new state, then perform the
    /// side effects. Called from the watcher thread each poll tick; returns
    /// the terminal transitions for it to notify about.
    pub fn drive_loops(&self) -> Result<Vec<LoopNotice>> {
        // Fast path: no loops anywhere (the common case) is one atomic load,
        // not a registry scan on every poll tick.
        if !self.loops_active.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(Vec::new());
        }
        // Snapshot the generation BEFORE scanning: if a loop is created while
        // this pass runs, the generation changes and we must not clear the flag.
        let gen_at_start = self.loop_generation.load(std::sync::atomic::Ordering::SeqCst);
        let mut notices = Vec::new();
        let mut any_active = false;
        let projects = self.registry.lock().unwrap().list_projects()?;
        for proj in projects {
            let runs = self.registry.lock().unwrap().list_runs(&proj.id)?;
            for run in runs {
                if run.loop_config.is_none() {
                    continue;
                }
                // The whole read → step → persist → act sequence runs under
                // the loop gate, and the record is re-read inside it: a Stop
                // (or archive/discard) landing after the list_runs read above
                // has already killed the session, and acting on the stale
                // state would respawn it as an unkillable orphan.
                let _gate = self.loop_gate.lock().unwrap();
                let Some(run) = self.registry.lock().unwrap().get_run(&run.id)? else {
                    continue;
                };
                let (Some(cfg), Some(prev)) = (run.loop_config.clone(), run.loop_state.clone())
                else {
                    continue;
                };
                if prev.status.is_terminal() || run.archived_at.is_some() {
                    continue;
                }
                any_active = true;
                let agent = self
                    .term
                    .read()
                    .unwrap()
                    .status(&session_name(&run.id))
                    .unwrap_or(SessionStatus::Gone);
                let (check_in_flight, check) = {
                    let mut checks = self.checks.lock().unwrap();
                    match checks.get(&run.id).map(|s| *s.status.lock().unwrap()) {
                        Some(CheckStatus::Running) => (true, None),
                        Some(CheckStatus::Done(code)) => {
                            checks.remove(&run.id);
                            (false, Some(crate::looper::CheckResult { exit_code: code }))
                        }
                        None => (false, None),
                    }
                };
                let snap = crate::looper::LoopSnapshot { agent, check_in_flight, check };
                let (next, actions) = crate::looper::step(&cfg, &prev, &snap, now_secs());
                if next != prev {
                    self.registry.lock().unwrap().set_loop_state(&run.id, &next)?;
                }
                if actions.is_empty() {
                    continue;
                }
                let mut updated = run.clone();
                updated.loop_state = Some(next.clone());
                let label = format!(
                    "{}: {}",
                    run.agent,
                    run.title
                        .clone()
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| run.prompt.clone())
                );
                for action in actions {
                    match action {
                        crate::looper::LoopAction::SpawnAttempt => {
                            if let Err(e) = self.spawn_loop_attempt(&updated) {
                                // A respawn that can't work (agent gone, git
                                // broken) would otherwise retry every tick
                                // forever; stall loudly instead.
                                log::warn!("loop {}: attempt spawn failed, stalling: {e}", run.id);
                                let mut stalled = next.clone();
                                stalled.status = agency_core::loops::LoopStatus::Stalled;
                                stalled.updated_at = now_secs();
                                self.registry.lock().unwrap().set_loop_state(&run.id, &stalled)?;
                                notices.push(LoopNotice {
                                    project_id: proj.id.clone(),
                                    run_id: run.id.clone(),
                                    label: label.clone(),
                                    done: false,
                                    attempt: stalled.attempt,
                                });
                            }
                        }
                        crate::looper::LoopAction::StartCheck => {
                            if let Err(e) = self.start_loop_check(&updated, &cfg) {
                                log::warn!("loop {}: check spawn failed: {e}", run.id);
                            }
                        }
                        crate::looper::LoopAction::NotifyComplete => notices.push(LoopNotice {
                            project_id: proj.id.clone(),
                            run_id: run.id.clone(),
                            label: label.clone(),
                            done: true,
                            attempt: next.attempt,
                        }),
                        crate::looper::LoopAction::NotifyStalled => notices.push(LoopNotice {
                            project_id: proj.id.clone(),
                            run_id: run.id.clone(),
                            label: label.clone(),
                            done: false,
                            attempt: next.attempt,
                        }),
                    }
                }
            }
        }
        // A full pass with nothing active parks the driver until the next
        // loop is created (terminal transitions above count as active for one
        // last pass, which is what clears the flag). Clear under loop_gate and
        // re-read the generation while holding it: create_run_spec bumps the
        // generation and sets the flag under this same gate, so a loop created
        // after gen_at_start was snapshotted no longer matches here and we leave
        // the flag set. Taking the gate makes the read-and-clear atomic against
        // that set — otherwise the load-then-store races the creator's
        // set-then-store and can park the new loop until the next creation.
        if !any_active {
            let _gate = self.loop_gate.lock().unwrap();
            if self.loop_generation.load(std::sync::atomic::Ordering::SeqCst) == gen_at_start {
                self.loops_active.store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        Ok(notices)
    }

    // ── worktree path ──────────────────────────────────────────────────────────

    /// The working directory the run's git/file commands operate on.
    ///
    /// For a run with its own worktree this is `<repo>/.agency/worktrees/<id>`.
    /// For terminals, and for agents started directly on the checked-out branch,
    /// it is the project repo root, so Source Control / Files act on the live
    /// branch.
    pub fn worktree_path(&self, id: &str) -> Result<std::path::PathBuf> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        Ok(workspace_dir(&repo, &run))
    }

    /// Resolve a git-root token to a working directory. A `project:<id>` token
    /// targets the project's main checkout (its active branch); any other token
    /// is a run id resolved via [`worktree_path`] (terminals already map to the
    /// repo root). Lets the git commands operate at project level, not just per
    /// run, without changing their IPC signatures.
    pub fn git_root(&self, token: &str) -> Result<std::path::PathBuf> {
        Ok(self.git_target(token)?.path)
    }

    /// [`git_root`], plus which project's *own* checkout that directory is —
    /// the thing `repo_gates` is keyed by. `primary` is set for a
    /// `project:<id>` token, for a terminal, and for an agent started without a
    /// worktree, because all three resolve to the one checkout the merge family
    /// moves between branches. It is `None` for an agent's private worktree,
    /// which has an index of its own.
    fn git_target(&self, token: &str) -> Result<GitTarget> {
        if let Some(pid) = token.strip_prefix("project:") {
            return Ok(GitTarget { path: self.project_repo(pid)?, primary: Some(pid.to_string()) });
        }
        let run = self.run_record(token)?;
        let repo = self.project_repo(&run.project_id)?;
        Ok(GitTarget {
            path: workspace_dir(&repo, &run),
            primary: (!run.worktree).then(|| run.project_id.clone()),
        })
    }

    /// Run a git command that writes its target — the index, the working tree,
    /// a ref, the config — holding that project's checkout gate when the target
    /// *is* the project's checkout. Every mutating `git_*` command goes through
    /// this rather than [`git_root`]: they are async now, so this is what keeps
    /// a stage or a checkout from landing in the middle of a merge. See
    /// `repo_gates` for why an agent's own worktree isn't gated.
    pub fn git_mutate<T>(&self, token: &str, f: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
        let target = self.git_target(token)?;
        match &target.primary {
            Some(project_id) => self.repo_gates.with(project_id, || f(&target.path)),
            None => f(&target.path),
        }
    }

    /// Source and destination branch names for a run, for the status bar.
    /// `branch` is the worktree's live current branch (for terminals, which
    /// share the project checkout, this is whatever is checked out there);
    /// `base` is the resolved merge target.
    pub fn run_branches(&self, id: &str) -> anyhow::Result<(String, String)> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
        let wt = self.worktree_path(id)?;
        let branch = agency_core::git::branch_info(&wt).map(|b| b.branch).unwrap_or_default();
        Ok((branch, base))
    }

    // ── merge operations ───────────────────────────────────────────────────────

    /// Inspect what merging this run's branch would do, without touching the
    /// repo: which base it targets, how many commits the branch is ahead, and
    /// whether the agent's worktree still has uncommitted changes.
    pub fn merge_preview(&self, id: &str) -> anyhow::Result<MergePreview> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        require_own_branch(&run, &repo, "merge")?;
        require_branch_exists(&run, &repo)?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
        let commits_ahead = agency_core::merge::commits_ahead(&repo, &run.branch, &base)?;
        let commits_behind = agency_core::merge::commits_behind(&repo, &run.branch, &base)?;
        let worktree = workspace_dir(&repo, &run);
        let dirty_files: Vec<String> = if worktree.exists() {
            agency_core::git::status(&worktree)
                .map(|cs| cs.into_iter().map(|c| c.path).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(MergePreview {
            base,
            branch: run.branch,
            commits_ahead,
            commits_behind,
            worktree_dirty: !dirty_files.is_empty(),
            dirty_files,
        })
    }

    pub fn merge_task(&self, id: &str) -> anyhow::Result<agency_core::merge::MergeOutcome> {
        self.merge_task_with_progress(id, &mut |_| {})
    }

    /// [`merge_task`] reporting each step it is on. `git merge` runs in the
    /// project's shared checkout and takes seconds on a large repo — a checkout
    /// of the base branch, then the merge itself — behind a modal that could
    /// otherwise only sit there.
    pub fn merge_task_with_progress(
        &self,
        id: &str,
        on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress),
    ) -> anyhow::Result<agency_core::merge::MergeOutcome> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        require_own_branch(&run, &repo, "merge")?;
        require_branch_exists(&run, &repo)?;
        // An active loop is still committing attempts onto this branch; merging
        // mid-flight would take a half-done attempt and keep drifting after.
        if has_active_loop(&run) {
            bail!("this run is looping — stop the loop before merging");
        }
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
        // try_with, not with: a merge that queued behind whatever else is
        // writing this checkout would go on to run against a repo it never
        // looked at, and a second merge of the same branch would land as a
        // no-op that reports as a fresh success. Fail fast and let the user
        // retry once they can see what the checkout is doing.
        let merged = self.repo_gates.try_with(&run.project_id, || {
            // Captured before the merge moves the checkout: on the conflict path
            // `merge()` deliberately stays on `base`, so this is the only record of
            // where to return once resolution ends.
            let original = agency_core::merge::current_branch(&repo);
            let outcome =
                agency_core::merge::merge_with_progress(&repo, &run.branch, &base, on_progress)?;
            match &outcome {
                agency_core::merge::MergeOutcome::Clean { .. } => {
                    step(on_progress, "Tidying up", &run.branch);
                    self.after_merge_landed(&run, &repo);
                }
                agency_core::merge::MergeOutcome::Conflicts { .. } => {
                    if let Some(orig) = original.filter(|o| o != &base) {
                        self.merge_origins.lock().unwrap().insert(id.to_string(), orig);
                    }
                }
            }
            Ok(outcome)
        });
        merged.unwrap_or_else(|| Err(anyhow!(busy_checkout_error())))
    }

    /// Bookkeeping for a merge that landed, however it landed: first try, or
    /// after conflicts were resolved. Best-effort throughout — the merge itself
    /// already succeeded and must not be reported as a failure.
    fn after_merge_landed(&self, run: &agency_core::registry::Run, repo: &std::path::Path) {
        self.maybe_rebuild_knowledge_graph(repo);
        // A merged run completes its issue; the forward-only guard in
        // `advance_issue` keeps this from regressing anything.
        if let Some(issue_id) = &run.issue_id {
            let reg = self.registry.lock().unwrap();
            if let Err(e) = self.advance_issue(&reg, issue_id, IssueStatus::Done) {
                log::warn!("closing issue {issue_id} after merge: {e}");
            }
        }
    }

    /// Where this run's conflicted merge stands right now. The modal asks git
    /// after a resolver exits instead of re-running the merge: a merge in
    /// progress is a dirty checkout, so a second `merge_task` could only ever
    /// report that as an error.
    pub fn merge_status(&self, id: &str) -> anyhow::Result<agency_core::merge::MergeState> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        require_own_branch(&run, &repo, "merge")?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
        agency_core::merge::merge_state(&repo, &run.branch, &base)
    }

    /// Commit a resolved merge (or accept one the resolver already committed)
    /// and restore the checkout's original branch, then do the same
    /// bookkeeping a clean first-try merge does.
    pub fn finish_merge_task(&self, id: &str) -> anyhow::Result<agency_core::merge::MergeOutcome> {
        self.finish_merge_task_with_progress(id, &mut |_| {})
    }

    /// [`finish_merge_task`] reporting each step, and holding the project's
    /// checkout gate for the same reason [`merge_task_with_progress`] does:
    /// committing a merge and putting the checkout back on its old branch both
    /// rewrite the working tree.
    pub fn finish_merge_task_with_progress(
        &self,
        id: &str,
        on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress),
    ) -> anyhow::Result<agency_core::merge::MergeOutcome> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        require_own_branch(&run, &repo, "merge")?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
        let finished = self.repo_gates.try_with(&run.project_id, || {
            step(on_progress, "Checking the merge", &run.branch);
            let state = agency_core::merge::merge_state(&repo, &run.branch, &base)?;
            if let Some(other) = &state.blocked_by {
                bail!(
                    "the merge in progress is {other}'s, not this run's — finish or abort it there"
                );
            }
            if !state.merging && !state.merged {
                bail!("no merge in progress for this run, and its branch hasn't landed on {base}");
            }
            let restore = self.merge_origins.lock().unwrap().remove(id);
            step(on_progress, "Completing the merge", &run.branch);
            let commit = agency_core::merge::finish_merge(&repo, restore.as_deref())?;
            step(on_progress, "Tidying up", &run.branch);
            self.after_merge_landed(&run, &repo);
            Ok(agency_core::merge::MergeOutcome::Clean { commit })
        });
        finished.unwrap_or_else(|| Err(anyhow!(busy_checkout_error())))
    }

    pub fn abort_merge_task(&self, id: &str) -> anyhow::Result<()> {
        self.abort_merge_task_with_progress(id, &mut |_| {})
    }

    /// [`abort_merge_task`] reporting each step, under the same gate: undoing a
    /// merge rewrites the working tree back, which is as slow as making it.
    pub fn abort_merge_task_with_progress(
        &self,
        id: &str,
        on_progress: &mut dyn FnMut(agency_core::setup::CloneProgress),
    ) -> anyhow::Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        require_own_branch(&run, &repo, "merge")?;
        let aborted = self.repo_gates.try_with(&run.project_id, || {
            step(on_progress, "Checking the merge", &run.branch);
            // Aborting throws away whatever resolution has been done so far, so it
            // must never reach across runs: the shared checkout means this run's
            // Abort would otherwise discard another run's half-resolved merge.
            if let Some(m) = agency_core::merge::in_progress_merge(&repo) {
                if !agency_core::merge::owns_merge(&repo, &run.branch) {
                    bail!(
                        "the merge in progress is {}'s, not this run's — abort it there",
                        m.branch
                    );
                }
            }
            let restore = self.merge_origins.lock().unwrap().remove(id);
            step(on_progress, "Undoing the merge", &run.branch);
            agency_core::merge::abort_merge(&repo, restore.as_deref())
        });
        aborted.unwrap_or_else(|| Err(anyhow!(busy_checkout_error())))
    }

    // ── pull requests (gh CLI) ─────────────────────────────────────────────────

    pub fn gh_readiness(&self, project_id: &str) -> Result<agency_core::gh::GhReadiness> {
        let repo = self.project_repo(project_id)?;
        Ok(agency_core::gh::GhCli::default().readiness(&repo))
    }

    /// Push the run's branch and open a PR against its merge target. The PR
    /// title is the run's title (or branch name) and the body is generated
    /// from the branch's commits. Idempotent-ish: if a PR already exists for
    /// the branch, gh fails and the existing PR is returned instead.
    pub fn create_pr(&self, id: &str) -> Result<agency_core::gh::PrInfo> {
        let run = self.run_record(id)?;
        if run.kind != "agent" {
            bail!("only agent runs have a branch to open a PR for");
        }
        let repo = self.project_repo(&run.project_id)?;
        require_own_branch(&run, &repo, "open a PR for")?;
        require_branch_exists(&run, &repo)?;
        let worktree = workspace_dir(&repo, &run);
        if !worktree.exists() {
            bail!("workspace is archived — restore it before creating a PR");
        }
        let gh = agency_core::gh::GhCli::default();
        agency_core::git::push(&worktree)?;
        let pr = match gh.view_pr(&repo, &run.branch)? {
            Some(existing) => existing,
            None => {
                let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
                let title = run
                    .title
                    .clone()
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| run.branch.clone());
                let body = agency_core::git::branch_summary(&repo, &run.branch, &base)?;
                gh.create_pr(&repo, &run.branch, &base, &title, &body)?
            }
        };
        // A PR means the work awaits review. Best-effort: the PR exists
        // either way, so an issue-status hiccup must not fail the call.
        if let Some(issue_id) = &run.issue_id {
            let reg = self.registry.lock().unwrap();
            if let Err(e) = self.advance_issue(&reg, issue_id, IssueStatus::InReview) {
                log::warn!("advancing issue {issue_id} to in_review: {e}");
            }
        }
        Ok(pr)
    }

    /// The run's PR (if any) plus its check rollup, polled by the UI.
    pub fn pr_status(&self, id: &str) -> Result<PrStatus> {
        let run = self.run_record(id)?;
        // A run in the main checkout shares the user's branch; any PR open on
        // it belongs to them, not to this run.
        if !run.worktree {
            return Ok(PrStatus { pr: None, checks: Vec::new() });
        }
        let repo = self.project_repo(&run.project_id)?;
        let gh = agency_core::gh::GhCli::default();
        let pr = gh.view_pr(&repo, &run.branch)?;
        let checks = match &pr {
            Some(_) => gh.pr_checks(&repo, &run.branch).unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(PrStatus { pr, checks })
    }

    // ── In-app PR review (keyed by project + PR number) ─────────────────────────
    // These power the Source Control → Pull Requests review surface. Identity is
    // (project_id, number): a PR list already has both, and the agent's Approve
    // window resolves its number once via `pr_number_for_run`.

    /// Full PR detail (description, head SHA, mergeable, review decision).
    pub fn pr_detail(
        &self,
        project_id: &str,
        number: u64,
    ) -> Result<Option<agency_core::gh::PrDetail>> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().view_pr_detail(&repo, number)
    }

    /// The authenticated gh user's login (for self-review detection).
    pub fn gh_current_login(&self, project_id: &str) -> Result<String> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().current_login(&repo)
    }

    /// Which merge methods the repo allows (drives the merge dialog's options).
    pub fn pr_merge_methods(&self, project_id: &str) -> Result<agency_core::gh::MergeMethods> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().merge_methods(&repo)
    }

    /// Merge a PR from the review view (the solo-dev path — you can't approve
    /// your own PR but can merge it).
    pub fn merge_pr(
        &self,
        project_id: &str,
        number: u64,
        method: &str,
        delete_branch: bool,
    ) -> Result<agency_core::gh::PrMergeResult> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().merge_pr(&repo, number, method, delete_branch)
    }

    /// The PR's full multi-file diff, split per file for the diff renderer.
    pub fn pr_diff(
        &self,
        project_id: &str,
        number: u64,
    ) -> Result<Vec<agency_core::gh::PrFileDiff>> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().pr_diff(&repo, number)
    }

    /// The PR's review threads (with resolution state) for inline rendering.
    pub fn pr_review_threads(
        &self,
        project_id: &str,
        number: u64,
    ) -> Result<Vec<agency_core::gh::ReviewThread>> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().pr_review_threads(&repo, number)
    }

    /// Submit a review verdict + inline comments as one atomic operation. The
    /// head SHA the comments anchor to is read here rather than trusted from the
    /// client — the reviews API rejects a stale `commit_id`.
    pub fn submit_pr_review(
        &self,
        project_id: &str,
        number: u64,
        event: &str,
        body: Option<&str>,
        comments: &[agency_core::gh::DraftComment],
    ) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        let gh = agency_core::gh::GhCli::default();
        let detail =
            gh.view_pr_detail(&repo, number)?.ok_or_else(|| anyhow!("PR #{number} not found"))?;
        gh.submit_pr_review(&repo, number, &detail.head_ref_oid, event, body, comments)
    }

    /// Reply into an existing review thread. `in_reply_to` is a comment's
    /// `database_id` from `pr_review_threads`.
    pub fn reply_pr_comment(
        &self,
        project_id: &str,
        number: u64,
        in_reply_to: u64,
        body: &str,
    ) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().reply_review_comment(&repo, number, in_reply_to, body)
    }

    /// Resolve a review thread. `thread_id` is the GraphQL node id.
    pub fn resolve_pr_thread(&self, project_id: &str, thread_id: &str) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().resolve_review_thread(&repo, thread_id)
    }

    /// Reopen a resolved review thread.
    pub fn unresolve_pr_thread(&self, project_id: &str, thread_id: &str) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().unresolve_review_thread(&repo, thread_id)
    }

    /// Open a PR from an existing branch in the project repo (the Pull Requests
    /// tab's "New pull request" flow, not tied to any run). Pushes the branch
    /// first, defaults the base to the repo's merge target and the title/body
    /// from the branch when not given. If a PR already exists for the branch it
    /// is returned instead of erroring.
    pub fn create_pr_from_branch(
        &self,
        project_id: &str,
        head: &str,
        base: Option<&str>,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<agency_core::gh::PrInfo> {
        let repo = self.project_repo(project_id)?;
        let gh = agency_core::gh::GhCli::default();
        if let Some(existing) = gh.view_pr(&repo, head)? {
            return Ok(existing);
        }
        let base = match base {
            Some(b) if !b.trim().is_empty() => b.to_string(),
            _ => agency_core::merge::resolve_target(None, &repo)?,
        };
        if head == base {
            bail!("a PR's branch and base must differ (both are {base})");
        }
        // Only a local branch needs publishing. A head that exists solely on
        // origin (someone else's branch, or one pushed from another checkout)
        // is already there, and `git push origin <name>` would fail on it with
        // "src refspec does not match any".
        if agency_core::git::local_branch_exists(&repo, head) {
            agency_core::git::push_branch(&repo, head)?;
        } else if !agency_core::git::remote_branch_exists(&repo, head) {
            bail!("branch {head} exists neither locally nor on origin");
        }
        let title = match title {
            Some(t) if !t.trim().is_empty() => t.to_string(),
            _ => head.to_string(),
        };
        let body = match body {
            Some(b) if !b.trim().is_empty() => b.to_string(),
            _ => agency_core::git::branch_summary(&repo, head, &base)?,
        };
        gh.create_pr(&repo, head, &base, &title, &body)
    }

    /// The PR number for a run's branch, if a PR exists — lets the agent's
    /// Approve window deep-link into the review view.
    pub fn pr_number_for_run(&self, id: &str) -> Result<Option<u64>> {
        let run = self.run_record(id)?;
        // Same reasoning as `pr_status`: the checkout's branch is not this
        // run's, so a PR on it isn't this run's either.
        if !run.worktree {
            return Ok(None);
        }
        let repo = self.project_repo(&run.project_id)?;
        Ok(agency_core::gh::GhCli::default().view_pr(&repo, &run.branch)?.map(|p| p.number))
    }

    /// Type the PR's failing checks into the agent's live session so it can
    /// investigate — same delivery path as review comments.
    pub fn send_check_feedback(&self, id: &str) -> Result<()> {
        let status = self.pr_status(id)?;
        let failing: Vec<_> =
            status.checks.iter().filter(|c| c.bucket == "fail" || c.bucket == "cancel").collect();
        if failing.is_empty() {
            bail!("no failing checks to send");
        }
        if !matches!(
            self.term.read().unwrap().status(&session_name(id)),
            Ok(SessionStatus::Running)
        ) {
            bail!("agent session {id} is not running");
        }
        let mut msg =
            format!("CI feedback: {} check(s) failing on this branch's PR — ", failing.len());
        let parts: Vec<String> = failing
            .iter()
            .map(|c| {
                if c.link.is_empty() {
                    c.name.clone()
                } else {
                    format!("{} ({})", c.name, c.link)
                }
            })
            .collect();
        msg.push_str(&parts.join("; "));
        msg.push_str(
            ". Please investigate the failures, fix them, commit, and push to update the PR.",
        );
        self.term.read().unwrap().send_text(&session_name(id), &msg)?;
        Ok(())
    }

    /// Hand a conflicted merge to the run's own agent by typing a prompt into
    /// its live session, the same delivery path as review comments and CI
    /// feedback. Replaces the old one-shot resolver process, which spawned a
    /// second, context-free agent that had no idea what the branch was for.
    pub fn send_merge_conflict(&self, id: &str) -> anyhow::Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)
            .unwrap_or_else(|_| "main".to_string());
        match agency_core::merge::in_progress_merge(&repo) {
            None => bail!("no merge is in progress, so there is no conflict to send"),
            // Another run's conflict describes files this agent never touched;
            // handing it that prompt would send it off editing someone else's work.
            Some(m) if !agency_core::merge::owns_merge(&repo, &run.branch) => {
                bail!("the merge in progress is {}'s, not this run's", m.branch)
            }
            Some(_) => {}
        }
        if !matches!(
            self.term.read().unwrap().status(&session_name(id)),
            Ok(SessionStatus::Running)
        ) {
            bail!("agent session {id} is not running");
        }
        let msg = compose_merge_conflict(&repo, &run.branch, &base, &conflict_status(&repo));
        self.term.read().unwrap().send_text(&session_name(id), &msg)?;
        Ok(())
    }

    /// Update focus/active-run state. On an unfocused→focused edge, hand back a
    /// still-fresh pending notification target (consuming it) so the caller can
    /// deep-link the UI to the run the user was just notified about.
    pub fn set_ui_state(
        &self,
        focused: bool,
        active_run: Option<String>,
    ) -> Option<(String, String)> {
        let mut ui = self.ui.lock().unwrap();
        let was_focused = ui.focused;
        ui.focused = focused;
        ui.active_run = active_run;
        if focused && !was_focused {
            if let Some(p) = ui.pending_open.take() {
                if p.at.elapsed() < PENDING_OPEN_TTL {
                    return Some((p.project_id, p.run_id));
                }
            }
        }
        None
    }

    /// Record the run a just-shown notification is about (see [`PendingOpen`]).
    pub fn note_notification(&self, project_id: &str, run_id: &str) {
        let mut ui = self.ui.lock().unwrap();
        ui.pending_open = Some(PendingOpen {
            project_id: project_id.to_string(),
            run_id: run_id.to_string(),
            at: std::time::Instant::now(),
        });
    }

    pub fn ui_snapshot(&self) -> (bool, Option<String>) {
        let ui = self.ui.lock().unwrap();
        (ui.focused, ui.active_run.clone())
    }

    pub fn add_review_comment(
        &self,
        run_id: &str,
        path: &str,
        line_start: u32,
        line_end: u32,
        body: &str,
    ) -> Result<agency_core::registry::ReviewComment> {
        let comment = agency_core::registry::ReviewComment {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: run_id.to_string(),
            path: path.to_string(),
            line_start,
            line_end,
            body: body.to_string(),
            sent: false,
            created_at: now_secs(),
        };
        self.registry.lock().unwrap().insert_review_comment(&comment)?;
        Ok(comment)
    }

    pub fn list_review_comments(
        &self,
        run_id: &str,
    ) -> Result<Vec<agency_core::registry::ReviewComment>> {
        self.registry.lock().unwrap().list_review_comments(run_id)
    }

    pub fn delete_review_comment(&self, id: &str) -> Result<()> {
        self.registry.lock().unwrap().delete_review_comment(id)
    }

    /// Type the unsent comments into the agent's live session and mark them sent.
    pub fn send_review_comments(&self, run_id: &str) -> Result<()> {
        let unsent = self.registry.lock().unwrap().list_unsent_review_comments(run_id)?;
        if unsent.is_empty() {
            bail!("no unsent review comments");
        }
        if !matches!(
            self.term.read().unwrap().status(&session_name(run_id)),
            Ok(SessionStatus::Running)
        ) {
            bail!("agent session {run_id} is not running");
        }
        let message = compose_feedback(&unsent);
        self.term.read().unwrap().send_text(&session_name(run_id), &message)?;
        self.registry.lock().unwrap().mark_review_comments_sent(run_id)?;
        Ok(())
    }

    /// Tell the daemon to shut down without killing sessions first. The daemon
    /// intentionally outlives the app (sessions survive restarts), which means
    /// every `AppState::new` in a test would otherwise leak a daemon process.
    pub fn shutdown_daemon(&self) {
        let _ = self.term.read().unwrap().shutdown();
    }

    /// Test-only teardown: kill every session this state started, then shut the
    /// daemon down, so no test leaks a daemon (or its session child processes).
    /// The integration-test harness (`tests/common`) calls this from a drop
    /// guard so cleanup runs even when a test panics mid-way.
    pub fn test_teardown(&self) {
        self.kill_all_and_shutdown();
    }

    /// Count of all sessions currently tracked by the daemon (used for the quit
    /// confirmation message). Returns 0 on any error.
    pub(crate) fn session_count(&self) -> usize {
        self.term.read().unwrap().list().map(|s| s.len()).unwrap_or(0)
    }

    /// Kill every daemon session, then tell the daemon to shut down.
    /// Called from the quit confirmation flow; errors are swallowed because we
    /// are about to exit anyway.
    pub(crate) fn kill_all_and_shutdown(&self) {
        let sessions = self.term.read().unwrap().list();
        if let Ok(sessions) = sessions {
            for (id, _) in sessions {
                let _ = self.term.read().unwrap().kill(&id);
            }
        }
        let _ = self.term.read().unwrap().shutdown();
    }

    /// Whether Agency checks GitHub for a newer release on launch. Defaults to
    /// on so beta testers hear about fixes; the toggle lives in Settings and
    /// only ever suppresses the automatic check — the manual button still works.
    pub fn update_check_enabled(&self) -> Result<bool> {
        let raw = self.registry.lock().unwrap().get_setting(SETTING_UPDATE_CHECK)?;
        Ok(raw.as_deref() != Some("0"))
    }

    pub fn set_update_check_enabled(&self, enabled: bool) -> Result<()> {
        self.registry
            .lock()
            .unwrap()
            .set_setting(SETTING_UPDATE_CHECK, if enabled { "1" } else { "0" })
    }

    pub fn notif_settings(&self) -> Result<notifier::NotifSettings> {
        let raw = self.registry.lock().unwrap().get_setting(SETTING_NOTIF)?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    pub fn save_notif_settings(&self, s: &notifier::NotifSettings) -> Result<()> {
        let mut s = s.clone();
        s.idle_secs = s.idle_secs.max(5);
        let json = serde_json::to_string(&s)?;
        self.registry.lock().unwrap().set_setting(SETTING_NOTIF, &json)
    }

    /// Snapshot every non-archived run across all projects for the watcher:
    /// agent + run-script session status and a hash of the agent pane (for idle).
    pub fn watch_snapshot(&self) -> Result<Vec<notifier::RunSnapshot>> {
        // Detect a dropped daemon and attempt a single respawn. The read guard on
        // `is_alive()` is a temporary that is released at the end of the `if`
        // condition, so the `write()` swap below cannot deadlock against it.
        if !self.term.read().unwrap().is_alive() {
            match TermClient::connect_or_spawn(termd_socket(&self.data_dir), termd_bin()) {
                Ok(client) => {
                    let n = client.list().map(|s| s.len()).unwrap_or(0);
                    log::warn!("termd reconnected; adopted {n} surviving session(s)");
                    *self.term.write().unwrap() = client;
                }
                Err(e) => log::error!("termd unavailable (agents may have stopped): {e}"),
            }
        }
        // One daemon round-trip for every run script in the app: each workspace
        // can have several, and asking per script per run would multiply this
        // tick by the size of the board.
        let live = self.term.read().unwrap().list().unwrap_or_default();
        let run_scripts_of = |target: &str| run_script_statuses_from(target, &live);

        let projects = self.registry.lock().unwrap().list_projects()?;
        let mut out = Vec::new();
        for proj in projects {
            // The project's own checkout runs scripts too (its Run tab), and a
            // release build failing while you're off in another app is exactly
            // the thing worth a toast. It has no agent, so only the run-script
            // edge can fire for it.
            let project_target = format!("project:{}", proj.id);
            out.push(notifier::RunSnapshot {
                run_scripts: run_scripts_of(&project_target),
                id: project_target,
                project_id: proj.id.clone(),
                label: proj.name.clone(),
                is_terminal: false,
                is_loop: false,
                agent: SessionStatus::Gone,
                pane_hash: 0,
                user_input_pending: false,
            });
            let runs = self.registry.lock().unwrap().list_runs(&proj.id)?;
            for run in runs {
                let agent = self
                    .term
                    .read()
                    .unwrap()
                    .status(&session_name(&run.id))
                    .unwrap_or(SessionStatus::Gone);
                let run_scripts = run_scripts_of(&run.id);
                let pane = self
                    .term
                    .read()
                    .unwrap()
                    .capture(&session_name(&run.id), 50)
                    .unwrap_or_default();
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&pane, &mut hasher);
                let pane_hash = std::hash::Hasher::finish(&hasher);
                let label = format!(
                    "{}: {}",
                    run.agent,
                    run.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| {
                        if run.prompt.is_empty() {
                            run.branch.clone()
                        } else {
                            run.prompt.clone()
                        }
                    })
                );
                let user_input_pending = self.input_seen.lock().unwrap().contains(&run.id);
                // Any run with a loop config, active OR terminal: suppression
                // must not depend on when the driver persists the terminal
                // transition, or the final attempt's exit edge (which lands on
                // the same tick) would toast "Agent exited" next to the loop's
                // own complete/stalled notification.
                let is_loop = run.loop_config.is_some();
                out.push(notifier::RunSnapshot {
                    id: run.id,
                    project_id: proj.id.clone(),
                    label,
                    is_terminal: run.kind == "terminal",
                    is_loop,
                    agent,
                    run_scripts,
                    pane_hash,
                    user_input_pending,
                });
            }
        }
        Ok(out)
    }
}

/// The install script for the graphify tooling, adapted to this machine: it
/// picks up uv first when uv isn't there yet. Probed at call time rather than
/// cached, so a uv installed since app start is noticed.
fn graphify_install_script() -> String {
    agency_core::config::graphify_install_script(command_on_path("uv"))
}

/// The graphify MCP server to hand a workspace, or `Err(reason)` explaining why
/// there is nothing to hand it. Pure over the two probes (does the graph file
/// exist, is the command on PATH) so the skip rules are testable without a repo.
///
/// Two things have to hold. The command must resolve — otherwise every agent
/// launch spawns a server that immediately ENOENTs. And, for the default serve
/// command, the graph it reads must already have been built: pointing
/// `graphify.serve` at a `graph.json` that was never written hands every agent a
/// server that dies on startup, which is worse than no server at all. A
/// user-supplied serve command is trusted to know where its own graph lives.
fn graphify_server(
    repo: &Path,
    knowledge: &agency_core::config::KnowledgeConfig,
    graph_exists: impl Fn(&Path) -> bool,
    on_path: impl Fn(&str) -> bool,
) -> std::result::Result<agency_core::mcp::McpServer, String> {
    // An argv, not a whitespace split: the default serve command embeds the
    // primary repo's absolute graph.json path, which would break graphify
    // injection for any repo path containing a space.
    let argv = knowledge
        .serve_command
        .as_deref()
        .map(agency_core::config::split_command)
        .unwrap_or_else(|| agency_core::config::default_serve_argv(repo));
    let mut parts = argv.into_iter();
    let cmd = parts.next().ok_or_else(|| "the serve command is empty".to_string())?;
    if knowledge.serve_command.is_none() {
        let graph = agency_core::config::graph_path(repo);
        if !graph_exists(&graph) {
            return Err(format!("{} has not been built", graph.display()));
        }
    }
    if !on_path(&cmd) {
        return Err(format!("'{cmd}' is not installed"));
    }
    Ok(agency_core::mcp::McpServer {
        name: "graphify".to_string(),
        command: Some(cmd),
        args: parts.collect(),
        ..Default::default()
    })
}

/// The last few lines a failed command wrote, preferring stderr and falling back
/// to stdout — enough to show the user *why* a build failed without pasting a
/// whole log into the settings panel. `None` when the command said nothing.
fn failure_tail(stderr: &[u8], stdout: &[u8]) -> Option<String> {
    const LINES: usize = 4;
    [stderr, stdout].into_iter().find_map(|bytes| {
        let text = String::from_utf8_lossy(bytes);
        let lines: Vec<&str> =
            text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(LINES)..].join("\n");
        (!tail.is_empty()).then_some(tail)
    })
}

/// True when `command` resolves to an executable file: checked directly when it
/// contains a path separator, otherwise searched across the PATH directories.
/// Whether the first whitespace token of a command line resolves to an
/// executable on PATH (or a runnable absolute/relative path). Command overrides
/// and the graphify defaults are full command lines, not bare binaries.
fn first_token_on_path(command_line: &str) -> bool {
    command_line.split_whitespace().next().map(command_on_path).unwrap_or(false)
}

fn command_on_path(command: &str) -> bool {
    fn executable(p: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            p.is_file()
                && p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            p.is_file()
        }
    }
    if command.contains('/') {
        return executable(Path::new(command));
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| executable(&dir.join(command))))
        .unwrap_or(false)
}

/// Any existing folder is addable as a project. Git is what unlocks worktrees
/// (and everything downstream: branches, merges, races, loops), not what makes
/// a folder a project — a plain scratch folder still gets agents, they just
/// work in it directly. `create_run_spec` is where the git-shaped requests are
/// refused, and the UI hides them ahead of that.
fn validate_project_path(repo_path: &Path) -> Result<()> {
    if !repo_path.exists() {
        bail!("{} does not exist", repo_path.display());
    }
    if !repo_path.is_dir() {
        bail!("{} is not a folder", repo_path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        agent_argv, command_on_path, failure_tail, graphify_server, new_task_id, pick_port,
        require_branch_exists, require_gitless_known, require_own_branch, slugify,
        split_session_id,
    };
    use agency_core::config::KnowledgeConfig;
    use agency_core::profile::AgentProfile;
    use std::collections::HashSet;

    /// AGE-83: enabling the knowledge graph looked like it did nothing. Nothing
    /// built the first graph, and the serve command was injected anyway — every
    /// agent got a graphify MCP server pointed at a `graph.json` that had never
    /// been written. No graph, no server.
    #[test]
    fn graphify_is_not_injected_before_the_graph_is_built() {
        let repo = Path::new("/repo");
        let enabled = KnowledgeConfig { graph: true, ..Default::default() };

        let err = graphify_server(repo, &enabled, |_| false, |_| true).unwrap_err();
        assert!(err.contains("has not been built"), "{err}");
        assert!(err.contains("graph.json"), "the reason names the file the user must build: {err}");

        let server = graphify_server(repo, &enabled, |_| true, |_| true).unwrap();
        assert_eq!(server.command.as_deref(), Some("uv"));
        assert_eq!(server.args.last().unwrap(), "/repo/graphify-out/graph.json");
    }

    #[test]
    fn graphify_is_not_injected_when_the_tooling_is_missing() {
        let enabled = KnowledgeConfig { graph: true, ..Default::default() };
        let err = graphify_server(Path::new("/repo"), &enabled, |_| true, |_| false).unwrap_err();
        assert!(err.contains("'uv' is not installed"), "{err}");
    }

    /// A hand-written serve command may read a graph from anywhere, so the
    /// default `graphify-out/graph.json` check must not gate it.
    #[test]
    fn a_custom_serve_command_is_not_gated_on_the_default_graph_path() {
        let custom = KnowledgeConfig {
            graph: true,
            serve_command: Some("my-server \"/my graphs/g.json\"".to_string()),
            build_command: None,
        };
        let server = graphify_server(Path::new("/repo"), &custom, |_| false, |_| true).unwrap();
        assert_eq!(server.command.as_deref(), Some("my-server"));
        assert_eq!(server.args, vec!["/my graphs/g.json".to_string()]);
    }

    #[test]
    fn failure_tail_prefers_stderr_and_keeps_the_last_lines() {
        assert_eq!(failure_tail(b"boom", b"noise").as_deref(), Some("boom"));
        assert_eq!(failure_tail(b"", b"only stdout").as_deref(), Some("only stdout"));
        assert_eq!(failure_tail(b"  \n\n", b"  \n").as_deref(), None, "whitespace is not a reason");
        assert_eq!(
            failure_tail(b"a\nb\nc\nd\ne\nf\n", b"").as_deref(),
            Some("c\nd\ne\nf"),
            "the tail is what says why it failed"
        );
    }

    #[test]
    fn command_on_path_finds_shell_binaries() {
        assert!(command_on_path("sh"), "sh must be on PATH");
        assert!(command_on_path("/bin/sh"), "absolute path to sh");
        assert!(!command_on_path("definitely-not-a-real-binary-4k2x"));
        assert!(!command_on_path("/nonexistent/path/to/agent"));
    }

    /// A path with no MCP config in it, for argv tests about everything else:
    /// with no file to point at, the MCP launch flags stay out of the way.
    fn no_worktree() -> &'static Path {
        Path::new("/nonexistent/agency-argv-test-worktree")
    }

    #[test]
    fn agent_argv_uses_resume_args_when_available() {
        let p = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec!["{{prompt}}".into()],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let (cmd, args) = agent_argv(&p, no_worktree(), "do the thing", true, None);
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["--continue".to_string()]);
    }

    #[test]
    fn agent_argv_falls_back_to_prompt_without_resume_args() {
        let p = AgentProfile {
            name: "cursor".into(),
            command: "cursor-agent".into(),
            args: vec!["{{prompt}}".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        };
        let (cmd, args) = agent_argv(&p, no_worktree(), "hello", true, None);
        assert_eq!(cmd, "cursor-agent");
        assert_eq!(args, vec!["hello".to_string()]);
    }

    #[test]
    fn agent_argv_fresh_ignores_resume_args() {
        let p = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec!["{{prompt}}".into()],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let (_cmd, args) = agent_argv(&p, no_worktree(), "fresh prompt", false, None);
        assert_eq!(args, vec!["fresh prompt".to_string()]);
    }

    #[test]
    fn fresh_agent_argv_appends_prompt_only_without_a_token() {
        let templated = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec!["--flag".into(), "{{prompt}}".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        };
        // The token places the prompt; it must not also be appended.
        let (cmd, args) = super::fresh_agent_argv(&templated, no_worktree(), "review it", None);
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["--flag".to_string(), "review it".to_string()]);

        let plain = AgentProfile { args: vec!["--flag".into()], ..templated.clone() };
        let (_cmd, args) = super::fresh_agent_argv(&plain, no_worktree(), "review it", None);
        assert_eq!(args, vec!["--flag".to_string(), "review it".to_string()]);

        // An empty prompt is the promptless launch every ordinary tab uses.
        let (_cmd, args) = super::fresh_agent_argv(&plain, no_worktree(), "", None);
        assert_eq!(args, vec!["--flag".to_string()]);
    }

    /// AGE-79: the prompt only rides as a positional argument for the CLIs that
    /// actually take one. Copilot rejects the whole launch without `-i`, and
    /// opencode reads a positional as a directory to open.
    #[test]
    fn fresh_argv_delivers_the_prompt_the_way_each_cli_takes_it() {
        let profile = |name: &str| AgentProfile {
            name: name.into(),
            command: name.into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        };
        let args_for = |name: &str, prompt: &str| {
            super::fresh_agent_argv(&profile(name), no_worktree(), prompt, None).1
        };

        assert_eq!(args_for("claude", "go"), vec!["go".to_string()]);
        assert_eq!(args_for("cursor", "go"), vec!["go".to_string()]);
        assert_eq!(args_for("copilot", "go"), vec!["-i".to_string(), "go".to_string()]);
        assert_eq!(args_for("opencode", "go"), vec!["--prompt".to_string(), "go".to_string()]);
        // crush parses a prompt as a subcommand and has no flag for one, so the
        // session comes up promptless rather than dying on "Unknown command".
        assert!(args_for("crush", "go").is_empty());
        // A custom profile Agency knows nothing about keeps the positional default.
        assert_eq!(args_for("my-agent", "go"), vec!["go".to_string()]);

        // None of it fires for a promptless launch: no stray `-i` with no value.
        for agent in ["claude", "copilot", "opencode", "crush"] {
            assert!(args_for(agent, "").is_empty(), "{agent}");
            assert!(args_for(agent, "   ").is_empty(), "{agent}");
        }

        // A `{{prompt}}` token in the user's own args still wins outright.
        let hand_rolled = AgentProfile {
            args: vec!["--interactive".into(), "{{prompt}}".into()],
            ..profile("copilot")
        };
        let (_cmd, args) = super::fresh_agent_argv(&hand_rolled, no_worktree(), "go", None);
        assert_eq!(args, vec!["--interactive".to_string(), "go".to_string()]);
    }

    /// AGE-71: Copilot ignores the `.mcp.json` Agency wrote until the user
    /// approves folder trust, and every run gets a brand-new worktree, so the
    /// file has to be handed over on the command line instead.
    #[test]
    fn copilot_argv_carries_the_emitted_mcp_config() {
        let dir = tempfile::tempdir().unwrap();
        let copilot = AgentProfile {
            name: "copilot".into(),
            command: "copilot".into(),
            args: vec![],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let flag = "--additional-mcp-config".to_string();
        let arg = format!("@{}", dir.path().join(".mcp.json").display());

        // Nothing emitted yet → no flag pointing at a file that isn't there.
        let (_cmd, args) = super::fresh_agent_argv(&copilot, dir.path(), "", None);
        assert!(args.is_empty(), "{args:?}");

        agency_core::mcp::emit_for_agent(
            "copilot",
            dir.path(),
            &[agency_core::mcp::McpServer {
                name: "kg".into(),
                command: Some("graphify".into()),
                ..Default::default()
            }],
        )
        .unwrap();

        // Fresh launch: flags first, prompt still last (behind `-i`, per AGE-79).
        let (cmd, args) = super::fresh_agent_argv(&copilot, dir.path(), "go", None);
        assert_eq!(cmd, "copilot");
        assert_eq!(args, vec![flag.clone(), arg.clone(), "-i".to_string(), "go".to_string()]);

        // Resume launch: the recipe keeps its own args and gains the config.
        let (_cmd, args) = agent_argv(&copilot, dir.path(), "go", true, None);
        assert_eq!(args, vec!["--continue".to_string(), flag.clone(), arg.clone()]);

        // A loop recipe gets it ahead of wherever it places the prompt.
        let looping = AgentProfile {
            loop_args: Some(vec!["-p".into(), "{{prompt}}".into()]),
            ..copilot.clone()
        };
        let (_cmd, args) = super::loop_argv(&looping, dir.path(), "go", None).unwrap();
        assert_eq!(args, vec!["-p".to_string(), flag.clone(), arg.clone(), "go".to_string()]);

        // The user's own flag wins: Agency doesn't add a rival copy.
        let hand_rolled =
            AgentProfile { args: vec![flag.clone(), "@/my/own.json".into()], ..copilot.clone() };
        let (_cmd, args) = super::fresh_agent_argv(&hand_rolled, dir.path(), "", None);
        assert_eq!(args, vec![flag.clone(), "@/my/own.json".to_string()]);

        // Agents that read the emitted file unaided get no extra flags.
        let claude = AgentProfile { name: "claude".into(), command: "claude".into(), ..copilot };
        let (_cmd, args) = super::fresh_agent_argv(&claude, dir.path(), "", None);
        assert!(args.is_empty(), "{args:?}");
    }

    #[test]
    fn pr_review_prompt_switches_on_post_comments() {
        let quiet = super::pr_review_prompt(12, "Add widgets", "https://x/pull/12", "main", false);
        assert!(quiet.contains("Review GitHub pull request #12: Add widgets"));
        assert!(quiet.contains("git diff main...HEAD"));
        assert!(quiet.contains("Do not post anything to GitHub"));
        assert!(!quiet.contains("gh api"));
        // Both modes promise the follow-up fixing session the review is for.
        assert!(quiet.contains("stay available"));

        let posting = super::pr_review_prompt(12, "Add widgets", "https://x/pull/12", "main", true);
        assert!(posting.contains("repos/$SLUG/pulls/12/reviews"));
        assert!(posting.contains("\"event\": \"COMMENT\""));
        assert!(!posting.contains("Do not post anything to GitHub"));
        assert!(posting.contains("stay available"));
    }

    #[test]
    fn issue_prompt_carries_the_issue_and_does_not_send_the_agent_reading() {
        let root = Path::new("/repo");
        let p = super::issue_prompt("AGE-14", "Fix login", "The button does nothing.", &[], root);
        assert!(p.starts_with("Work on issue AGE-14: Fix login\n\nThe button does nothing."));
        // The body is already in the prompt, so the agent must be told not to
        // go fetch it — this is what stopped a third of runs opening the file
        // as their very first tool call.
        assert!(p.contains("nothing to go and read"), "{p}");
        assert!(p.contains("/repo/.agency/issues/AGE-14.md"), "{p}");
        // The README is worth naming, but only as the follow-up path — never
        // as something to read on the way in.
        assert!(
            p.contains("To file a follow-up issue, read `/repo/.agency/issues/README.md`"),
            "{p}"
        );
        assert!(p.contains("marks AGE-14 done automatically"), "{p}");
    }

    #[test]
    fn issue_prompt_handles_an_empty_body_and_resolves_attachments() {
        let root = Path::new("/repo");
        let bare = super::issue_prompt("AGE-9", "Just a title", "  ", &[], root);
        assert!(
            bare.starts_with("Work on issue AGE-9: Just a title\n\nThat is the whole of AGE-9"),
            "{bare}"
        );

        // Relative `assets/…` links are dead paths from inside a worktree, so
        // the prompt resolves them against the project checkout.
        let shot =
            super::issue_prompt("AGE-9", "T", "before ![](assets/AGE-9-shot.png) after", &[], root);
        assert!(shot.contains("`/repo/.agency/issues/assets/AGE-9-shot.png`"), "{shot}");
        assert!(!bare.contains("assets/"), "{bare}");
    }

    #[test]
    fn with_model_pins_the_model_on_every_launch_path() {
        let claude = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec![],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: Some(vec![
                "-p".into(),
                "{{prompt}}".into(),
                "--permission-mode".into(),
                "acceptEdits".into(),
            ]),
        };
        let pinned = super::with_model(&claude, Some("opus"));

        // Fresh: ahead of the prompt, which is appended after these.
        let (_cmd, args) = super::fresh_agent_argv(&pinned, no_worktree(), "go", None);
        assert_eq!(args, vec!["--model", "opus", "go"]);
        // Resume: the same session must come back on the same model.
        let (_cmd, args) = agent_argv(&pinned, no_worktree(), "go", true, None);
        assert_eq!(args, vec!["--continue", "--model", "opus"]);
        // Loop: at the end, so {{prompt}} stays where the recipe put it.
        let (_cmd, args) = super::loop_argv(&pinned, no_worktree(), "go", None).unwrap();
        assert_eq!(args, vec!["-p", "go", "--permission-mode", "acceptEdits", "--model", "opus"]);
    }

    #[test]
    fn with_model_is_a_no_op_without_a_model_or_a_recipe() {
        let claude = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec!["--verbose".into()],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        assert_eq!(super::with_model(&claude, None), claude);
        // crush takes no model flag: pinning one must not invent an argument
        // that would stop its CLI from starting at all.
        let crush =
            AgentProfile { name: "crush".into(), command: "crush".into(), ..claude.clone() };
        assert_eq!(super::with_model(&crush, Some("anything")), crush);
    }

    #[test]
    fn push_mru_moves_the_model_to_the_front_and_caps_the_list() {
        assert_eq!(super::push_mru("", "opus"), "opus");
        assert_eq!(super::push_mru("opus", "sonnet"), "sonnet\nopus");
        // Re-picking an old model promotes it rather than duplicating it.
        assert_eq!(super::push_mru("sonnet\nopus", "opus"), "opus\nsonnet");
        let long = (0..10).map(|i| format!("m{i}")).collect::<Vec<_>>().join("\n");
        let capped = super::push_mru(&long, "new");
        assert_eq!(capped.lines().count(), super::MODEL_MRU_MAX);
        assert!(capped.starts_with("new\n"));
    }

    #[test]
    fn loop_argv_renders_prompt_and_requires_recipe() {
        let p = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: Some(vec![
                "-p".into(),
                "{{prompt}}".into(),
                "--permission-mode".into(),
                "acceptEdits".into(),
            ]),
        };
        let (cmd, args) = super::loop_argv(&p, no_worktree(), "fix the tests", None).unwrap();
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["-p", "fix the tests", "--permission-mode", "acceptEdits"]);

        let no_recipe = AgentProfile { loop_args: None, ..p.clone() };
        assert!(super::loop_argv(&no_recipe, no_worktree(), "x", None).is_err());

        // An empty recipe is no recipe — Settings saves None for an empty
        // field, but a hand-edited profile must not slip through.
        let empty_recipe = AgentProfile { loop_args: Some(vec![]), ..p };
        assert!(super::loop_argv(&empty_recipe, no_worktree(), "x", None).is_err());
    }

    #[test]
    fn loop_argv_appends_prompt_when_recipe_has_no_token() {
        // A recipe without {{prompt}} must still deliver the prompt (as the
        // final positional arg, like the interactive path) — never launch
        // promptless attempts that burn the loop budget doing nothing.
        let p = AgentProfile {
            name: "codex".into(),
            command: "codex".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: Some(vec!["exec".into(), "--full-auto".into()]),
        };
        let (cmd, args) = super::loop_argv(&p, no_worktree(), "fix the tests", None).unwrap();
        assert_eq!(cmd, "codex");
        assert_eq!(args, vec!["exec", "--full-auto", "fix the tests"]);
    }

    #[test]
    fn slugify_makes_readable_ref_safe_slugs() {
        assert_eq!(slugify("Fix the tmux resize bug!"), "fix-the-tmux-resize-bug");
        assert_eq!(slugify("  leading/trailing  "), "leading-trailing");
        assert_eq!(slugify("multi   space\tand\nnewline"), "multi-space-and-newline");
    }

    #[test]
    fn slugify_falls_back_when_no_usable_chars() {
        assert_eq!(slugify(""), "agent");
        assert_eq!(slugify("!!! ???"), "agent");
    }

    #[test]
    fn slugify_caps_length() {
        let slug = slugify(&"word ".repeat(40));
        assert!(slug.len() <= 40, "slug too long: {slug}");
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn new_task_id_is_slug_plus_suffix_and_ref_safe() {
        let id = new_task_id("Add a login page");
        assert!(id.starts_with("add-a-login-page-"), "unexpected id: {id}");
        assert!(id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    }

    /// `--` is the extra-session separator, so it must be impossible inside a
    /// task id — even for prompts full of dashes.
    #[test]
    fn task_ids_never_contain_double_hyphen() {
        for prompt in ["a--b", "-- -- --", "trailing- -leading", "dash - dash -- dash"] {
            let id = new_task_id(prompt);
            assert!(!id.contains("--"), "id contains --: {id}");
        }
    }

    #[test]
    fn split_session_id_roundtrips() {
        assert_eq!(split_session_id("fix-login-a3k2"), ("fix-login-a3k2", None));
        assert_eq!(split_session_id("fix-login-a3k2--2"), ("fix-login-a3k2", Some(2)));
        assert_eq!(split_session_id("fix-login-a3k2--12"), ("fix-login-a3k2", Some(12)));
        // Defensive: a non-numeric tail is not a tab id.
        assert_eq!(split_session_id("weird--tail"), ("weird--tail", None));
    }

    #[test]
    fn new_task_id_disambiguates_same_prompt() {
        let a = new_task_id("same prompt");
        let b = new_task_id("same prompt");
        assert_ne!(a, b);
    }

    #[test]
    fn pick_port_returns_base_when_unused() {
        let used = HashSet::new();
        assert_eq!(pick_port(&used, 5200, 10), Some(5200));
    }

    #[test]
    fn pick_port_skips_used_blocks_lowest_first() {
        let used: HashSet<u16> = [5200, 5210].into_iter().collect();
        assert_eq!(pick_port(&used, 5200, 10), Some(5220));
    }

    #[test]
    fn pick_port_fills_lowest_gap() {
        let used: HashSet<u16> = [5200, 5220].into_iter().collect();
        assert_eq!(pick_port(&used, 5200, 10), Some(5210));
    }

    #[test]
    fn pick_port_treats_zero_block_size_as_one() {
        let used: HashSet<u16> = [5200].into_iter().collect();
        // block_size 0 must not hang; it falls back to a stride of 1.
        assert_eq!(pick_port(&used, 5200, 0), Some(5201));
    }

    #[test]
    fn run_session_name_is_namespaced_per_workspace_and_script() {
        assert_eq!(
            super::run_session_name("fix-login-a3k2", "dev"),
            "agency-run-fix-login-a3k2#dev"
        );
        assert_eq!(super::run_session_name("project:p1", "build"), "agency-run-project:p1#build");
    }

    #[test]
    fn run_sessions_for_a_workspace_exclude_its_neighbours() {
        let live: Vec<String> = [
            // This workspace: the current form, plus the pre-AGE-34 unnamed one.
            "agency-run-fix-a1#dev",
            "agency-run-fix-a1#build mac",
            "agency-run-fix-a1",
            // A different run whose id merely starts the same way.
            "agency-run-fix-a12#dev",
            // The project's own checkout, and unrelated session kinds.
            "agency-run-project:p1#dev",
            "agency-fix-a1",
            "agency-shell-fix-a1",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let mut mine = super::run_session_names_for("fix-a1", &live);
        mine.sort();
        assert_eq!(
            mine,
            vec![
                "agency-run-fix-a1".to_string(),
                "agency-run-fix-a1#build mac".to_string(),
                "agency-run-fix-a1#dev".to_string(),
            ]
        );
        assert_eq!(
            super::run_session_names_for("project:p1", &live),
            vec!["agency-run-project:p1#dev".to_string()]
        );
    }

    #[test]
    fn run_script_statuses_key_by_script_name() {
        use agency_core::term::SessionStatus;
        let live = vec![
            ("agency-run-fix-a1#dev".to_string(), SessionStatus::Running),
            ("agency-run-fix-a1#build".to_string(), SessionStatus::Exited { code: 2 }),
            // Pre-AGE-34, before scripts had names.
            ("agency-run-fix-a1".to_string(), SessionStatus::Running),
            ("agency-run-other#dev".to_string(), SessionStatus::Running),
            ("agency-fix-a1".to_string(), SessionStatus::Running),
        ];
        let map = super::run_script_statuses_from("fix-a1", &live);
        assert_eq!(map.len(), 3);
        assert!(matches!(map["dev"], SessionStatus::Running));
        assert!(matches!(map["build"], SessionStatus::Exited { code: 2 }));
        assert!(matches!(map[agency_core::config::DEFAULT_RUN_NAME], SessionStatus::Running));
        // A workspace with nothing started reports nothing, so the crash
        // detector has no edge to invent.
        assert!(super::run_script_statuses_from("never-run", &live).is_empty());
    }

    #[test]
    fn run_scripts_live_only_while_something_is_running() {
        use agency_core::term::SessionStatus;
        let live = vec![
            ("agency-run-fix-a1#dev".to_string(), SessionStatus::Running),
            ("agency-run-fix-a1#build".to_string(), SessionStatus::Exited { code: 0 }),
            // A workspace whose only script has finished: the dot goes out.
            ("agency-run-fix-b2#build".to_string(), SessionStatus::Exited { code: 0 }),
            ("agency-run-project:p1#dev".to_string(), SessionStatus::Running),
            // The agent's own session — never a run script.
            ("agency-fix-c3".to_string(), SessionStatus::Running),
        ];
        assert!(super::any_run_script_live("fix-a1", &live));
        assert!(!super::any_run_script_live("fix-b2", &live));
        assert!(super::any_run_script_live("project:p1", &live));
        assert!(!super::any_run_script_live("fix-c3", &live));
        assert!(!super::any_run_script_live("never-run", &live));
    }

    use super::{compose_feedback, compose_merge_conflict};
    use agency_core::registry::ReviewComment;
    use std::path::Path;

    fn rc(path: &str, a: u32, b: u32, body: &str) -> ReviewComment {
        ReviewComment {
            id: "x".into(),
            run_id: "r".into(),
            path: path.into(),
            line_start: a,
            line_end: b,
            body: body.into(),
            sent: false,
            created_at: 0,
        }
    }

    #[test]
    fn compose_feedback_is_single_line_with_locations() {
        let msg = compose_feedback(&[
            rc("src/a.rs", 10, 12, "rename this"),
            rc("src/b.rs", 5, 5, "remove dead code"),
        ]);
        assert!(!msg.contains('\n'), "must be single-line");
        assert!(msg.contains("src/a.rs:10-12"));
        assert!(msg.contains("src/b.rs:5"));
        assert!(!msg.contains("src/b.rs:5-5"), "equal start/end shows one number");
        assert!(msg.contains("rename this"));
        assert!(msg.contains("remove dead code"));
    }

    #[test]
    fn compose_merge_conflict_is_single_line_and_names_the_checkout() {
        let msg = compose_merge_conflict(
            Path::new("/tmp/demo"),
            "agent/feature",
            "main",
            &["UU src/a.rs".to_string(), "UU src/b.rs".to_string()],
        );
        // A newline would submit the prompt half-typed: send_text appends the
        // carriage return itself.
        assert!(!msg.contains('\n') && !msg.contains('\r'), "must be single-line");
        assert!(msg.starts_with("Help me fix this merge conflict: "), "got: {msg}");
        assert!(msg.contains("UU src/a.rs | UU src/b.rs"), "got: {msg}");
        assert!(msg.contains("/tmp/demo"), "names the checkout the merge is in: {msg}");
        assert!(msg.contains("agent/feature") && msg.contains("main"), "got: {msg}");
    }

    #[test]
    fn compose_merge_conflict_survives_an_empty_status() {
        let msg = compose_merge_conflict(Path::new("/tmp/demo"), "b", "main", &[]);
        assert!(msg.contains("no unmerged files"), "got: {msg}");
    }

    #[test]
    fn compose_merge_conflict_caps_a_huge_file_list() {
        let files: Vec<String> = (0..60).map(|i| format!("UU src/f{i}.rs")).collect();
        let msg = compose_merge_conflict(Path::new("/tmp/demo"), "b", "main", &files);
        assert!(msg.contains("UU src/f39.rs"), "keeps the first 40: {msg}");
        assert!(!msg.contains("UU src/f40.rs"), "drops the rest: {msg}");
        assert!(msg.contains("and 20 more"), "says how many it dropped: {msg}");
    }

    #[test]
    fn compose_feedback_strips_newlines_in_body() {
        let msg = compose_feedback(&[rc("src/a.rs", 1, 1, "line one\nline two\r\nthree")]);
        assert!(!msg.contains('\n'), "no raw newlines");
        assert!(!msg.contains('\r'), "no carriage returns");
        assert!(msg.contains("line one line two"));
    }

    #[test]
    fn terminal_run_record_has_no_branch_and_terminal_kind() {
        // Shape check independent of the backend: a terminal Run carries kind="terminal",
        // an empty branch, and no port — the invariants discard_run/run_info rely on.
        let run = agency_core::registry::Run {
            id: new_task_id("terminal"),
            project_id: "proj".to_string(),
            agent: "terminal".to_string(),
            prompt: String::new(),
            base: String::new(),
            branch: String::new(),
            created_at: 0,
            port_base: None,
            archived_at: None,
            title: Some("terminal".to_string()),
            kind: "terminal".to_string(),
            merge_target: None,
            race_id: None,
            loop_config: None,
            loop_state: None,
            issue_id: None,
            worktree: false,
            model: None,
        };
        assert_eq!(run.kind, "terminal");
        assert!(run.branch.is_empty());
        assert!(run.port_base.is_none());
        assert!(run.id.starts_with("terminal-"));
    }

    /// A run that stays in the project checkout, shaped like every terminal:
    /// `worktree: false` and no branch of its own recorded.
    fn checkout_run() -> agency_core::registry::Run {
        agency_core::registry::Run {
            id: "terminal-x".to_string(),
            project_id: "proj".to_string(),
            agent: "terminal".to_string(),
            prompt: String::new(),
            base: String::new(),
            branch: String::new(),
            created_at: 0,
            port_base: None,
            archived_at: None,
            title: None,
            kind: "terminal".to_string(),
            merge_target: None,
            race_id: None,
            loop_config: None,
            loop_state: None,
            issue_id: None,
            worktree: false,
            model: None,
        }
    }

    fn init_repo_with_commit(dir: &Path) {
        for args in [
            &["init", "-q"][..],
            &["config", "user.email", "t@e.com"],
            &["config", "user.name", "T"],
            &["commit", "-q", "--allow-empty", "-m", "init"],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap()
                .success());
        }
    }

    #[test]
    fn require_branch_exists_names_the_missing_branch_not_git_syntax() {
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());

        let mut run = checkout_run();
        run.worktree = true;
        run.branch = "agent/gone".to_string();

        let err = require_branch_exists(&run, repo.path()).unwrap_err().to_string();
        assert!(err.contains("agent/gone"), "names the branch: {err}");
        assert!(err.contains("no longer exists"), "says what is wrong: {err}");
        assert!(err.contains("archive"), "says what to do next: {err}");
        assert!(!err.contains("ambiguous argument"), "no raw git error: {err}");

        // A branch that really is there passes.
        run.branch = agency_core::merge::current_branch(repo.path()).unwrap();
        assert!(require_branch_exists(&run, repo.path()).is_ok());

        // An empty stored branch counts as missing, not as a range against
        // nothing, which is what git would make of "main..".
        run.branch = String::new();
        assert!(require_branch_exists(&run, repo.path()).is_err());
    }

    #[test]
    fn require_own_branch_reads_the_checkout_not_the_stored_branch() {
        // Every terminal stores an empty branch, in a git repo or not, so the
        // refusal can't take that as evidence the folder has no repository.
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        let branch = agency_core::merge::current_branch(repo.path()).unwrap();

        let err =
            require_own_branch(&checkout_run(), repo.path(), "merge").unwrap_err().to_string();
        assert!(err.contains(&branch), "names the branch the user can see: {err}");
        assert!(!err.contains("not a git repository"), "the folder plainly is one: {err}");

        // Only a folder actually without a repository gets that wording.
        let plain = tempfile::tempdir().unwrap();
        let err =
            require_own_branch(&checkout_run(), plain.path(), "merge").unwrap_err().to_string();
        assert!(err.contains("not a git repository"), "got: {err}");

        // A run with its own worktree is never refused.
        let mut owned = checkout_run();
        owned.worktree = true;
        assert!(require_own_branch(&owned, repo.path(), "merge").is_ok());
    }

    #[test]
    fn require_gitless_known_refuses_to_guess_when_git_cannot_run() {
        let plain = tempfile::tempdir().unwrap();
        assert!(require_gitless_known(plain.path()).unwrap(), "a plain folder is gitless");

        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        assert!(!require_gitless_known(repo.path()).unwrap());

        // The folder is gone, so git can't answer. Guessing "gitless" here is
        // what would turn an agent loose in a real checkout, so this errors.
        let gone = plain.path().join("removed");
        let err = require_gitless_known(&gone).unwrap_err().to_string();
        assert!(err.contains("could not run git"), "got: {err}");
    }
}
