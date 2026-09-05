use crate::notifier;
use agency_core::buildlog::Stream;
use agency_core::cleanup::{BranchFacts, Disposal};
use agency_core::profile::AgentProfile;
use agency_core::registry::{IssueStatus, Project, Registry};
use agency_core::term::client::{Subscription, TermClient};
use agency_core::term::SessionStatus;
use agency_core::worktree::WorktreeManager;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
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

/// How a sync pass ended. "Needs seeding" is an outcome rather than an error
/// because it is a question for the user, not a failure: the UI has to be able
/// to tell it apart from a broken remote without reading an error message, and
/// it carries both counts so the prompt can state them.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SyncResult {
    Done { outcome: agency_core::issueref::Outcome },
    NeedsSeeding { local: usize, remote: usize },
}

/// A project's backlog-sharing config for the settings UI, plus the two facts
/// the UI needs to say what the choice actually means.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSyncDto {
    pub sync: bool,
    pub remote: String,
    /// The remotes this repo has, so the UI offers names rather than asking the
    /// user to remember them. Empty for a project with no git remote at all,
    /// which is the case where sync cannot be turned on usefully.
    pub remotes: Vec<String>,
    /// Set by the tracked `agency.toml` rather than this machine's local file.
    /// The UI says so, because turning it off here is a local override of a
    /// decision the repo made, not a change everyone sees.
    pub from_repo: bool,
}

/// A project's effective knowledge-graph config for the settings UI. Command
/// overrides are `None` when unset (the `*_default` fields show what runs then);
/// the `*_installed` flags report whether that tooling is actually on PATH.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeConfigDto {
    pub graph: bool,
    /// Rebuild the graph after every clean merge. The only thing here that
    /// spends a model budget without the user pressing anything, so it is a
    /// switch on the panel rather than a behaviour they discover from a bill.
    pub rebuild_on_merge: bool,
    pub serve_command: Option<String>,
    pub build_command: Option<String>,
    pub serve_default: String,
    pub build_default: String,
    pub serve_installed: bool,
    pub build_installed: bool,
    /// The models this machine can build a graph with, best first, each with
    /// the line about cost and destination the picker shows before a build.
    pub backends: Vec<agency_core::config::KnowledgeBackend>,
    /// Which of `backends` the effective build command names, or "custom" for
    /// a command the picker didn't write and won't touch.
    pub build_backend: String,
    /// The model named in that command, empty for the backend's own default.
    pub build_model: String,
    /// The graph file the serve command reads, and whether it exists yet. Until
    /// a build has produced it there is nothing to serve, so the MCP server is
    /// not injected — the UI says so rather than leaving the feature silent.
    pub graph_path: String,
    pub graph_built: bool,
    /// A build is running right now (the Build button, or a merge). The UI
    /// polls while this is true.
    pub building: bool,
    /// How long the running build has been going, and how long the last
    /// finished one took. A graph build of this repository takes ten minutes
    /// and printed nothing at all while it ran, which reads as a hung panel
    /// rather than a working build (AGE-180); the elapsed time is the cheapest
    /// half of the answer.
    pub build_elapsed_secs: Option<u64>,
    pub last_build_secs: Option<u64>,
    /// The tail of the running (or last) build's output, oldest first. The
    /// other half of the answer: what it is actually doing right now.
    pub build_log: Vec<String>,
    /// Why the last finished build failed, `None` if it succeeded, was stopped
    /// or none ran. A build the user stopped is not a failure, so it reports as
    /// `last_build_stopped` instead of inventing a reason.
    pub last_build_error: Option<String>,
    pub last_build_stopped: bool,
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
    /// When the running build started, and how long the last finished one ran.
    started: Option<Instant>,
    took: Option<Duration>,
    /// The tail of the build's output, shared with the threads reading its
    /// pipes. Kept after the build ends: a failure's reason and the last thing
    /// a stopped build managed to say are both in here.
    log: Arc<Mutex<agency_core::buildlog::BuildLog>>,
    /// The running child, so the user can stop a build that is going nowhere.
    /// `None` once it has been reaped.
    child: Option<Arc<Mutex<std::process::Child>>>,
    /// The user asked for this build to stop. Kept so the outcome is reported
    /// as their decision rather than as a failure with a signal in it.
    stopped: bool,
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
    /// Set when this workspace's preview MCP server is listening: the port
    /// whose instrumented proxy the preview iframe should load instead of the
    /// dev server directly, so the dispatched agent can see and drive the
    /// pane (AGE-143). `None` for the project checkout and for runs without
    /// the server; those load the dev server URL as always.
    pub preview_port: Option<u16>,
    /// `[preview] agent_tools` for this project, so the script editor can say
    /// what marking a script "web" grants a dispatched agent — at the moment
    /// the user is making that choice, and only when it is true.
    pub preview_tools_enabled: bool,
}

/// One script's live session state, as `run_scripts_status` reports it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunScriptStatusDto {
    pub name: String,
    pub status: SessionStatus,
}

/// Where a run's visible preview iframe sits in the app window, in CSS pixels
/// relative to the webview. Reported by the Run tab while the pane is on
/// screen; what the native preview screenshot crops to.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One run whose preview MCP server is up, for the frontend. `url` is the
/// instrumented preview the iframes load; `active` says whether anything is
/// serving on the run's app port, which is when a hidden preview host is
/// worth mounting at all (see the UI's PreviewKeeper).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTargetDto {
    pub run_id: String,
    pub url: String,
    pub active: bool,
}

/// Takes a run id, returns PNG bytes of that run's preview pane as shown in
/// the app window, or a reason there is no such picture right now. Installed
/// by the app shell (`crate::preview_shot`); absent in tests.
pub type PreviewShotFn =
    std::sync::Arc<dyn Fn(&str) -> std::result::Result<Vec<u8>, String> + Send + Sync>;

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
    /// What the user has said about this run — settled, active, snoozed,
    /// pinned — and what it means right now (see `crate::activity`). Always
    /// present, unlike `activity`: the standing and the pin are the user's own
    /// record, not a sample the notifier may not have taken yet.
    pub attention: crate::activity::AttentionInfo,
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
    /// Where this workspace's browser GUI is served (see
    /// `agent_catalog::WebUi`), on 127.0.0.1. None when no session here serves
    /// one, while a loop is driving (the headless attempt opens no port), and
    /// for a GUI run whose block had no room — the server is then on the CLI's
    /// own default port, which Agency won't claim to know.
    pub gui_port: Option<u16>,
    /// Which session serves `gui_port`: the run's own id when its primary
    /// agent is the web-served one, or an extra tab's id. The UI keys the GUI
    /// pane off this, so a web agent opened as a tab gets its GUI too rather
    /// than a server nobody can reach.
    pub gui_session_id: Option<String>,
    /// Something is accepting connections on `gui_port` right now, so the GUI
    /// pane can load it instead of a connection error. Only ever true while
    /// the session that owns the port is running, and (for a web agent) after
    /// the workspace handshake has finished — their first-paint selection
    /// runs once, and an empty list never picks the folder we just launched in.
    pub gui_live: bool,
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
    /// Messages Agency is holding for this run's sessions and has not typed in
    /// yet (see `crate::sendq`). Zero for almost every run almost always; while
    /// it isn't, the tile and the run header say so, because a message waiting
    /// behind a long turn is otherwise indistinguishable from one that was
    /// never sent.
    pub queued_messages: u32,
    /// The run's own agent tab has been closed (AGE-184), so the tab strip
    /// stops drawing it and `status` above describes whichever extra tab is
    /// standing in for it. See `lead_session_name`.
    pub primary_closed: bool,
    /// Epoch seconds. Exposed for time views (the weekly note); archived_at is
    /// None for live runs and last-archive-wins after a restore cycle.
    pub created_at: i64,
    pub archived_at: Option<i64>,
    /// What this run's archive still holds. Only set by `list_archived_runs`:
    /// answering it costs a git call per run, and the live board polls every
    /// 1.5 seconds while the Archived list is read once, when it is opened.
    pub archived: Option<ArchivedInfo>,
}

/// What is left of an archived run — the two things the list has to be able to
/// say, because the difference between them is what "archived" now means.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedInfo {
    /// Its branch is still in the repo, so the run can be restored into a
    /// fresh worktree. False for the normal ending of merged work.
    pub branch_kept: bool,
    /// With `branch_kept` false, the branch a restore would cut the run's
    /// branch afresh from — the base its work went into. `None` only when that
    /// branch has gone too, which is the one archive nothing can be restored
    /// from; the list disables Restore on exactly that.
    pub restore_base: Option<String>,
    /// There is a record file to read.
    pub has_record: bool,
    /// There is a conversation to read: a transcript in a format we parse,
    /// either rescued into the archive or still in the agent's own store
    /// (runs archived before rescues existed). Runs archived before records
    /// existed can have this and no record, which is why it is asked
    /// separately.
    pub has_conversation: bool,
}

/// A run's conversation as the archive viewer renders it.
///
/// `supported` false means Agency does not know this agent's transcript
/// format at all, and the UI must say "we cannot see this", never "the agent
/// said nothing" — the same distinction usage keeps for tokens. With
/// `supported` true and no sessions, nothing was kept or nothing was said.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationInfo {
    pub supported: bool,
    pub sessions: Vec<agency_core::transcript::Conversation>,
}

/// One message the send queue is holding, as shown by the run's marker.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMessageInfo {
    /// The session it will be typed into: the run's own id, or `<run>--<n>`
    /// for one of its extra agent tabs.
    pub session_id: String,
    /// The feature that composed it (`crate::sendq::ORIGINS`).
    pub origin: String,
    /// The whole text, so the user can read what will be typed before deciding
    /// whether to drop it, and the handle the drop is matched on.
    pub text: String,
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

/// Where an agent started on a PR (a review, or a conflict resolution) landed.
/// `session_id` is set when it had to run as an extra tab inside an existing
/// run (the PR's branch was already checked out there); the UI focuses that tab
/// instead of the run's primary agent. None means it got a workspace of its own.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrAgentRun {
    pub run: RunInfo,
    pub session_id: Option<String>,
}

/// Why a PR can't be merged, in the terms the user needs to act on it: which
/// branch is stuck on which base, and the files a merge would collide in.
///
/// `files` empty with `probed` false means the conflict is real (GitHub says
/// so) but the local probe couldn't run — an old git, or a branch we can't
/// fetch. The UI says "conflicts" without pretending to know where.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrConflicts {
    pub base: String,
    pub head: String,
    pub files: Vec<String>,
    pub probed: bool,
}

/// One attempt in a race: an agent, and the model that attempt runs on.
///
/// The attempt is the unit rather than the agent, so the same agent can appear
/// twice on two models (AGE-118) — the comparison a model picker invites, and
/// the one racing could not express while it took a set of agent names. The
/// model rides on the attempt because model ids live in each CLI's own
/// namespace, so there is no one model to give a whole race.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RaceAttempt {
    pub agent: String,
    /// None = whatever that agent defaults to.
    #[serde(default)]
    pub model: Option<String>,
}

/// Everything a race has to satisfy before the first workspace is cut. Pure, so
/// the rules are unit-testable without an app, and checked up front because a
/// failure on the third attempt would leave the first two running a race they
/// can no longer win.
fn validate_race(prompt: &str, attempts: &[RaceAttempt]) -> Result<()> {
    if prompt.trim().is_empty() {
        bail!("racing needs a prompt — it is sent to every agent at launch");
    }
    // Two attempts, not two agents: one agent on two models is the race a model
    // picker invites, and it is indistinguishable here from two different
    // agents (AGE-118).
    if attempts.len() < 2 {
        bail!("racing needs at least two attempts");
    }
    for attempt in attempts {
        if attempt.model.is_some() && !crate::agent_catalog::supports_model(&attempt.agent) {
            bail!("'{}' has no way to be told a model on the command line", attempt.agent);
        }
    }
    Ok(())
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
    /// The run's name in the UI, where the dispatcher already has one (issue,
    /// PR, race). Also the text the run's id and branch are derived from: see
    /// [`id_source`].
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

/// What each teardown verb would remove from one run, and why. Both verbs come
/// back together because every surface that offers one offers the other, and
/// the difference between them is the thing the user is deciding.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCleanup {
    pub facts: BranchFacts,
    pub archive: agency_core::cleanup::CleanupPlan,
    pub delete: agency_core::cleanup::CleanupPlan,
    /// The branch the plans are about, and the base they were measured
    /// against — both named in the dialog, so both travel with the plan rather
    /// than being re-derived by the UI.
    pub branch: String,
    pub base: String,
    /// Agency knows where this agent keeps its conversation for this worktree,
    /// so archiving rescues it into the archive and deleting removes it along
    /// with the run.
    ///
    /// False in two common cases, and in both of them neither verb touches the
    /// agent's history at all: an agent whose transcript format Agency cannot
    /// read (`usage::format_for` is a default-deny list of two, so most agents
    /// land here), and a run in the project's own checkout, whose session
    /// directory holds the user's own conversations in that folder and is not
    /// Agency's to move. The teardown copy has to be able to tell those apart:
    /// "the transcript goes too" is a warning, and a warning that is wrong for
    /// most agents is one nobody reads on the day it is right.
    pub manages_transcript: bool,
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

/// The daemon session that speaks for a run: its status dot, the notifier's
/// busy/idle watch, and the default target for text Agency types in.
///
/// Normally the run's own session. Once the user has closed the run's first
/// agent tab (AGE-184) that session is gone for good, and the run is carried
/// by its extra tabs — so the lowest-numbered one still alive stands in for
/// it. Lowest-numbered rather than newest: it is the tab strip's leftmost, so
/// what the rail's dot describes is what the strip opens on.
///
/// `live` is the daemon's session listing; a session missing from it is gone,
/// which is why this can decide without a registry read. With the primary
/// closed and no tab alive, it falls back to the run's own name, whose status
/// then reads Gone — which is the truth about the run.
fn lead_session_name(run: &agency_core::registry::Run, live: &[(String, SessionStatus)]) -> String {
    if run.primary_closed_at.is_none() {
        return session_name(&run.id);
    }
    let prefix = format!("{}--", session_name(&run.id));
    live.iter()
        .filter_map(|(name, _)| {
            let seq = name.strip_prefix(&prefix)?.parse::<u32>().ok()?;
            Some((seq, name.clone()))
        })
        .min_by_key(|(seq, _)| *seq)
        .map(|(_, name)| name)
        .unwrap_or_else(|| session_name(&run.id))
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
///
/// `resume_args` is left alone: a resume launches `args` ahead of the resume
/// recipe (AGE-100), so pinning the model in both would hand the CLI `--model`
/// twice.
fn with_model(profile: &AgentProfile, model: Option<&str>) -> AgentProfile {
    let extra = crate::agent_catalog::model_args(&profile.name, model);
    if extra.is_empty() {
        return profile.clone();
    }
    let with_extra = |recipe: &[String]| -> Vec<String> {
        recipe.iter().cloned().chain(extra.iter().cloned()).collect()
    };
    AgentProfile {
        args: with_extra(&profile.args),
        loop_args: profile.loop_args.as_deref().map(with_extra),
        ..profile.clone()
    }
}

/// A copy of `profile` whose interactive launch boots the agent's web GUI on
/// the run's own port (see [`crate::agent_catalog::WebUi`]). A no-op for every
/// terminal agent and for custom profiles.
///
/// Prepended, not appended: dsh's launcher hands everything after its own
/// flags to the booted app, so the `web` selector has to come ahead of
/// whatever the user put in the profile's arguments, or a user argument would
/// be read as the app selection. Loop recipes are left alone — the headless
/// one-shot opens no port and must never boot a server.
///
/// For dsh, also inserts `web --patch <agency overlay>` so the conversation
/// sidebar starts collapsed (their layout store has no durable default; see
/// [`crate::web_ui::ensure_collapse_sidebar_patch`]). `--patch` is a flag on
/// the `web` subcommand itself, so it sits immediately after `web`, ahead of
/// `--no-open` / `--port`.
///
/// `gui_port` is None for a run that predates port blocks or whose block is
/// too small to hold a GUI port; the server then comes up on the CLI's own
/// default port. One such run works; a second collides there, loudly, in its
/// own pane — which beats refusing to launch over a ports-config edge.
fn with_web_ui(
    profile: &AgentProfile,
    gui_port: Option<u16>,
    data_dir: &std::path::Path,
) -> AgentProfile {
    let Some(web) = crate::agent_catalog::web_ui(&profile.name) else {
        return profile.clone();
    };
    let mut args: Vec<String> = web.args.iter().map(|a| a.to_string()).collect();
    // args[0] is the app selector (`web`); launcher overlays belong right after.
    if profile.name == "dsh" {
        match crate::web_ui::ensure_collapse_sidebar_patch(data_dir) {
            Ok(patch) => {
                args.insert(1, "--patch".into());
                args.insert(2, patch.to_string_lossy().into_owned());
            }
            Err(e) => {
                log::warn!("dsh collapse-sidebar patch unavailable; sidebar stays open: {e:#}")
            }
        }
    }
    if let Some(port) = gui_port {
        args.extend(web.port_args.iter().map(|a| a.replace("{{port}}", &port.to_string())));
    }
    args.extend(profile.args.iter().cloned());
    AgentProfile { args, ..profile.clone() }
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

/// The profile's own arguments as they survive a resume: everything the user
/// put in Settings -> agent -> Arguments, minus the prompt.
///
/// AGE-100: a resume used to launch the resume recipe *instead of* the
/// profile's args, so a profile carrying `--permission-mode acceptEdits` or
/// `--add-dir` ran with those flags on the first launch and silently without
/// them from the first resume onwards. Only the prompt is meant to go, and
/// `{{prompt}}` is already the token that marks it. A prompt written as the
/// value of a flag (copilot's `-i`, opencode's `--prompt`) takes that flag with
/// it: left behind, it would swallow the resume verb that follows. Only the
/// flag the agent's own catalog recipe names is dropped that way, so a boolean
/// flag the user happened to write ahead of a positional `{{prompt}}` stays.
fn args_without_prompt(profile: &AgentProfile) -> Vec<String> {
    let prompt_flag = match crate::agent_catalog::prompt_delivery(&profile.name) {
        crate::agent_catalog::PromptDelivery::Args(recipe) => recipe
            .iter()
            .position(|a| a.contains("{{prompt}}"))
            .filter(|&i| i > 0)
            .map(|i| recipe[i - 1]),
        _ => None,
    };
    let mut out: Vec<String> = Vec::with_capacity(profile.args.len());
    for arg in profile.args.iter().filter(|a| !a.is_empty()) {
        if !arg.contains("{{prompt}}") {
            out.push(arg.clone());
            continue;
        }
        if let Some(flag) = prompt_flag {
            if out.last().is_some_and(|a| a == flag) {
                out.pop();
            }
        }
    }
    out
}

/// Decide the (command, args) to launch for an agent run. With `use_resume` and
/// a resume recipe present, launch the profile's own args minus the prompt (see
/// [`args_without_prompt`]) followed by the resume recipe. Otherwise hand the
/// whole decision to [`fresh_agent_argv`], the one fresh recipe. The optional
/// setup script wraps the command in both cases (same as create_run/rerun).
///
/// AGE-137: the fresh branch used to render the profile's args and stop there,
/// which was the same thing only while profiles carried a `{{prompt}}` token.
/// AGE-79 moved prompt delivery out of the args, so catalogue profiles ship
/// with `args: []` and both of this function's fresh callers — `rerun_locked`
/// and the resume fallback in `ensure_run_active` — launched `claude` in the
/// worktree with the task text nowhere. One fresh recipe, not two.
///
/// `conversation` is the conversation this session owns, for an agent that
/// names them (AGE-177). It replaces the profile's recipe rather than joining
/// it: `claude --continue --resume <id>` is two answers to one question, and
/// the generic half is the one that crossed two runs sharing a directory.
fn agent_argv(
    profile: &AgentProfile,
    worktree: &Path,
    prompt: &str,
    use_resume: bool,
    setup: Option<&str>,
    conversation: Option<&str>,
) -> (String, Vec<String>) {
    let Some(generic) = profile.resume_args.as_ref().filter(|_| use_resume) else {
        return fresh_agent_argv(profile, worktree, prompt, setup, conversation);
    };
    // The profile's flags first, the resume recipe last: `codex resume --last`
    // is a subcommand, and a flag written after it would be read as the
    // subcommand's rather than the CLI's.
    let mut base_args = args_without_prompt(profile);
    let exact = conversation
        .map(|c| agency_core::sessionstore::resume_args(&profile.command, c, &base_args))
        .filter(|args| !args.is_empty());
    base_args.extend(exact.unwrap_or_else(|| generic.to_vec()));
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
        crate::agent_catalog::PromptDelivery::AfterGuiReady => {
            // Delivered by `web_ui::handshake` once the server answers, not argv.
            Vec::new()
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
/// `{{prompt}}` token. An empty prompt yields the plain promptless argv. This is
/// the only fresh recipe: [`agent_argv`] delegates its non-resume branch here.
///
/// `conversation` is a freshly minted id for an agent that names conversations
/// (AGE-177), so this launch opens one only this session knows the name of.
/// Every fresh launch mints its own: a rerun is a new conversation, and claude
/// refuses `--session-id` for an id already in use.
fn fresh_agent_argv(
    profile: &AgentProfile,
    worktree: &Path,
    prompt: &str,
    setup: Option<&str>,
    conversation: Option<&str>,
) -> (String, Vec<String>) {
    // Rendered alongside the raw args so the `{{prompt}}` token's place is
    // still known after substitution: Agency's own flags go in front of it, and
    // a flag behind a positional argument is the shape most CLIs are least
    // happy with. A profile that places the prompt itself used to get them
    // behind it, which was invisible while the only such flags were MCP's
    // `--additional-mcp-config` (copilot reads it either way) and became a hard
    // error the moment `--session-id` joined them.
    let mut args: Vec<String> = Vec::with_capacity(profile.args.len());
    let mut prompt_at = None;
    for (raw, rendered) in profile.args.iter().zip(profile.render_args(prompt)) {
        if rendered.is_empty() {
            continue;
        }
        if prompt_at.is_none() && raw.contains("{{prompt}}") {
            prompt_at = Some(args.len());
        }
        args.push(rendered);
    }
    let mut ours = conversation
        .map(|c| agency_core::sessionstore::open_args(&profile.command, c, &args))
        .unwrap_or_default();
    ours.extend(mcp_launch_args(profile, worktree, &args));
    match prompt_at {
        Some(i) => {
            args.splice(i..i, ours);
        }
        None => args.extend(ours),
    }
    if !prompt.trim().is_empty() && prompt_at.is_none() {
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

/// Where a run on a PR's head branch can work. git allows a branch to be checked
/// out in one worktree at a time, so the workspace a PR run gets is decided by
/// who already holds the branch, not by preference.
#[derive(Clone, PartialEq, Eq, Debug)]
enum PrWorkspace {
    /// Nothing holds the branch: the run gets a worktree of its own on it.
    Worktree,
    /// The project's own checkout is standing on the branch, so the run works
    /// there. It is the only tree a fix can be committed to that branch in and
    /// pushed from, which is the whole point of the review staying open.
    Checkout,
    /// Some other worktree holds the branch. An agent run of ours living there
    /// can host the review as an extra tab; anything else has to be refused,
    /// and by path, since only the user can free the branch.
    Held(PathBuf),
}

impl PrWorkspace {
    /// The tree the branch is already in, if it is in one. `None` means the
    /// branch is free and the run gets a workspace cut for it.
    fn holder(&self, repo: &Path) -> Option<PathBuf> {
        match self {
            PrWorkspace::Worktree => None,
            PrWorkspace::Checkout => Some(repo.to_path_buf()),
            PrWorkspace::Held(path) => Some(path.clone()),
        }
    }
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
    workspace: PrWorkspace,
    post_comments: bool,
) -> String {
    let mut p = format!(
        "Review GitHub pull request #{number}: {title}\n\n\
         The PR's head branch is checked out in this workspace. Read the change with \
         `gh pr diff {number}`, or `git diff {base}...HEAD`, and review it for correctness, \
         bugs, security problems, missing tests, and anything else that should block the \
         merge. Read the surrounding code too, not just the diff.\n\n"
    );
    if workspace == PrWorkspace::Checkout {
        // The branch was already checked out in the project's own tree, so this
        // run has no worktree of its own; the agent has to know it is standing
        // in the user's working copy and not in a scratch workspace.
        p.push_str(&format!(
            "This workspace is the project's own checkout, not an isolated worktree: git \
             allows a branch to be checked out in one place at a time, and the PR's branch \
             was already here. Treat the working tree as someone else's. It may hold \
             uncommitted changes that have nothing to do with the PR, and it can sit behind \
             the PR's head if commits were pushed to the branch elsewhere, so take the \
             change from `gh pr diff {number}` rather than from the working tree. Do not \
             switch branches, stash, reset, or revert anything you did not write.\n\n"
        ));
    }
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

/// The prompt for an agent sent to clear a PR's merge conflicts.
///
/// The conflicting paths are listed rather than left to be discovered: the
/// probe has already run to draw the UI, so the agent may as well start from
/// the answer. The direction is spelled out too — merging the base into the
/// head branch is what updates the PR, and a rebase or a force-push would
/// rewrite a branch someone else may already have pulled. `workspace` says
/// whether the merge is about to happen in a worktree of the run's own or in
/// the user's checkout, which the agent has to know before it touches anything.
fn pr_conflict_prompt(
    number: u64,
    title: &str,
    url: &str,
    base: &str,
    head: &str,
    workspace: PrWorkspace,
    files: &[String],
) -> String {
    let mut p = format!(
        "Resolve the merge conflicts blocking GitHub pull request #{number}: {title}\n\n\
         The PR's head branch `{head}` is checked out in this workspace, and it conflicts \
         with its base branch `{base}`, so GitHub refuses to merge it.\n\n"
    );
    let list = files.join(", ");
    match files.len() {
        0 => p.push_str("Run the merge to find out which files collide.\n\n"),
        1 => p.push_str(&format!("One file conflicts: {list}.\n\n")),
        n => p.push_str(&format!("These {n} files conflict: {list}.\n\n")),
    }
    if workspace == PrWorkspace::Checkout {
        // The PR's branch was already checked out in the project's own tree, so
        // this run has no worktree of its own and the merge lands in the user's
        // working copy. Uncommitted work there is not this agent's to move: git
        // refuses a merge over it, and the way out is to say so, not to stash.
        p.push_str(
            "This workspace is the project's own checkout, not an isolated worktree: git \
             allows a branch to be checked out in one place at a time, and the PR's branch \
             was already here. Treat the working tree as someone else's. If it holds \
             uncommitted changes that are not yours, stop and tell me rather than stashing, \
             resetting or committing them to get the merge going.\n\n",
        );
    }
    p.push_str(&format!(
        "Do this here, in this workspace:\n\
         1. `git fetch origin {base}`\n\
         2. `git merge origin/{base}`\n\
         3. Resolve every conflict, keeping both sides' intent. Read enough of each file to \
            know what the other change was for; a conflict is two people's work, not one \
            person's to delete.\n\
         4. Run the project's build or tests if it has quick ones.\n\
         5. Commit the merge and `git push` to update the PR.\n\n\
         Do not rebase and do not force-push: this branch is published. Do not merge the PR \
         itself, that is mine to do. Tell me what you had to decide.\n\n\
         PR link: {url}\n"
    ));
    p
}

/// The (command, args) for one headless loop attempt: the profile's loop
/// recipe with `{{prompt}}` filled in, wrapped by the optional setup script.
/// Errors when the profile has no loop recipe — such agents can't loop.
///
/// `conversation` names the attempt's own conversation for an agent that takes
/// one. Attempts are fresh sessions by design, so each gets a new name; the
/// last one's is what an interactive resume finds when the loop ends.
fn loop_argv(
    profile: &AgentProfile,
    worktree: &Path,
    prompt: &str,
    setup: Option<&str>,
    conversation: Option<&str>,
) -> Result<(String, Vec<String>)> {
    let recipe = profile.loop_args.as_ref().filter(|r| !r.is_empty()).ok_or_else(|| {
        anyhow!("agent '{}' has no loop recipe — set the profile's loop args first", profile.name)
    })?;
    let mut args: Vec<String> = recipe.iter().map(|a| a.replace("{{prompt}}", prompt)).collect();
    // Ahead of wherever the prompt lands, for the same reason as the
    // interactive path: keep Agency's flags out from behind a positional.
    let mut ours = conversation
        .map(|c| agency_core::sessionstore::open_args(&profile.command, c, &args))
        .unwrap_or_default();
    ours.extend(mcp_launch_args(profile, worktree, &args));
    match recipe.iter().position(|a| a.contains("{{prompt}}")) {
        Some(i) => {
            args.splice(i..i, ours);
        }
        None => args.extend(ours),
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

/// Whether this run can offer a browser-GUI pane at all (see
/// `agent_catalog::WebUi`): an agent run that a loop is not driving.
///
/// A loop attempt is `spawn_loop_attempt`'s headless one-shot, which by its own
/// contract opens no listening port — so a GUI pane on a looping run would sit
/// on "starting the GUI" for the life of the loop, waiting for a server that
/// was never launched. A finished loop is interactive again, and gets one back.
fn wants_web_ui(run: &agency_core::registry::Run) -> bool {
    run.kind == "agent" && !has_active_loop(run)
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

/// The text a run's id, and so its `agent/<id>` branch, is derived from: the
/// title where the dispatcher supplied one, else the prompt.
///
/// A merge writes the branch name into the base branch's history for good, so
/// what goes into it matters. An issue- or PR-dispatched prompt is the title
/// *plus the whole body*, and slugifying that takes the first 40 characters of
/// both: on AGE-139, whose body led with a link, the branch came out carrying
/// `-https-github-com-…` from that link. The title alone is the part written
/// to be read. Promptless and free-text runs have no title yet and keep
/// deriving from the prompt.
fn id_source<'a>(title: Option<&'a str>, prompt: &'a str) -> &'a str {
    match title {
        Some(t) if !t.trim().is_empty() => t,
        _ => prompt,
    }
}

/// Build a readable, unique task id from the prompt: a slug derived from the
/// prompt text plus a short suffix that disambiguates runs sharing a prompt.
/// The id doubles as the worktree dir, branch (`agent/<id>`) and daemon session
/// name, so it stays restricted to `[a-z0-9-]`, which is safe for all three.
pub fn new_task_id(prompt: &str) -> String {
    format!("{}-{}", slugify(prompt), short_suffix())
}

/// Which workspace a run on `branch` can have, from git's answer to who is
/// holding the branch rather than from the registry's record of it, which goes
/// stale the moment a checkout moves off the branch.
///
/// Observed (AGE-169): a PR opened from the user's own checkout, still standing
/// on that branch, made "Review with an agent" fail with git's raw
/// "fatal: 'features/accessibility-pass' is already used by worktree". The
/// checkout is a fine place to review from, and the only tree a fix could be
/// committed to that branch in, so it gets its own variant here instead of
/// being lumped in with the trees Agency cannot use.
fn pr_workspace(repo: &Path, branch: &str) -> PrWorkspace {
    match agency_core::git::branch_worktree(repo, branch) {
        None => PrWorkspace::Worktree,
        Some(holder) if same_dir(&holder, repo) => PrWorkspace::Checkout,
        Some(holder) => PrWorkspace::Held(holder),
    }
}

/// The refusal for a branch held by a tree Agency cannot put an agent in. git's
/// own "already used by worktree" says the same thing, but says it as a fatal
/// from a command the user never ran.
fn branch_held_elsewhere(branch: &str, holder: &Path) -> anyhow::Error {
    anyhow!(
        "'{branch}' is checked out in {}, and git allows a branch in only one workspace at a \
         time. Point that workspace at another branch, then start the review again.",
        holder.display()
    )
}

/// Whether two paths name the same directory. Compared through
/// `canonicalize` because git prints resolved paths in `worktree list` while a
/// project's root is whatever path it was registered under, and on macOS those
/// differ for anything under `/tmp` or a symlinked home. Falls back to a literal
/// comparison when either path cannot be resolved.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The environment that keeps this session's conversation to itself, for the
/// agents Agency can pin (see [`agency_core::sessionstore`]). Empty for
/// everyone else, so every agent launch can extend with it unconditionally.
///
/// AGE-175: without it, "resume the most recent conversation in this
/// directory" is whatever session was touched last, and every run sharing a
/// directory — a `worktree: false` run, a gitless project, an extra agent tab
/// — comes back into the same one. Applied on fresh launches as much as on
/// resumes: the store a session resumes from is the one its first launch
/// wrote to.
fn session_store_env(command: &str, worktree: &Path, session: &str) -> Vec<(String, String)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    agency_core::sessionstore::env(&home, command, worktree, session)
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

/// Where a restore cuts this run's branch afresh when the branch itself is
/// gone: the branch its work was merged into, if that is still in the repo.
///
/// The merge target first, since that is where the run's commits actually went;
/// `run.base` second, for a run that never named one. `None` when neither is in
/// the repo any more — the only archive left that cannot be restored, and the
/// only one whose Restore button is disabled.
fn restore_start_point(repo: &Path, run: &agency_core::registry::Run) -> Option<String> {
    agency_core::merge::resolve_target(run.merge_target.as_deref(), repo)
        .ok()
        .into_iter()
        .chain(std::iter::once(run.base.clone()))
        .find(|b| agency_core::merge::branch_exists(repo, b))
}

/// The session files in a transcript directory. What the record calls "2
/// session files": the directory's own, plus those in the per-session stores
/// one level down (AGE-175). Deeper subdirectories (claude's tool results,
/// subagent transcripts) ride along in the rescue but are not sessions, which
/// is where `usage::transcripts` stops.
fn count_sessions(dir: &Path) -> usize {
    agency_core::usage::transcripts(dir).len()
}

/// The agent's own command for picking the newest rescued session up again,
/// for the agents whose resume-by-id verb is actually known. Claude's session
/// id is its file's stem and `--resume <id>` its flag; no other agent's has
/// been verified, and a guessed command in a record is worse than none.
///
/// Newest by mtime, which the rescue preserved; mtime is also what the
/// agent's own "resume most recent" goes by.
fn resume_command(command: &str, dir: &Path) -> Option<String> {
    let base = Path::new(command).file_name().and_then(|s| s.to_str()).unwrap_or(command);
    if base != "claude" {
        return None;
    }
    let mut newest: Option<(std::time::SystemTime, String)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
        if newest.as_ref().map_or(true, |(t, _)| modified > *t) {
            newest = Some((modified, stem.to_string()));
        }
    }
    newest.map(|(_, id)| format!("claude --resume {id}"))
}

/// Delete a file, treating "it was not there" as success. Both callers are
/// removing a record that may predate records existing at all.
fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
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

/// The four-character disambiguator `new_task_id` appends. Used when a
/// first-prompt rename rebuilds the branch leaf so the worktree dir and
/// daemon session (which stay on the original id) keep matching the suffix.
fn id_suffix(run_id: &str) -> Option<&str> {
    let (_, suf) = run_id.rsplit_once('-')?;
    if suf.len() == 4 && suf.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(suf)
    } else {
        None
    }
}

/// Whether `branch` is still the *empty-prompt* auto-cut name for `run_id`,
/// i.e. `agent/agent-<suffix>`. Both halves are load-bearing. A manual rename
/// (AGE-148) or an earlier first-prompt rename (AGE-183) moves the branch off
/// that shape, and neither should be overwritten by a later pass.
///
/// The id check is what keeps a prompt-derived name safe. `create_run_spec`
/// titles any run created with a real prompt, so the title guard in
/// `apply_first_prompt` normally short-circuits before this is reached — but
/// `rename_run` stores a trimmed title, and an empty one is the documented way
/// to clear it. Observed on a probe of that path: clearing the title on a
/// weekly-narration run and typing one line renamed
/// `agent/narrate-the-weekly-review-note-docs-week-q3w7` to
/// `agent/continue-please-q3w7`. Matching only the `agent-<suffix>` id shape
/// means there is no good name to lose.
fn is_auto_cut_branch(run_id: &str, branch: &str) -> bool {
    let Some(suffix) = id_suffix(run_id) else { return false };
    run_id == format!("agent-{suffix}") && branch == format!("agent/{run_id}")
}

/// The branch leaf a first-prompt rename would apply: `<slug>-<suffix>`,
/// reusing the suffix already on the run id. `None` when the prompt still
/// slugifies to the empty-prompt fallback (`agent`), which would leave the
/// branch as `agent/agent-<suffix>` and only churn git for nothing.
///
/// AGE-183: promptless starts cut `agent/agent-<suffix>` before anyone types;
/// the first prompt titles the run but used to leave the branch stuck there.
fn branch_leaf_from_first_prompt(run_id: &str, prompt: &str) -> Option<String> {
    let suffix = id_suffix(run_id)?;
    let slug = slugify(prompt);
    if slug == "agent" {
        return None;
    }
    Some(format!("{slug}-{suffix}"))
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

/// The port a run's preview MCP server binds — the last of the run's port
/// block — or `None` when the run gets no preview server: switched off in
/// `[preview]`, no web run script to preview, no port block at all, or a
/// block too small to hold a second port beside the app's (AGE-143).
fn preview_mcp_port_for(
    config: &agency_core::config::AgencyConfig,
    port_base: Option<u16>,
) -> Option<u16> {
    if !config.preview.agent_tools {
        return None;
    }
    if !config.scripts.run_list().iter().any(|s| s.web) {
        return None;
    }
    agency_core::preview::mcp_port(port_base?, config.ports.block_size)
}

/// The port a web-GUI agent's server is told to bind: the second-to-last of
/// the run's port block. The block's first port is the workspace's own
/// `$AGENCY_PORT` (the dev server the agent may start) and its last is the
/// preview MCP server, so the GUI has to sit elsewhere or the agent's own app
/// would fight it. `None` when the agent serves no GUI, the run has no port
/// block, or the block is too small to hold a third port — the launch then
/// falls back to the CLI's default port (see `with_web_ui`).
fn gui_port_for(
    config: &agency_core::config::AgencyConfig,
    port_base: Option<u16>,
    agent: &str,
) -> Option<u16> {
    crate::agent_catalog::web_ui(agent)?;
    let base = port_base?;
    if config.ports.block_size < 3 {
        return None;
    }
    base.checked_add(config.ports.block_size - 2)
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

/// The run the most recent notification was about, waiting for the user to ask
/// for it. Two things may ask, and `notifier::opens_notified_run` decides which
/// one this entry answers to: a click on the notification (macOS routes that
/// through the delegate hook in `notif_macos`), or Agency becoming the focused
/// app again — but only for a notification posted while it was in the
/// background, because that return is the user coming back to the banner.
struct PendingOpen {
    project_id: String,
    run_id: String,
    /// Whether Agency was in the background when the notification went out —
    /// the OS's answer, not the webview's (see `foreground`).
    from_background: bool,
    at: std::time::Instant,
}

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

/// Hand over the pending notification target if `trigger` is one it answers to,
/// and clear it. Anything else leaves it in place for the trigger that does —
/// a banner the user has not clicked yet outlives a focus change.
fn take_pending_open(
    ui: &mut UiState,
    trigger: crate::notifier::OpenTrigger,
) -> Option<(String, String)> {
    let opens = ui.pending_open.as_ref().is_some_and(|p| {
        crate::notifier::opens_notified_run(trigger, p.from_background, p.at.elapsed())
    });
    if !opens {
        return None;
    }
    let p = ui.pending_open.take()?;
    Some((p.project_id, p.run_id))
}

/// One asking of an agent's listing command: the agent, and the directory the
/// command was run in. Both halves are the key because a project-scoped CLI
/// gives a different answer in each place (see `model_probes`).
type ProbeKey = (String, Option<PathBuf>);

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
    /// Web-GUI sessions whose post-boot handshake has finished (workspace
    /// adopted, or the budget expired). `gui_live` stays false until the id is
    /// here, so the iframe does not load an empty folder picker.
    web_ui_ready: crate::web_ui::ReadySet,
    /// Per-session keyboard bookkeeping for the send queue: when the human last
    /// touched this pane, and whether what they typed is still sitting unsent on
    /// the prompt line. Written by `run_input`, read by `drain_session`.
    /// In-memory: after a restart no draft is known, and the queue is empty then
    /// anyway.
    human_input: Mutex<HashMap<String, crate::sendq::HumanInput>>,
    /// Text Agency owes each session — review comments, failing checks, a merge
    /// conflict — waiting for a moment when typing it won't land mid-turn or on
    /// top of a half-typed prompt. Drained on the notifier tick; see
    /// `crate::sendq` for the rules.
    send_queue: Mutex<HashMap<String, crate::sendq::SendQueue>>,
    /// Queue events waiting to be told to the user, for the ones a command
    /// thread's own drain produced rather than the notifier tick's. The tick
    /// hands everything to the UI, so this is only the crossing between the
    /// two; it is never read anywhere else.
    queue_notices: Mutex<Vec<crate::sendq::Notice>>,
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
    /// Cancel tokens for in-flight repo setup, keyed by folder: the commit's
    /// repo path, or the folder a clone is downloading into. The dialogs' Cancel
    /// flips one so the git behind it stops: staging a folder of model weights,
    /// or cloning a huge repository, can run for many minutes, and until this
    /// existed the only way out was to quit the app.
    setup_cancels: Mutex<HashMap<PathBuf, agency_core::setup::CancelToken>>,
    /// Cancel tokens for in-flight pushes, keyed by the worktree being pushed.
    /// Source control's Cancel flips one so the `git push` behind it stops: a
    /// branch with large objects on a slow uplink uploads for minutes, and the
    /// progress bar was unstoppable short of quitting the app. Separate from
    /// `setup_cancels` because the two are keyed by different things (a folder
    /// being set up, a worktree being pushed) and a project's own checkout can
    /// be both.
    push_cancels: Mutex<HashMap<PathBuf, agency_core::setup::CancelToken>>,
    /// What each agent's own CLI said it can run, from the last time a model
    /// picker asked it (see `crate::model_probe`). Cached for the app session
    /// because the asking is a ~1s network round trip inside an open menu, and
    /// a CLI's catalogue does not change while the app is running. Failures are
    /// deliberately not cached: the usual reason one fails is that the agent is
    /// not logged in yet, and that is fixed from another window mid-session.
    ///
    /// Keyed by agent *and by the directory it was asked from*, because for a
    /// project-scoped CLI those are two different answers: one cache entry per
    /// agent listed the home directory's providers in every project, and
    /// opencode's project providers were nowhere (AGE-135). A user-scoped
    /// agent is always asked from home, so it still has the one entry.
    model_probes: Mutex<HashMap<ProbeKey, Vec<String>>>,
    /// Per-run preview MCP servers (AGE-143), keyed by run id. Started when a
    /// run is dispatched and re-converged by `sync_preview_servers` on the
    /// notifier tick, so archive/discard/restore/config edits all take effect
    /// within a tick without every one of those paths owning teardown.
    preview: Mutex<HashMap<String, agency_core::preview::PreviewServer>>,
    /// Runs whose preview server failed to bind (something else sat on the
    /// port), and when — retried on a slow cadence so the 2s sweep neither
    /// hammers the port nor spams the log.
    preview_failures: Mutex<HashMap<String, Instant>>,
    /// See [`PreviewRect`]. Absent entry = that run's pane is not on screen.
    preview_rects: Mutex<HashMap<String, PreviewRect>>,
    /// Native screenshot provider, installed by the app shell once a window
    /// exists (`crate::preview_shot`). An `Arc<OnceLock>` so server hooks
    /// capture the cell and read it at call time — preview servers start
    /// before the window is up, and tests never install one at all.
    preview_shot: std::sync::Arc<std::sync::OnceLock<PreviewShotFn>>,
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
    /// Why a stalled loop stalled, so the notification can say which cap to
    /// raise or what to fix. None for completions and for driver-side stalls
    /// (attempt spawn failure).
    pub reason: Option<agency_core::loops::StallReason>,
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
            web_ui_ready: Arc::new(Mutex::new(HashSet::new())),
            human_input: Mutex::new(HashMap::new()),
            send_queue: Mutex::new(HashMap::new()),
            queue_notices: Mutex::new(Vec::new()),
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
            push_cancels: Mutex::new(HashMap::new()),
            model_probes: Mutex::new(HashMap::new()),
            preview: Mutex::new(HashMap::new()),
            preview_failures: Mutex::new(HashMap::new()),
            preview_rects: Mutex::new(HashMap::new()),
            preview_shot: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        // Rehydrate: any run the daemon still hosts is adopted as-is; the watch
        // loop (watch_snapshot) then reports live status. Nothing to spawn here —
        // surviving sessions are already running in the daemon.
        if let Ok(sessions) = state.term.read().unwrap().list() {
            log::info!("termd: adopted {} surviving session(s)", sessions.len());
        }
        // Those surviving sessions are exactly why the send queue is stored:
        // a message held at the last quit is still worth typing into the agent
        // that is still sitting there waiting for it.
        state.restore_send_queues(crate::activity::now_ms());
        Ok(state)
    }

    fn provider_env(&self) -> Result<Vec<(String, String)>> {
        // Point OpenAI-protocol agents at the configured local model (LM Studio
        // by default). Agents with their own CLI auth (claude, codex, …) ignore
        // these. No cloud keys are injected — each agent uses its own login.
        let s = self.get_settings()?;
        let mut env = vec![
            ("OPENAI_BASE_URL".into(), s.lm_studio_base_url),
            ("OPENAI_API_KEY".into(), "lm-studio".into()),
        ];
        // Finder-launched bundles inherit no user secrets. dsh's first-run
        // modal asks for DEEPSEEK_API_KEY on every launch until the process
        // environment (or $DSH_HOME) already has one; passing the login-shell
        // value is the same repair PATH already does, not Agency storing a key.
        if let Some(key) = crate::pathenv::harvested("DEEPSEEK_API_KEY") {
            env.push(("DEEPSEEK_API_KEY".into(), key));
        }
        Ok(env)
    }

    /// After a web-served agent binds, adopt this worktree as its GUI workspace
    /// and (when `prompt` is non-empty) queue the opening ask. No-op for
    /// terminal agents. `prompt` is empty on a resume so we do not re-send an
    /// issue the existing dsh session already has.
    fn kick_web_ui(
        &self,
        session_id: &str,
        agent: &str,
        port: Option<u16>,
        cwd: &Path,
        prompt: &str,
    ) {
        if crate::agent_catalog::web_ui(agent).is_none() {
            return;
        }
        crate::web_ui::kick(&self.web_ui_ready, session_id, port, cwd, prompt);
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
                list_command: crate::agent_catalog::list_command(&profile.name),
            });
        }
        Ok(out)
    }

    /// Set the model `agent`'s next run starts on, exactly as finishing a
    /// launch does. `None` is the agent's own default, which is a choice of its
    /// own and not the absence of one.
    ///
    /// Settings writes the same per-agent setting the picker reads rather than
    /// a second "default model" beside it: there is one model the next run
    /// starts on, and two stores for it would need a rule for which of them
    /// the menu reopens on, with the loser silently ignored.
    pub fn set_agent_model(&self, agent: &str, model: Option<&str>) -> Result<()> {
        // The UI sends "the agent's default" as null, but an empty string is
        // the same statement, and stored as a model it would put a blank entry
        // at the head of the recents.
        let model = model.map(str::trim).filter(|m| !m.is_empty());
        let reg = self.registry.lock().unwrap();
        if reg.get_profile(agent)?.is_none() {
            bail!("no profile for {agent}");
        }
        remember_model(&reg, agent, model)
    }

    /// Ask `agent`'s own CLI which models it has, cached for the app session.
    ///
    /// The counterpart to `list_agent_models`, which is free and static: this
    /// one runs the agent's binary and waits on the network, so it is reached
    /// only from a picker that is already open, and only for the agents whose
    /// CLI has a listing command at all (`agent_catalog::model_listing`).
    /// Everything it returns is a validated model id, and the picker's typed
    /// field stays regardless — a probe can fail, and a CLI can run a model it
    /// does not list.
    ///
    /// `project_id` is the project whose picker is open. It is what makes the
    /// answer true for a CLI that reads config from the directory it runs in
    /// (`ListScope::Project`); everything else is asked from home either way.
    /// `None` where the caller genuinely has no project, which falls back to
    /// home and to the user-level answer.
    pub fn probe_agent_models(&self, agent: &str, project_id: Option<&str>) -> Result<Vec<String>> {
        let listing = crate::agent_catalog::model_listing(agent)
            .ok_or_else(|| anyhow!("{agent} has no command for listing its models"))?;
        // Where to ask from. Home is the neutral place: a CLI finds its own
        // user-level configuration there and no project's. A project-scoped CLI
        // is asked in the project instead, because that is the only place its
        // project providers exist (AGE-135) — the project's own checkout, not a
        // run's worktree, since every picker that reaches here is choosing a
        // model for a run that does not exist yet.
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let cwd = match (listing.scope, project_id) {
            (crate::model_probe::ListScope::Project, Some(id)) => Some(self.project_repo(id)?),
            _ => home,
        };
        let key = (agent.to_string(), cwd);
        if let Some(models) = self.model_probes.lock().unwrap().get(&key) {
            return Ok(models.clone());
        }
        // The profile's command, not the catalog's: a user who pointed this
        // agent at a different binary is asking *that* binary what it has, and
        // it is the one a run would launch.
        let command = self
            .registry
            .lock()
            .unwrap()
            .get_profile(agent)?
            .map(|p| p.command)
            .or_else(|| crate::agent_catalog::find(agent).map(|e| e.command.to_string()))
            .ok_or_else(|| anyhow!("no profile for {agent}"))?;
        let models = crate::model_probe::probe(&command, listing, key.1.as_deref())?;
        self.model_probes.lock().unwrap().insert(key, models.clone());
        Ok(models)
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
                serves_web_ui: entry.web_ui.is_some(),
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

    /// Fill the run's title (and stored prompt) from the first line typed into
    /// a promptless agent. When the branch is still the empty-prompt fallback
    /// (`agent/agent-<suffix>`), rename it to a slug of that prompt plus the
    /// same short suffix — the title already became memorable; the branch
    /// follows.
    ///
    /// AGE-183: observed on promptless starts where the agent chip read a real
    /// first prompt while the branch chip stayed on `agent/agent-<suffix>`.
    ///
    /// Refusals are logged, not returned: this runs on the keystroke that
    /// submits the first prompt, so a toast would interrupt the user about an
    /// action they never asked for, and returning would undo the title we just
    /// set. Every refusal `rename_run_branch` can raise is also unreachable
    /// this early — a run seconds old is not mid-merge, not published, and its
    /// `<slug>-<suffix>` leaf carries a suffix unique to it, so nothing can
    /// have taken the name. The log line is for the case that proves that
    /// wrong.
    ///
    /// The rename races the agent's first turn and is safe anyway, which is
    /// worth writing down because the shape invites the opposite conclusion:
    /// `FocusTerminal` sends the keystroke to the PTY before calling this, so
    /// the agent starts thinking while `rename_run_branch` moves the branch and
    /// rewrites the workspace skill under it. Neither half can be caught in the
    /// act. `skills::emit_for_agent` writes through `issuefs::atomic_write`, so
    /// a reader sees the whole old file or the whole new one; and the branch
    /// name sits in that skill's *body*, below the frontmatter, so it is read
    /// only when the skill is invoked — which costs a model round-trip, orders
    /// of magnitude longer than the local git calls here.
    pub fn apply_first_prompt(&self, id: &str, first_prompt: &str) -> Result<()> {
        let _ = self.store_run_prompt(id, first_prompt);
        if let Ok(Some(existing)) = self.run_title(id) {
            if !existing.is_empty() {
                return Ok(());
            }
        }
        let title = agency_core::title::fallback_title(first_prompt);
        if title.is_empty() {
            return Ok(());
        }
        self.store_run_title(id, &title)?;

        let Ok(run) = self.run_record(id) else {
            return Ok(());
        };
        if !run.worktree || !is_auto_cut_branch(&run.id, &run.branch) {
            return Ok(());
        }
        let Some(leaf) = branch_leaf_from_first_prompt(&run.id, first_prompt) else {
            return Ok(());
        };
        if let Err(e) = self.rename_run_branch(id, &leaf) {
            log::warn!("first-prompt branch rename for {id} to agent/{leaf} skipped: {e:#}");
        }
        Ok(())
    }

    // ── the user's word on a run (AGE-141) ─────────────────────────────────

    /// Record what the user has said about a run — settled, active, snoozed —
    /// or clear it with `None`, which hands the run back to the time decay.
    /// The clock is stamped here rather than sent from the UI: `at_ms` is what
    /// decides whether later output has un-settled the run, so it has to come
    /// from the same clock the notifier's observations do.
    pub fn set_run_standing(
        &self,
        id: &str,
        kind: Option<agency_core::attention::StandingKind>,
    ) -> Result<()> {
        let standing =
            kind.map(|k| agency_core::attention::Standing::new(k, crate::activity::now_ms()));
        self.registry.lock().unwrap().set_run_standing(id, standing.as_ref())
    }

    /// Pin a run to the end of its project's pinned runs, or unpin it. The
    /// order is the order they were pinned in, and it is the user's: unpinning
    /// and pinning again moves a run to the end. Ranks are fractional so a
    /// drag-to-reorder can land between two of them later without renumbering.
    pub fn pin_run(&self, id: &str, pinned: bool) -> Result<()> {
        let reg = self.registry.lock().unwrap();
        if !pinned {
            return reg.set_run_pin_rank(id, None);
        }
        let run = reg.get_run(id)?.ok_or_else(|| anyhow!("unknown run: {id}"))?;
        let rank = reg.max_pin_rank(&run.project_id)?.unwrap_or(0.0) + 1.0;
        reg.set_run_pin_rank(id, Some(rank))
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

    /// Clone `url` under `parent_dir`, registering a cancel token against the
    /// folder the clone will create so [`AppState::cancel_clone`] can stop it.
    pub fn clone_repo(
        &self,
        url: &str,
        parent_dir: &Path,
        on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<std::path::PathBuf> {
        let cancel = agency_core::setup::CancelToken::new();
        // A URL with no folder name in it fails before git runs, so there is
        // nothing to cancel and nothing to key the token by.
        let key = agency_core::setup::clone_destination(url, parent_dir);
        if let Some(key) = &key {
            self.setup_cancels.lock().unwrap().insert(key.clone(), cancel.clone());
        }
        let out =
            agency_core::setup::clone_repo_with_progress(url, parent_dir, &cancel, on_progress);
        if let Some(key) = &key {
            self.setup_cancels.lock().unwrap().remove(key);
        }
        out
    }

    /// Stop a clone of `url` into `parent_dir`, if one is running. Takes the
    /// clone's own two arguments rather than a path so the frontend never has to
    /// reproduce `clone_destination`'s naming rule, where a mismatch of one
    /// character would silently cancel nothing.
    pub fn cancel_clone(&self, url: &str, parent_dir: &Path) {
        if let Some(dest) = agency_core::setup::clone_destination(url, parent_dir) {
            self.cancel_repo_setup(&dest);
        }
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
    /// Also the landing point for [`AppState::cancel_clone`], which keys the same
    /// map by the clone's destination folder.
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

    /// The live read of one run's activity: its derived state (`None` until the
    /// notifier has observed it) and what the user's own word says about it.
    /// Shared by the board and the notifier tick, which are asking the same
    /// question — does this run want the user right now — and must not answer
    /// it differently.
    fn read_activity(
        &self,
        run: &agency_core::registry::Run,
        now_ms: i64,
    ) -> (Option<crate::activity::ActivityInfo>, crate::activity::AttentionInfo) {
        // Loops drive themselves — a quiet attempt isn't waiting on the user,
        // so it classifies as idle at most.
        let turn_driven =
            run.loop_config.is_none() && self.prompted.lock().unwrap().contains(&run.id);
        let entry = self.activity.lock().unwrap().get(&run.id).copied();
        let activity = entry
            .map(|e| crate::activity::classify(&e, turn_driven, run.standing.as_ref(), now_ms));
        let attention = crate::activity::attention(
            run.standing.as_ref(),
            run.pin_rank,
            entry.as_ref(),
            activity.map(|a| a.state),
            now_ms,
        );
        (activity, attention)
    }

    fn run_record(&self, id: &str) -> Result<agency_core::registry::Run> {
        let reg = self.registry.lock().unwrap();
        reg.get_run(id)?.ok_or_else(|| anyhow!("unknown run: {id}"))
    }

    /// The session in this workspace whose agent serves a browser GUI, as
    /// `(session id, agent)`. The primary session's id is the run's own; an
    /// extra tab's is `<run id>--<n>`. `start_run_session` admits at most one
    /// web-GUI session per workspace, so there is never a second to choose
    /// between.
    ///
    /// The extra-tab lookup is one small indexed read, and only for runs that
    /// could have a GUI at all — the same poll already runs a whole git
    /// process per run for the diff stat.
    fn web_ui_session(&self, run: &agency_core::registry::Run) -> Option<(String, String)> {
        if crate::agent_catalog::web_ui(&run.agent).is_some() {
            return Some((run.id.clone(), run.agent.clone()));
        }
        let sessions = self.registry.lock().unwrap().list_run_sessions(&run.id).ok()?;
        sessions
            .into_iter()
            .find(|s| crate::agent_catalog::web_ui(&s.agent).is_some())
            .map(|s| (s.id, s.agent))
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
        let name = lead_session_name(run, live);
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
        let gui_session = wants_web_ui(run).then(|| self.web_ui_session(run)).flatten();
        // Computed rather than stored so it can never disagree with what the
        // launch rendered — both come from the same `gui_port_for`. The config
        // load is two small file reads; the diff stat above already runs a
        // whole git process on the same poll.
        let gui_port = gui_session.as_ref().and_then(|(_, agent)| {
            let repo = self.project_repo(&run.project_id).ok()?;
            gui_port_for(&agency_core::config::load(&repo), run.port_base, agent)
        });
        // Probed only while the session that owns the port is running — which
        // is not always the run's primary one, since a web agent can be an
        // extra tab. A dead session's port is either free (instant refusal) or
        // someone else's server, and claiming the latter as this run's GUI
        // would render a stranger's page in its pane.
        //
        // The running check narrows that window but does not close it: if the
        // server lost the bind (something outside Agency already had the port),
        // the session is alive, the port answers, and the answer is the other
        // process's. Nothing cheap distinguishes them from here — the bind
        // error is in the session's own log, which is a tab away.
        let gui_live = gui_port.is_some_and(|port| {
            let name = gui_session.as_ref().map(|(id, _)| session_name(id)).unwrap_or_default();
            let running =
                live.iter().any(|(n, s)| *n == name && matches!(s, SessionStatus::Running));
            // The iframe must not win the race against workspace.create: their
            // startInitialSelection runs once and treats an empty list as done.
            let handshake_done = gui_session
                .as_ref()
                .is_none_or(|(id, _)| self.web_ui_ready.lock().unwrap().contains(id));
            running && agency_core::preview::serving(port) && handshake_done
        });
        let (activity, attention) = self.read_activity(run, crate::activity::now_ms());
        RunInfo {
            id: run.id.clone(),
            project_id: run.project_id.clone(),
            agent: run.agent.clone(),
            prompt: run.prompt.clone(),
            title: run.title.clone(),
            branch,
            status,
            activity,
            attention,
            usage: self.usage.lock().unwrap().get(&run.id).map(|(_, u)| u.into()),
            added: stat.added,
            deleted: stat.deleted,
            files: stat.files,
            port: run.port_base,
            gui_port,
            gui_session_id: gui_session.map(|(id, _)| id),
            gui_live,
            kind: run.kind.clone(),
            run_scripts_live: any_run_script_live(&run.id, live),
            queued_messages: self.queued_message_count(&run.id),
            primary_closed: run.primary_closed_at.is_some(),
            worktree: run.worktree,
            race_id: run.race_id.clone(),
            loop_config: run.loop_config.clone(),
            loop_state: run.loop_state.clone(),
            issue_id: run.issue_id.clone(),
            model: run.model.clone(),
            created_at: run.created_at,
            archived_at: run.archived_at,
            archived: None,
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
            let profile = with_web_ui(
                &profile,
                gui_port_for(&config, Some(port), spec.agent),
                &self.data_dir,
            );
            // The prefix the worktree's tracker briefing names, so `AGE-14`
            // reads to the agent as this project's key rather than a shape it
            // recognizes from some other tracker.
            let (_, key) = self.issue_root(&reg, spec.project_id)?;
            (profile, key)
        };
        let id = new_task_id(id_source(spec.title.as_deref(), spec.prompt));
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
            self.emit_mcp(
                spec.agent,
                &repo,
                &workspace.path,
                &config,
                preview_mcp_port_for(&config, Some(port)),
            );
            // The agent's MCP client connects while its CLI boots, so the
            // preview server must already be listening when the session
            // spawns below — the 2s sweep would be a race.
            self.ensure_preview_server(&id, &repo, Some(port), &config);
            self.emit_skills(
                spec.agent,
                &repo,
                &workspace.path,
                &workspace.branch,
                &issue_key,
                &config,
                spec.loop_config.as_ref(),
                Some(port),
            );
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
            let env = self.agent_env(&profile, &workspace.path, &repo, &id, &id, Some(port))?;
            let conversation = self.open_conversation(&profile.command, &id, &id);
            // The default flow passes "" and behaves exactly as before: the user
            // types the real prompt into the live terminal.
            let (command, args) = fresh_agent_argv(
                &profile,
                &workspace.path,
                spec.prompt,
                config.scripts.setup.as_deref(),
                conversation.as_deref(),
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
            self.kick_web_ui(
                &id,
                spec.agent,
                gui_port_for(&config, Some(port), spec.agent),
                &workspace.path,
                spec.prompt,
            );
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
            // Resolved now, while the answer is unambiguous. Once this branch
            // merges, `merge-base(base, branch)` is the branch's own tip and
            // the range that names its commits is gone — so an archive record
            // written after a successful merge would list nothing at all
            // without this. Best-effort: a run whose base won't resolve still
            // starts, and its record falls back to the merge base.
            base_commit: spec.worktree.then(|| agency_core::merge::rev(&repo, spec.base)).flatten(),
            standing: None,
            pin_rank: None,
            primary_closed_at: None,
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

    /// Fan one prompt out to several attempts in parallel workspaces (racing).
    /// Each attempt is an ordinary run sharing a race_id; the user compares
    /// them and merges the winner. Partial failures leave the already-created
    /// attempts in place (visible and individually discardable).
    pub fn create_race(
        &self,
        project_id: &str,
        prompt: &str,
        attempts: &[RaceAttempt],
        base: &str,
        merge_target: Option<&str>,
    ) -> Result<Vec<RunInfo>> {
        self.create_race_inner(project_id, prompt, attempts, base, merge_target, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_race_inner(
        &self,
        project_id: &str,
        prompt: &str,
        attempts: &[RaceAttempt],
        base: &str,
        merge_target: Option<&str>,
        title: Option<String>,
        issue_id: Option<String>,
    ) -> Result<Vec<RunInfo>> {
        validate_race(prompt, attempts)?;
        let race_id = uuid::Uuid::new_v4().to_string();
        let mut out = Vec::new();
        for attempt in attempts {
            out.push(self.create_run_spec(
                NewRunSpec {
                    project_id,
                    prompt,
                    agent: &attempt.agent,
                    model: attempt.model.as_deref(),
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
        max_wall_secs: Option<u64>,
        max_tokens: Option<u64>,
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
            max_wall_secs,
            max_tokens,
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
        max_wall_secs: Option<u64>,
        max_tokens: Option<u64>,
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
            // None = off, and stays off: clamping must never invent a cap the
            // user did not set. Set values are bounded to one minute..one week
            // and 1k..1B tokens, the same spirit as the attempt clamp above.
            max_wall_secs: max_wall_secs.map(|s| s.clamp(60, 604_800)),
            max_tokens: max_tokens.map(|t| t.clamp(1_000, 1_000_000_000)),
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

    /// Race several attempts on one issue; every attempt links the issue.
    pub fn start_issue_race(
        &self,
        issue_id: &str,
        attempts: &[RaceAttempt],
        base: Option<&str>,
        merge_target: Option<&str>,
    ) -> Result<Vec<RunInfo>> {
        let (issue, prompt, title) = self.issue_dispatch(issue_id)?;
        let base = self.issue_base(&issue.project_id, base)?;
        let out = self.create_race_inner(
            &issue.project_id,
            &prompt,
            attempts,
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
        max_wall_secs: Option<u64>,
        max_tokens: Option<u64>,
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
            max_wall_secs,
            max_tokens,
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
        // Preserved from the file for the same reason `extra` is: the identity
        // in the file may have been minted on another machine, and this write
        // (a status flip, a retitle) is no reason to overwrite it with ours.
        let uid = current.as_ref().and_then(|f| f.uid.clone()).or_else(|| Some(issue.id.clone()));
        // The thread belongs to the file, not to the index row: an agent can
        // append a comment while the app has the issue open, and every write
        // from here (title, body, status, links) carries over what the file
        // says rather than the row's possibly older copy. Comments are written
        // only by `edit_issue_comments`, which parses the file first.
        let comments = current.map_or_else(|| issue.comments.clone(), |f| f.comments);
        let file = issuefs::IssueFile {
            key,
            uid,
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
                uid: Some(row.id.clone()),
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

    /// This project's backlog-sharing settings, as the settings UI shows them.
    pub fn issue_sync_config(&self, project_id: &str) -> Result<IssueSyncDto> {
        let repo = self.project_repo(project_id)?;
        let cfg = agency_core::config::load(&repo).issues;
        Ok(IssueSyncDto {
            sync: cfg.sync,
            remote: cfg.remote,
            remotes: agency_core::git::remotes(&repo).unwrap_or_default(),
            from_repo: agency_core::config::issue_sync_declared_by_repo(&repo),
        })
    }

    /// Persist this project's backlog-sharing settings to its local config.
    pub fn save_issue_sync_config(&self, project_id: &str, sync: bool, remote: &str) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        let remote = remote.trim();
        let remote = if remote.is_empty() { "origin".to_string() } else { remote.to_string() };
        agency_core::config::save_issues(
            &repo,
            &agency_core::config::IssuesConfig { sync, remote },
        )?;
        Ok(())
    }

    /// Sync this project's backlog with the remote its config names, then put
    /// the index back in step with the files the merge changed.
    ///
    /// `mode` is the caller's call, not ours: two already-populated machines
    /// syncing for the first time share no history, so there is no base to
    /// merge against and only the user can say which side seeds the other. The
    /// error for that case is `issueref::Blocked::NeedsSeeding`, which carries
    /// both counts so the prompt can state them.
    pub fn sync_issues(
        &self,
        project_id: &str,
        mode: agency_core::issuesync::Mode,
        mut on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<SyncResult> {
        let reg = self.registry.lock().unwrap();
        self.ensure_issue_files(&reg, project_id)?;
        let (root, key) = self.issue_root(&reg, project_id)?;
        let remote = agency_core::config::issue_sync_remote(&root).ok_or_else(|| {
            anyhow!("this project's backlog is not set to sync; turn it on in settings first")
        })?;
        let pass =
            agency_core::issueref::sync_with_progress(&root, &remote, mode, &mut on_progress);
        let outcome = match pass {
            Ok(o) => o,
            Err(e) => {
                // A question, not a failure: hand it back as an outcome so the
                // UI can prompt rather than parse a message.
                if let Some(agency_core::issueref::Blocked::NeedsSeeding { local, remote }) =
                    e.downcast_ref::<agency_core::issueref::Blocked>()
                {
                    return Ok(SyncResult::NeedsSeeding { local: *local, remote: *remote });
                }
                return Err(e);
            }
        };
        // The merge wrote issue files behind the index's back. Drop the cached
        // stat signature first: `list_issues` skips reconciling when it has not
        // moved, and a sync that lands during the same second as an app write
        // could otherwise leave the board showing the pre-merge state.
        self.issue_sigs.lock().unwrap().remove(project_id);
        on_progress(agency_core::setup::CloneProgress {
            phase: "Updating the board".into(),
            percent: None,
            detail: String::new(),
        });
        agency_core::issuefs::reconcile(&reg, project_id, &key, &root)?;
        Ok(SyncResult::Done { outcome })
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
        // Nothing of ours has the branch, but the user's own checkout may:
        // work there rather than failing on git's "already used by worktree".
        let workspace = pr_workspace(&repo, &pr.head_ref_name);
        if let PrWorkspace::Held(holder) = &workspace {
            return Err(branch_held_elsewhere(&pr.head_ref_name, holder));
        }
        agency_core::git::fetch_branch(&repo, &pr.head_ref_name)?;
        let prompt = format!(
            "Review GitHub pull request #{number}: {title}. Its branch is checked out in this workspace. PR link: {url}",
            title = pr.title,
            url = pr.url,
        );
        let own_worktree = workspace == PrWorkspace::Worktree;
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
                existing_branch: own_worktree.then(|| pr.head_ref_name.clone()),
                loop_config: None,
                issue_id: None,
                worktree: own_worktree,
            },
            &mut |_| {},
        )
    }

    /// Start an agent that reviews an existing PR and then sticks around to fix
    /// what it found. The agent works in the PR's head branch, so its fixes
    /// commit and push straight onto the PR.
    ///
    /// git allows a branch in only one worktree, and fixes have to land on that
    /// branch anyway, so the review goes wherever the branch already is:
    ///
    /// - a tree an agent run of ours already lives in (the usual case for a PR
    ///   an Agency agent opened from the Approve window): an extra agent tab
    ///   inside that run, still a fresh agent with no memory of writing the
    ///   code;
    /// - the project's own checkout: a run in that checkout;
    /// - nowhere: a review worktree of its own, cut on the branch.
    ///
    /// Only a tree Agency cannot put an agent in refuses the review, and it
    /// says which tree.
    pub fn create_pr_review_run(
        &self,
        project_id: &str,
        number: u64,
        agent: &str,
        model: Option<&str>,
        post_comments: bool,
    ) -> Result<PrAgentRun> {
        let repo = self.project_repo(project_id)?;
        let pr = agency_core::gh::GhCli::default()
            .view_pr_by_number(&repo, number)?
            .ok_or_else(|| anyhow!("PR #{number} not found"))?;
        if pr.head_ref_name.is_empty() {
            bail!("PR #{number} has no local head branch (cross-fork PRs aren't supported yet)");
        }
        // Where the branch is, from git rather than from a run's recorded
        // branch: that record goes stale when a checkout moves off the branch,
        // and it is the disagreement between the two that produced AGE-169.
        let workspace = pr_workspace(&repo, &pr.head_ref_name);
        let prompt = pr_review_prompt(
            number,
            &pr.title,
            &pr.url,
            &pr.base_ref_name,
            workspace.clone(),
            post_comments,
        );
        self.spawn_pr_agent(
            project_id,
            &repo,
            &pr,
            agent,
            model,
            format!("Review PR #{number}"),
            &prompt,
            workspace,
        )
    }

    /// Put an agent on the PR's head branch with `prompt`, in whichever tree
    /// already holds that branch or in a worktree cut for it. Shared by the
    /// review and conflict-resolution entry points: both need the same branch
    /// under the same rule, and only the prompt and the title differ.
    ///
    /// `workspace` is passed in rather than probed here because the prompt has
    /// to describe the tree the agent lands in, so the caller has already had
    /// to ask.
    #[allow(clippy::too_many_arguments)]
    fn spawn_pr_agent(
        &self,
        project_id: &str,
        repo: &Path,
        pr: &agency_core::gh::PrInfo,
        agent: &str,
        model: Option<&str>,
        title: String,
        prompt: &str,
        workspace: PrWorkspace,
    ) -> Result<PrAgentRun> {
        // A live agent run already in the holding tree hosts the work as an
        // extra tab: the change has to land on that branch either way, and this
        // keeps two agents from editing one tree without either knowing.
        if let Some(dir) = workspace.holder(repo) {
            let host = self
                .registry
                .lock()
                .unwrap()
                .list_runs(project_id)?
                .into_iter()
                .find(|r| r.kind == "agent" && same_dir(&workspace_dir(repo, r), &dir));
            if let Some(run) = host {
                let session = self.start_run_session(&run.id, Some(agent), prompt)?;
                return Ok(PrAgentRun { run: self.run_info(&run), session_id: Some(session.id) });
            }
        }
        // Before the fetch: a branch this app cannot get a workspace on is a
        // refusal the user should see at once, not after a network round trip.
        if let PrWorkspace::Held(holder) = &workspace {
            return Err(branch_held_elsewhere(&pr.head_ref_name, holder));
        }
        agency_core::git::fetch_branch(repo, &pr.head_ref_name)?;
        let own_worktree = workspace == PrWorkspace::Worktree;
        let run = self.create_run_spec(
            NewRunSpec {
                project_id,
                prompt,
                agent,
                model,
                base: &pr.base_ref_name,
                merge_target: Some(&pr.base_ref_name),
                race_id: None,
                title: Some(title),
                existing_branch: own_worktree.then(|| pr.head_ref_name.clone()),
                loop_config: None,
                issue_id: None,
                worktree: own_worktree,
            },
            &mut |_| {},
        )?;
        Ok(PrAgentRun { run, session_id: None })
    }

    /// Start an agent whose job is to clear a PR's merge conflicts: merge the
    /// base branch into the PR's head branch, resolve, and push, which is what
    /// makes GitHub's Merge button work again.
    ///
    /// The conflicting paths are probed here and written into the prompt. The
    /// agent could find them itself, but a prompt that already names them is
    /// one that starts on the actual work.
    pub fn create_pr_conflict_run(
        &self,
        project_id: &str,
        number: u64,
        agent: &str,
        model: Option<&str>,
    ) -> Result<PrAgentRun> {
        let repo = self.project_repo(project_id)?;
        let pr = agency_core::gh::GhCli::default()
            .view_pr_by_number(&repo, number)?
            .ok_or_else(|| anyhow!("PR #{number} not found"))?;
        if pr.head_ref_name.is_empty() {
            bail!("PR #{number} has no local head branch (cross-fork PRs aren't supported yet)");
        }
        // Best effort: an unprobeable conflict still gets an agent, just without
        // the file list. Refusing to start over a failed probe would be worse
        // than starting slightly less informed.
        let files = self.pr_conflicts(project_id, number).map(|c| c.files).unwrap_or_default();
        let workspace = pr_workspace(&repo, &pr.head_ref_name);
        let prompt = pr_conflict_prompt(
            number,
            &pr.title,
            &pr.url,
            &pr.base_ref_name,
            &pr.head_ref_name,
            workspace.clone(),
            &files,
        );
        self.spawn_pr_agent(
            project_id,
            &repo,
            &pr,
            agent,
            model,
            format!("Fix conflicts on PR #{number}"),
            &prompt,
            workspace,
        )
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
        self.name_the_default_model(&repo);
        let k = agency_core::config::load(&repo).knowledge;
        let probe = self.backend_probe();
        let serve_default = agency_core::config::default_serve_command(&repo);
        let build_default = agency_core::config::default_build_command(probe.claude_on_path);
        let serve_effective = k.serve_command.clone().unwrap_or_else(|| serve_default.clone());
        let build_effective = k.build_command.clone().unwrap_or_else(|| build_default.clone());
        let (build_backend, build_model) = agency_core::config::build_selection(&build_effective);
        let graph = agency_core::config::graph_path(&repo);
        let build_state = self.kg_builds.lock().unwrap().get(&repo).cloned().unwrap_or_default();
        let build_log = build_state.log.lock().unwrap().tail(KG_LOG_SHOWN);
        Ok(KnowledgeConfigDto {
            graph: k.graph,
            rebuild_on_merge: k.rebuild_on_merge,
            serve_installed: first_token_on_path(&serve_effective),
            build_installed: first_token_on_path(&build_effective),
            serve_command: k.serve_command,
            build_command: k.build_command,
            serve_default,
            build_default,
            backends: agency_core::config::knowledge_backends(&probe),
            build_backend,
            build_model,
            graph_built: graph.is_file(),
            graph_path: graph.display().to_string(),
            building: build_state.running,
            build_elapsed_secs: build_state.started.map(|t| t.elapsed().as_secs()),
            last_build_secs: build_state.took.map(|d| d.as_secs()),
            build_log,
            last_build_error: build_state.error,
            last_build_stopped: build_state.stopped && !build_state.running,
            install_command: graphify_install_script(),
        })
    }

    /// What this machine can run a graph build on. Probed per call, not cached:
    /// an agent CLI installed since app start counts, and so does a local model
    /// URL the user just saved.
    fn backend_probe(&self) -> agency_core::config::BackendProbe {
        agency_core::config::BackendProbe {
            claude_on_path: command_on_path("claude"),
            ollama: command_on_path("ollama")
                || std::env::var_os("OLLAMA_BASE_URL").is_some()
                || std::env::var_os("OLLAMA_HOST").is_some(),
            local_model_url: self.get_settings().ok().map(|s| s.lm_studio_base_url),
            env_keys: agency_core::config::env_backend_keys(|var| {
                std::env::var(var).ok().filter(|v| !v.trim().is_empty())
            }),
        }
    }

    /// Write a chosen backend (and optional model) into the project's build
    /// command. The choice *is* the command: one string the settings panel
    /// shows, the build runs and the user can still edit by hand.
    ///
    /// A backend picked without a model gets that backend's own default written
    /// in, rather than an unnamed model the CLI or the vendor chooses at build
    /// time: a claude-CLI build left unnamed answered on the plan's default
    /// model and spent a 5-hour usage window in 30 minutes.
    pub fn set_knowledge_backend(
        &self,
        project_id: &str,
        backend: &str,
        model: &str,
    ) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        let k = agency_core::config::load(&repo).knowledge;
        let url = self.get_settings().map(|s| s.lm_studio_base_url).unwrap_or_default();
        let model = match model.trim() {
            "" => agency_core::config::default_model_for(backend, &self.backend_probe()),
            named => named.to_string(),
        };
        let build = agency_core::config::build_command_for(backend, &model, &url);
        agency_core::config::save_knowledge(
            &repo,
            &agency_core::config::KnowledgeConfig { build_command: Some(build), ..k },
        )?;
        Ok(())
    }

    /// Fill in the model of a saved build command that names none, once, in
    /// place. Best effort and silent: this is a migration, not an action the
    /// user took, and a project whose config cannot be written is a project
    /// whose build still runs.
    ///
    /// Run on both paths that reach a saved command, because they are reached
    /// in either order: the panel (so what it shows is what would run) and the
    /// start of a build (so the post-merge rebuild is repaired for a user who
    /// never opens the panel, which is exactly the user it burned).
    fn name_the_default_model(&self, repo: &Path) {
        let k = agency_core::config::load(repo).knowledge;
        // Only a command actually saved for this project. An unset one already
        // resolves to `default_build_command`, which names its model.
        let Some(saved) = k.build_command.as_deref() else { return };
        let url = self.get_settings().map(|s| s.lm_studio_base_url).unwrap_or_default();
        let Some(repaired) =
            agency_core::config::name_the_default_model(saved, &self.backend_probe(), &url)
        else {
            return;
        };
        let named = agency_core::config::KnowledgeConfig { build_command: Some(repaired), ..k };
        if let Err(e) = agency_core::config::save_knowledge(repo, &named) {
            log::warn!("naming the graph build's model in {}: {e}", repo.display());
        }
    }

    /// Persist a project's knowledge-graph config into its (gitignored) local
    /// override file. Empty command strings clear the override (runtime default
    /// applies) rather than persisting a blank command.
    pub fn save_knowledge_config(
        &self,
        project_id: &str,
        graph: bool,
        rebuild_on_merge: bool,
        serve_command: Option<String>,
        build_command: Option<String>,
    ) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        let clean = |s: Option<String>| s.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
        let k = agency_core::config::KnowledgeConfig {
            graph,
            rebuild_on_merge,
            serve_command: clean(serve_command),
            build_command: clean(build_command),
        };
        agency_core::config::save_knowledge(&repo, &k)?;
        if graph {
            // Enabling is where the panel starts showing the build command, and
            // a user who runs it in their own terminal gets the same
            // graphify-out/ in their changes as the Build button would. The
            // exclude belongs to the feature, not to who pressed what.
            agency_core::config::exclude_graph_output(&repo);
        }
        // Enabling deliberately does *not* start a build (it did until AGE-83's
        // follow-up). A build reads every doc in the project with an LLM, and
        // which one it uses, what that costs and who ends up with the corpus
        // are all things a user has to see before it runs, not discover from a
        // plan's usage page afterwards. Flicking a toggle is not that consent.
        // The panel answers the "then nothing happens" complaint the auto-build
        // was for: enabling reveals the backend picker, each option's cost, and
        // the Build button that spends it.
        Ok(())
    }

    /// Build (or rebuild) a project's knowledge graph now, from the UI.
    pub fn build_knowledge_graph(&self, project_id: &str) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        self.start_knowledge_build(&repo)
    }

    /// The Map's view of a project's knowledge graph: the primary repo's
    /// `graphify-out/graph.json` reduced to the drill-down view model.
    ///
    /// `Ok(None)` means no graph has been built yet, which is an empty state
    /// and not a failure; the tab words it from the knowledge config. Anything
    /// else that goes wrong is a real error and says so, because collapsing
    /// the two told the user "no graph has been built yet" about a graph that
    /// was sitting right there, and offered a Build button that could not fix
    /// whatever had actually happened.
    pub fn knowledge_graph_view(
        &self,
        project_id: &str,
    ) -> Result<Option<agency_core::graphview::GraphView>> {
        let repo = self.project_repo(project_id)?;
        let path = agency_core::config::graph_path(&repo);
        let size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(anyhow!("reading {}: {e}", path.display())),
        };
        // A graph this size is far outside what the viewer can render, and
        // parsing it costs several times the file in memory; 40 MB is ~6x the
        // graph of this repository.
        const MAX_GRAPH_BYTES: u64 = 40 * 1024 * 1024;
        if size > MAX_GRAPH_BYTES {
            anyhow::bail!("graph.json is {} MB, too large to map", size / (1024 * 1024));
        }
        let text = std::fs::read_to_string(&path)?;
        let repo_dir = repo.file_name().and_then(|n| n.to_str());
        agency_core::graphview::view(&text, repo_dir).map(Some)
    }

    /// Install the graphify tooling in a visible Agency terminal, the same way
    /// a missing agent CLI is installed: the user watches it run and can answer
    /// anything it asks, rather than Agency mutating their machine silently.
    /// Returns the terminal run to jump into.
    pub fn install_knowledge_tooling(&self, project_id: &str) -> Result<RunInfo> {
        self.create_install_terminal(project_id, "graphify", &graphify_install_script())
    }

    /// Stop a running graph build. The panel's escape hatch from a build that
    /// is going nowhere: without it a wedged build holds `running` forever, and
    /// with it every later build (the Build button, every post-merge rebuild)
    /// is refused as "already running" until the app restarts.
    pub fn stop_knowledge_build(&self, project_id: &str) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        let child = {
            let mut builds = self.kg_builds.lock().unwrap();
            let entry = builds
                .get_mut(&repo)
                .filter(|b| b.running)
                .ok_or_else(|| anyhow!("no graph build is running for this project"))?;
            entry.stopped = true;
            entry.child.clone()
        };
        if let Some(child) = child {
            signal_build_stop(&child);
        }
        Ok(())
    }

    /// Spawn the project's build command in the primary checkout, tracking it in
    /// `kg_builds` so the UI can show progress and failures. Returns an error
    /// (without spawning) when the tooling is missing or a build is already
    /// running for this repo — both are states the caller reports, not retries.
    fn start_knowledge_build(&self, repo: &Path) -> Result<()> {
        self.name_the_default_model(repo);
        let config = agency_core::config::load(repo);
        let build = config.knowledge.build_command.clone().unwrap_or_else(|| {
            agency_core::config::default_build_command(command_on_path("claude"))
        });
        let cmd = agency_core::config::command_binary(&build)
            .ok_or_else(|| anyhow!("the build command is empty"))?;
        if !command_on_path(&cmd) {
            return Err(anyhow!(
                "'{cmd}' is not installed. Install the graphify tooling and try again."
            ));
        }
        let buildlog = Arc::new(Mutex::new(agency_core::buildlog::BuildLog::default()));
        // Claim the slot before spawning: two Build presses a moment apart must
        // not both get a process.
        {
            let mut builds = self.kg_builds.lock().unwrap();
            let entry = builds.entry(repo.to_path_buf()).or_default();
            if entry.running {
                return Err(anyhow!("a graph build is already running for this project"));
            }
            *entry = KgBuild {
                running: true,
                started: Some(Instant::now()),
                log: buildlog.clone(),
                ..Default::default()
            };
        }
        // Before the build writes a line: graphify-out/ is ours to keep out of
        // the user's changes (AGE-170), and an exclude arriving after the files
        // do has already lost them a diff.
        agency_core::config::exclude_graph_output(repo);
        log::info!("building knowledge graph in {}: {build}", repo.display());
        let repo = repo.to_path_buf();
        let builds_handle = self.kg_builds.clone();

        // A login shell so the build resolves the same tooling the user's
        // terminal does, and piped output so the panel can show what the build
        // is doing instead of a spinner that never moves.
        let mut command = std::process::Command::new("sh");
        command
            .args(["-lc", &build])
            .current_dir(&repo)
            // graphify is Python, and Python block-buffers stdout when it is a
            // pipe: without this its progress lines arrive in 8 KB batches, so
            // a build that prints steadily for ten minutes looks silent for
            // nine of them. Harmless for a build command that is not Python.
            .env("PYTHONUNBUFFERED", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // Its own process group, so stopping the build can reach the agent CLI
        // graphify spawns per document rather than only the process Agency
        // started. `sh -lc <one command>` execs into the command, so the child
        // is graphify itself and its pid is the group's.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                let reason = format!("couldn't run the build command: {e}");
                let mut builds = builds_handle.lock().unwrap();
                let entry = builds.entry(repo).or_default();
                entry.running = false;
                entry.started = None;
                entry.error = Some(reason.clone());
                return Err(anyhow!(reason));
            }
        };
        let readers = [
            child.stdout.take().map(|p| read_build_output(p, Stream::Out, buildlog.clone())),
            child.stderr.take().map(|p| read_build_output(p, Stream::Err, buildlog.clone())),
        ];
        let child = Arc::new(Mutex::new(child));
        {
            let mut builds = builds_handle.lock().unwrap();
            builds.entry(repo.clone()).or_default().child = Some(child.clone());
        }
        std::thread::spawn(move || {
            let status = wait_for_build(&child, &builds_handle, &repo);
            // Give the readers a moment to drain what is left in the pipes, but
            // do not join them: a grandchild that inherited the pipe and
            // outlived its parent holds it open indefinitely, and waiting on
            // that would leave the panel saying "Building" for a build that has
            // already exited.
            let drained = Instant::now() + Duration::from_secs(2);
            for r in readers.iter().flatten() {
                while !r.is_finished() && Instant::now() < drained {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            let mut builds = builds_handle.lock().unwrap();
            let entry = builds.entry(repo.clone()).or_default();
            let error = match status {
                // Stopping is not failing: the panel says so from `stopped`.
                _ if entry.stopped => None,
                Ok(s) if s.success() => None,
                Ok(s) => Some(match buildlog.lock().unwrap().failure_tail(4) {
                    Some(tail) => tail,
                    None => format!("the build command exited with {s}"),
                }),
                Err(e) => Some(format!("couldn't wait for the build command: {e}")),
            };
            if let Some(e) = &error {
                log::warn!("knowledge graph build failed in {}: {e}", repo.display());
            }
            entry.running = false;
            entry.took = entry.started.take().map(|t| t.elapsed());
            entry.child = None;
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
    ///
    /// `preview_port` is the run's preview MCP server (AGE-143), merged in
    /// last so it rides the same emission as every other server. The caller
    /// computes it with [`preview_mcp_port_for`] so the entry and the server
    /// that answers on it can never disagree about the port.
    fn emit_mcp(
        &self,
        agent: &str,
        repo: &Path,
        worktree: &Path,
        config: &agency_core::config::AgencyConfig,
        preview_port: Option<u16>,
    ) {
        let mut servers = self.merged_mcp_servers(repo, config);
        if let Some(port) = preview_port {
            servers =
                agency_core::mcp::merge(&[servers, vec![agency_core::preview::server_entry(port)]]);
        }
        if servers.is_empty() {
            return;
        }
        match agency_core::mcp::emit_for_agent(agent, worktree, repo, &servers) {
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

    /// Emit the skills kit into a worktree, beside the MCP config and the
    /// tracker briefing. Best-effort, for the same reason those are: a kit
    /// that can't be written is not worth failing a run over.
    ///
    /// The facts are gathered here and baked into the emitted file. An agent
    /// told to go read a config file to find its check command has not been
    /// told its check command.
    fn emit_skills(
        &self,
        agent: &str,
        repo: &Path,
        worktree: &Path,
        branch: &str,
        issue_key: &str,
        config: &agency_core::config::AgencyConfig,
        loop_config: Option<&agency_core::loops::LoopConfig>,
        port: Option<u16>,
    ) {
        let ws = agency_core::skills::Workspace {
            worktree: worktree.to_path_buf(),
            repo_root: repo.to_path_buf(),
            issue_key: issue_key.to_string(),
            branch: branch.to_string(),
            setup_command: config.scripts.setup.clone().filter(|s| !s.trim().is_empty()),
            run_scripts: config
                .scripts
                .run_list()
                .into_iter()
                .map(|r| (r.name, r.command))
                .collect(),
            // An empty check command is the "fixed iterations" loop, which has
            // no verifier to name.
            loop_check: loop_config
                .filter(|c| !c.check_command.trim().is_empty())
                .map(|c| (c.check_command.clone(), c.max_attempts)),
            port,
            preview_tools: preview_mcp_port_for(config, port).is_some(),
        };
        if let Err(e) = agency_core::skills::emit_for_agent(agent, &ws) {
            log::warn!("emitting the skills kit for {agent} into {}: {e}", worktree.display());
        }
    }

    /// The hooks one run's preview server calls back through: facts read
    /// fresh per call (so guidance in tool errors never names a stale
    /// command), screenshots via whatever the app shell installed.
    fn preview_hooks(&self, run_id: &str, repo: &Path) -> agency_core::preview::Hooks {
        let repo = repo.to_path_buf();
        let facts = std::sync::Arc::new(move || {
            let config = agency_core::config::load(&repo);
            agency_core::preview::Facts {
                script: config
                    .scripts
                    .run_list()
                    .into_iter()
                    .find(|s| s.web)
                    .map(|s| (s.name, s.command)),
            }
        });
        let shot_cell = self.preview_shot.clone();
        let run_id = run_id.to_string();
        let screenshot = std::sync::Arc::new(move || match shot_cell.get() {
            Some(shot) => shot(&run_id),
            None => Err("Screenshots need the Agency app window, which is not available right \
                         now. preview_snapshot works without it."
                .to_string()),
        });
        agency_core::preview::Hooks { facts, screenshot }
    }

    /// Have this run's preview server listening on the port its emitted MCP
    /// config names, if the run should have one at all. Idempotent; a server
    /// already on the right ports is left alone.
    fn ensure_preview_server(
        &self,
        run_id: &str,
        repo: &Path,
        port_base: Option<u16>,
        config: &agency_core::config::AgencyConfig,
    ) {
        let Some(bind) = preview_mcp_port_for(config, port_base) else { return };
        let Some(app_port) = port_base else { return };
        let mut servers = self.preview.lock().unwrap();
        if servers.get(run_id).is_some_and(|s| s.port() == bind && s.app_port() == app_port) {
            return;
        }
        servers.remove(run_id);
        match agency_core::preview::PreviewServer::start(
            bind,
            app_port,
            self.preview_hooks(run_id, repo),
        ) {
            Ok(srv) => {
                servers.insert(run_id.to_string(), srv);
                self.preview_failures.lock().unwrap().remove(run_id);
            }
            Err(e) => {
                // Warn once per run, not once per 2s sweep; the sweep retries
                // on the failure map's cadence.
                let mut failures = self.preview_failures.lock().unwrap();
                if failures.insert(run_id.to_string(), Instant::now()).is_none() {
                    log::warn!(
                        "preview server for run {run_id} could not bind 127.0.0.1:{bind}: {e:#}; \
                         the agent's preview tools stay dark until that port frees up"
                    );
                }
            }
        }
    }

    /// Converge the preview servers on what the registry and each project's
    /// config say should exist. Called on the notifier tick, so runs being
    /// archived, discarded or restored, and `[preview]`/run-script edits, all
    /// take effect within ~2s without every one of those paths owning
    /// teardown.
    pub fn sync_preview_servers(&self) {
        let runs: Vec<(String, PathBuf, Option<u16>)> = {
            let reg = self.registry.lock().unwrap();
            let Ok(projects) = reg.list_projects() else { return };
            projects
                .iter()
                .filter_map(|p| reg.list_runs(&p.id).ok().map(|runs| (p.repo_path.clone(), runs)))
                .flat_map(|(repo, runs)| {
                    runs.into_iter()
                        .filter(|r| r.worktree && r.kind == "agent")
                        .map(move |r| (r.id, repo.clone(), r.port_base))
                })
                .collect()
        };
        let mut desired: HashMap<String, (PathBuf, Option<u16>, u16)> = HashMap::new();
        for (id, repo, port_base) in runs {
            let config = agency_core::config::load(&repo);
            if let Some(bind) = preview_mcp_port_for(&config, port_base) {
                desired.insert(id, (repo, port_base, bind));
            }
        }
        // Stop the no-longer-wanted before starting anything, so a port freed
        // by one run (a restore that reallocated its block, say) can be
        // rebound by another in the same pass. A young server gets a grace
        // period instead: `create_run_spec` starts the server before the run's
        // registry row exists (the agent's MCP client connects as the CLI
        // boots), and a tick landing in that window would tear down what was
        // just deliberately started. `ensure_preview_server` still replaces a
        // mismatched young server directly, so the grace never delays a port
        // move — only this sweep's deletions.
        self.preview.lock().unwrap().retain(|id, srv| {
            desired
                .get(id)
                .is_some_and(|(_, pb, bind)| srv.port() == *bind && Some(srv.app_port()) == *pb)
                || srv.age() < Duration::from_secs(15)
        });
        self.preview_rects.lock().unwrap().retain(|id, _| desired.contains_key(id));
        self.preview_failures.lock().unwrap().retain(|id, _| desired.contains_key(id));
        for (id, (repo, port_base, _)) in desired {
            let recently_failed = self
                .preview_failures
                .lock()
                .unwrap()
                .get(&id)
                .is_some_and(|at| at.elapsed() < Duration::from_secs(60));
            if recently_failed {
                continue;
            }
            let config = agency_core::config::load(&repo);
            self.ensure_preview_server(&id, &repo, port_base, &config);
        }
    }

    /// Every run with a live preview server, for the frontend's hidden-host
    /// keeper. `active` is a liveness probe of the run's app port: the moment
    /// anything serves there, a preview host is worth mounting (however the
    /// dev server was started — Run tab or the agent's own shell).
    pub fn preview_targets(&self) -> Vec<PreviewTargetDto> {
        self.preview
            .lock()
            .unwrap()
            .iter()
            .map(|(run_id, srv)| PreviewTargetDto {
                run_id: run_id.clone(),
                url: srv.preview_url(),
                active: agency_core::preview::serving(srv.app_port()),
            })
            .collect()
    }

    /// The Run tab reporting where (and whether) a run's preview pane is on
    /// screen; `None` clears it. Feeds the native screenshot crop.
    pub fn set_preview_rect(&self, run_id: &str, rect: Option<PreviewRect>) {
        let mut rects = self.preview_rects.lock().unwrap();
        match rect {
            Some(r) => {
                rects.insert(run_id.to_string(), r);
            }
            None => {
                rects.remove(run_id);
            }
        }
    }

    pub fn preview_rect(&self, run_id: &str) -> Option<PreviewRect> {
        self.preview_rects.lock().unwrap().get(run_id).copied()
    }

    /// Installed once by the app shell; see [`PreviewShotFn`].
    pub fn set_preview_shot(&self, shot: PreviewShotFn) {
        let _ = self.preview_shot.set(shot);
    }

    /// The issue key prefix a project's tracker uses, for the workspace
    /// catalog. Falls back to the same default [`issue_root`] uses.
    fn issue_key_for(&self, project_id: &str) -> String {
        let reg = self.registry.lock().unwrap();
        self.issue_root(&reg, project_id).map(|(_, key)| key).unwrap_or_else(|_| "ISSUE".into())
    }

    /// After a clean merge, rebuild the project's knowledge graph in the
    /// background so the next agent workspace starts with a fresh graph.
    ///
    /// A *re*build, strictly: with no graph on disk the user has never picked a
    /// model or agreed to what a build of this project costs, and landing a
    /// merge is not the moment to decide that for them. The first build is
    /// always the one they press.
    fn maybe_rebuild_knowledge_graph(&self, repo: &Path) {
        let k = agency_core::config::load(repo).knowledge;
        if !k.graph || !k.rebuild_on_merge {
            return;
        }
        if !agency_core::config::graph_path(repo).is_file() {
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
            base_commit: None,
            primary_closed_at: None,
            standing: None,
            pin_rank: None,
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
        let live = self.term.read().unwrap().list().unwrap_or_default();
        let mut out = Vec::new();
        for proj in projects {
            let runs = self.registry.lock().unwrap().list_runs(&proj.id)?;
            for run in runs {
                let status = self
                    .term
                    .read()
                    .unwrap()
                    .status(&lead_session_name(&run, &live))
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
            base_commit: None,
            primary_closed_at: None,
            standing: None,
            pin_rank: None,
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
        // seemingly random times for runs the user never prompted. The same
        // classification feeds the send queue's draft and echo-grace rules.
        let typed = crate::sendq::classify_input(data);
        if typed == crate::sendq::Typed::Submitted {
            self.input_seen.lock().unwrap().insert(id.to_string());
            self.prompted.lock().unwrap().insert(id.to_string());
        }
        self.human_input
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_default()
            .observe(typed, crate::activity::now_ms());
        Ok(())
    }

    /// Queue `text` for the run's session and try to deliver it immediately.
    ///
    /// Returns whether it went out now; `false` means it is waiting for the
    /// agent to finish its turn (or for the human to send whatever they are
    /// half-way through typing) and the notifier tick will deliver it.
    fn queue_send(&self, id: &str, origin: &'static str, text: String) -> Result<bool> {
        let msg = crate::sendq::Queued { text, origin, queued_at_ms: crate::activity::now_ms() };
        {
            let mut queues = self.send_queue.lock().unwrap();
            if !queues.entry(id.to_string()).or_default().push(msg) {
                bail!(
                    "this agent already has {} messages waiting; it hasn't taken any of them yet",
                    crate::sendq::MAX_PENDING
                );
            }
        }
        let notice = self.drain_session(id, crate::activity::now_ms());
        // After the drain, not before: the usual case is that it went straight
        // out, and then there is nothing to store.
        self.persist_queue(id);
        match notice {
            // The session died between the sender's own status check and this
            // drain, and the discard took the whole queue with it. Answered
            // here rather than left to a toast on the next tick: the surface
            // that sent it is still open, and it used to be told `Ok(true)`,
            // "it went out now", for a message nothing was ever typed of.
            Some(n) if n.kind == crate::sendq::NoticeKind::Dropped => {
                bail!("that agent's session is gone, so nothing was typed")
            }
            // An older message queued ahead of this one hit its deadline on the
            // way past. Nothing here is watching for that, so hand it to the
            // tick, which is.
            Some(n) => self.queue_notices.lock().unwrap().push(n),
            None => {}
        }
        Ok(self.send_queue.lock().unwrap().get(id).is_none_or(|q| q.is_empty()))
    }

    /// Mirror one session's queue into the registry, so quitting with something
    /// held doesn't lose it with no trace. Best effort: the queue in memory is
    /// what drains, and a write that fails costs the message only if the app is
    /// quit before the next one succeeds.
    fn persist_queue(&self, id: &str) {
        let json = self
            .send_queue
            .lock()
            .unwrap()
            .get(id)
            .filter(|q| !q.is_empty())
            .map(crate::sendq::encode);
        let reg = self.registry.lock().unwrap();
        let stored = match &json {
            Some(j) => reg.set_send_queue(id, j),
            None => reg.delete_send_queue(id),
        };
        if let Err(e) = stored {
            log::warn!("send queue: storing {id} failed: {e}");
        }
    }

    /// Load the queues a previous run of the app left behind.
    ///
    /// Nothing is delivered here. Every restored message goes back to waiting
    /// and is re-decided on the first notifier tick against the session as it
    /// is *now*: a session the daemon no longer hosts discards it (the daemon
    /// outlives the app, so plenty of them are still there), and one that
    /// survived takes it under the ordinary rules.
    fn restore_send_queues(&self, now_ms: i64) {
        let stored = match self.registry.lock().unwrap().list_send_queues() {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("send queue: reading stored queues failed: {e}");
                return;
            }
        };
        let mut restored = 0usize;
        for (id, json) in stored {
            let q = crate::sendq::decode(&json, now_ms);
            if q.is_empty() {
                // Nothing readable in the row: clear it rather than re-reading
                // it on every launch from here on.
                let _ = self.registry.lock().unwrap().delete_send_queue(&id);
                continue;
            }
            restored += q.len();
            self.send_queue.lock().unwrap().insert(id, q);
        }
        if restored > 0 {
            log::info!("send queue: restored {restored} message(s) held at the last quit");
        }
    }

    /// How many messages are waiting for this run, across its own session and
    /// any extra agent tabs sharing its worktree.
    fn queued_message_count(&self, run_id: &str) -> u32 {
        self.send_queue
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _)| split_session_id(id).0 == run_id)
            .map(|(_, q)| q.len() as u32)
            .sum()
    }

    /// What this run is still owed, oldest first, for the marker's popover.
    pub fn list_queued_messages(&self, run_id: &str) -> Vec<QueuedMessageInfo> {
        let queues = self.send_queue.lock().unwrap();
        let mut ids: Vec<&String> =
            queues.keys().filter(|id| split_session_id(id).0 == run_id).collect();
        // The map has no order of its own, and the run's own session sorts
        // before its tabs (`run` < `run--2`), which is the order they are shown
        // in.
        ids.sort();
        ids.iter()
            .flat_map(|id| {
                queues[*id].messages().map(move |m| QueuedMessageInfo {
                    session_id: (*id).clone(),
                    origin: m.origin.to_string(),
                    text: m.text.clone(),
                })
            })
            .collect()
    }

    /// Drop one waiting message. Returns false when it is no longer there — the
    /// drain runs on the notifier tick, so it may have gone out between the
    /// popover being drawn and the click landing.
    ///
    /// The queue is the only thing in Agency that types into a session with
    /// nobody watching, so this exists: what it holds must always be
    /// cancellable.
    pub fn cancel_queued_message(&self, session_id: &str, text: &str) -> bool {
        let dropped = {
            let mut queues = self.send_queue.lock().unwrap();
            let Some(q) = queues.get_mut(session_id) else { return false };
            let dropped = q.remove(text);
            if q.is_empty() {
                queues.remove(session_id);
            }
            dropped
        };
        if dropped {
            log::info!("send queue: dropped a message for {session_id} at the user's request");
            self.persist_queue(session_id);
        }
        dropped
    }

    /// Try to deliver one queued message to every session that has one. Called
    /// once per notifier tick, right after the tick has refreshed the
    /// busy/idle bookkeeping the decision reads. Costs two uncontended locks
    /// when no session has anything queued, which is the usual case.
    ///
    /// Hands back whatever the user has to be told about, this pass and from
    /// any command thread that drained since the last one: the tick is the only
    /// caller with a window to say it in. The pty write stays the caller's
    /// side effect and so does this.
    pub fn drain_send_queues(&self, now_ms: i64) -> Vec<crate::sendq::Notice> {
        let ids: Vec<String> = self.send_queue.lock().unwrap().keys().cloned().collect();
        let mut notices = std::mem::take(&mut *self.queue_notices.lock().unwrap());
        for id in ids {
            notices.extend(self.drain_session(&id, now_ms));
        }
        notices
    }

    /// Deliver at most one queued message to `id`'s session.
    ///
    /// One per pass, not the whole queue: every observation the decision rests
    /// on describes the pane *before* the write, and a second message typed
    /// against that same stale reading would be exactly the mid-turn
    /// interruption this queue exists to prevent.
    ///
    /// Returns the one thing that happened here the user cannot see for
    /// themselves: a queue thrown away, or a message that went in on top of
    /// something. A clean delivery and a hold report nothing — the marker
    /// already covers being held, and its going away covers the rest.
    fn drain_session(&self, id: &str, now_ms: i64) -> Option<crate::sendq::Notice> {
        if self.send_queue.lock().unwrap().get(id).is_none_or(|q| q.is_empty()) {
            return None;
        }
        // The spawn gate rather than one of its own: text must not be typed
        // into a session that a kill-then-spawn is in the middle of replacing,
        // and two drains of one session must not both write the same head.
        // `try_with`, because a drain that queued behind a respawn would come
        // out holding an observation taken before it.
        self.spawn_gates
            .try_with(id, || {
                // Read the head *inside* the gate. The command thread's own
                // drain and the notifier tick's can both reach this point;
                // whichever gets the gate second must see the queue the first
                // one left, not the one it looked at on the way in.
                let Some(head) =
                    self.send_queue.lock().unwrap().get(id).and_then(|q| q.head().cloned())
                else {
                    return None;
                };
                let obs = self.observe_for_send(id, now_ms);
                match crate::sendq::decide(&head, &obs) {
                    crate::sendq::Decision::Hold(_) => None,
                    crate::sendq::Decision::Discard(reason) => {
                        let n = self.send_queue.lock().unwrap().remove(id).map_or(0, |q| q.len());
                        self.persist_queue(id);
                        log::info!("send queue: dropped {n} message(s) for {id} ({reason:?})");
                        // The whole point of AGE-127: this used to end at the
                        // log line, and a marker that appeared for one tick and
                        // then vanished looked exactly like a message going in.
                        Some(crate::sendq::dropped_notice(
                            split_session_id(id).0,
                            &self.session_label(id),
                            head.origin,
                            n,
                        ))
                    }
                    crate::sendq::Decision::Send(reason) => {
                        // The only side effect in the whole mechanism, and
                        // still the same two writes it always was: the text, a
                        // beat, then the carriage return.
                        if let Err(e) =
                            self.term.read().unwrap().send_text(&session_name(id), &head.text)
                        {
                            // Left queued deliberately: if the session is
                            // really gone the next decision discards it, and if
                            // the daemon just blinked the next tick delivers it.
                            log::warn!("send queue: writing {} to {id} failed: {e}", head.origin);
                            return None;
                        }
                        log::info!("send queue: delivered {} to {id} ({reason:?})", head.origin);
                        {
                            let mut queues = self.send_queue.lock().unwrap();
                            if let Some(q) = queues.get_mut(id) {
                                q.pop();
                                if q.is_empty() {
                                    queues.remove(id);
                                }
                            }
                        }
                        self.persist_queue(id);
                        // A clean send is the non-event; an appended one landed
                        // on top of whatever was on the line, which nobody who
                        // has closed the sending surface would otherwise learn.
                        match reason {
                            crate::sendq::SendReason::Clear => None,
                            crate::sendq::SendReason::Appended => {
                                Some(crate::sendq::appended_notice(
                                    split_session_id(id).0,
                                    &self.session_label(id),
                                    head.origin,
                                ))
                            }
                        }
                    }
                }
            })
            .flatten()
    }

    /// What to call the run behind a session id in a sentence the user reads.
    /// Falls back to the id: a notice with an awkward name in it is worth more
    /// than no notice.
    fn session_label(&self, id: &str) -> String {
        let run_id = split_session_id(id).0;
        self.run_record(run_id).map(|r| run_label(&r)).unwrap_or_else(|_| run_id.to_string())
    }

    /// What the send queue needs to know about one session right now.
    fn observe_for_send(&self, id: &str, now_ms: i64) -> crate::sendq::Observation {
        let session_running = matches!(
            self.term.read().unwrap().status(&session_name(id)),
            Ok(SessionStatus::Running)
        );
        // No activity entry means the notifier has never seen this run — a
        // session spawned seconds ago. Unknown reads as working, which holds.
        // `turn_driven` never reaches the Working arm of `classify`, so what it
        // is passed here doesn't matter.
        let working = self
            .activity
            .lock()
            .unwrap()
            .get(id)
            .map(|e| {
                // The standing never reaches the Working arm either, so the
                // send queue reads the pane alone.
                crate::activity::classify(e, false, None, now_ms).state
                    == crate::activity::ActivityState::Working
            })
            .unwrap_or(true);
        let human = self.human_input.lock().unwrap().get(id).copied();
        let mut draft = human.is_some_and(|h| h.draft);
        // One-directional by construction: the pane is read only when a draft
        // is already blocking, and the read can only lift the block. Nothing
        // here can conclude that a draft exists from the pane alone.
        if draft {
            // One line asked for, the whole visible screen returned (the
            // emulator always renders every row); scrollback stays out of it.
            if let Ok(pane) = self.term.read().unwrap().capture(&session_name(id), 1) {
                if crate::sendq::prompt_looks_empty(&pane) {
                    draft = false;
                    if let Some(h) = self.human_input.lock().unwrap().get_mut(id) {
                        h.draft = false;
                    }
                }
            }
        }
        crate::sendq::Observation {
            now_ms,
            session_running,
            working,
            last_key_ms: human.map(|h| h.last_key_ms),
            draft,
        }
    }

    /// Forget the per-session bookkeeping that only lives in memory, for a
    /// session that is being torn down. Anything still queued for it is dropped
    /// with it: there will be no pty to type it into.
    fn forget_session_state(&self, id: &str) {
        self.input_seen.lock().unwrap().remove(id);
        self.prompted.lock().unwrap().remove(id);
        self.human_input.lock().unwrap().remove(id);
        self.send_queue.lock().unwrap().remove(id);
        self.persist_queue(id);
    }

    /// Whether the run has unconsumed user input for idle-notification gating.
    pub fn has_input_pending(&self, id: &str) -> bool {
        self.input_seen.lock().unwrap().contains(id)
    }

    /// Consume the idle gate after a "waiting for input" notification fires so the
    /// run stays quiet until the user drives another turn.
    ///
    /// Every session of the run, not just the one named: the notifier watches
    /// runs, and the session it read the gate off is whichever tab is speaking
    /// for the run (AGE-184). Clearing only `id` would leave the flag set on
    /// that tab and the nudge would fire again on every tick.
    pub fn clear_input_seen(&self, id: &str) {
        self.input_seen.lock().unwrap().retain(|s| split_session_id(s).0 != id);
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
        // Both of these are keyed by *session*, and a run's extra tabs
        // (`<run>--2`) never appear in the watch snapshot — so prune on the run
        // the session belongs to, not on the session id, or every tick would
        // throw away the keystroke history of every open tab.
        let kept_run = |id: &str| keep.contains(split_session_id(id).0);
        self.human_input.lock().unwrap().retain(|id, _| kept_run(id));
        // A run that has left the board (archived, discarded) has no session to
        // type into, so anything still queued for it goes with it — including
        // the stored copy, or it would come back at the next launch for a run
        // that is no longer there.
        let dropped: Vec<String> = {
            let mut queues = self.send_queue.lock().unwrap();
            let dropped =
                queues.keys().filter(|id| !kept_run(id)).cloned().collect::<Vec<String>>();
            queues.retain(|id, _| kept_run(id));
            dropped
        };
        for id in dropped {
            self.persist_queue(&id);
        }
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
                let Some(format) = agency_core::usage::format_for(command) else { continue };
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
                let next = entry.0.refresh(&dir, format);
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

    /// What tearing `id` down would actually remove, asked of git now.
    ///
    /// Both plans come back from one probe because every place that offers a
    /// teardown offers both verbs, and the dialog has to say what each one
    /// costs before the user picks. Every probe is local (`merge-base`,
    /// `rev-list`, `branch --remotes --contains`): this is read while a menu is
    /// opening, and a menu that waits on the network is a menu that hangs.
    pub fn run_cleanup(&self, id: &str) -> Result<RunCleanup> {
        let run = self.run_record(id)?;
        let facts = self.branch_facts(&run);
        let base = self
            .project_repo(&run.project_id)
            .ok()
            .and_then(|repo| {
                agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo).ok()
            })
            .unwrap_or_else(|| run.base.clone());
        // The same two conditions `rescue_transcript` and `discard_run` apply
        // before they move or remove anything, asked here so the words and the
        // filesystem operations cannot drift apart.
        let manages_transcript = run.kind == "agent"
            && run.worktree
            && agency_core::usage::format_for(&self.agent_command(&run)).is_some();
        Ok(RunCleanup {
            archive: agency_core::cleanup::plan(&facts, Disposal::Archive),
            delete: agency_core::cleanup::plan(&facts, Disposal::Delete),
            facts,
            branch: run.branch.clone(),
            base,
            manages_transcript,
        })
    }

    /// Where this run's commits live besides its own branch. Default-deny: a
    /// fact we cannot establish is not asserted, so anything unreadable ends up
    /// keeping the branch rather than deleting it.
    fn branch_facts(&self, run: &agency_core::registry::Run) -> BranchFacts {
        // A terminal, and an agent working in the project's own checkout, own
        // no branch of ours. Their changes are the user's, in the user's
        // checkout, and nothing here may weigh them.
        if run.kind != "agent" || !run.worktree {
            return BranchFacts::default();
        }
        let Ok(repo) = self.project_repo(&run.project_id) else {
            return BranchFacts { owns_branch: true, gone: true, ..BranchFacts::default() };
        };
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)
            .unwrap_or_else(|_| run.base.clone());
        let base_exists = agency_core::merge::branch_exists(&repo, &base);
        if !agency_core::merge::branch_exists(&repo, &run.branch) {
            return BranchFacts {
                owns_branch: true,
                gone: true,
                base_exists,
                ..BranchFacts::default()
            };
        }
        let ahead = agency_core::merge::commits_ahead(&repo, &run.branch, &base).ok();
        let worktree = workspace_dir(&repo, run);
        BranchFacts {
            owns_branch: true,
            commits_ahead: ahead.unwrap_or(0),
            commits_known: ahead.is_some(),
            merged: agency_core::merge::is_merged(&repo, &run.branch, &base),
            pushed: agency_core::merge::is_pushed(&repo, &run.branch),
            gone: false,
            dirty: worktree.exists()
                && agency_core::git::status(&worktree).is_ok_and(|cs| !cs.is_empty()),
            base_exists,
        }
    }

    /// The launch command for this run's agent, from its profile when there is
    /// one. Its basename is what selects the transcript dialect.
    fn agent_command(&self, run: &agency_core::registry::Run) -> String {
        self.profile_command(&run.agent)
    }

    /// The environment every agent launch runs in: the user's provider keys,
    /// the profile's own variables, the run's script variables, and the pin
    /// that keeps this session's conversation to itself.
    ///
    /// One builder for all five launch paths — create, resume, rerun, loop
    /// attempt and extra tab — so a sixth cannot quietly ship without the pin.
    /// AGE-175 was a resume reading a sibling session's conversation, and a
    /// launch that skipped the pin would go on writing into one.
    ///
    /// `run_id` and `session` differ only for an extra agent tab: the scripts
    /// belong to the run whose workspace they operate in, the conversation to
    /// the tab that is having it.
    fn agent_env(
        &self,
        profile: &AgentProfile,
        worktree: &Path,
        repo: &Path,
        run_id: &str,
        session: &str,
        port: Option<u16>,
    ) -> Result<Vec<(String, String)>> {
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        env.extend(agency_core::scripts::script_env(worktree, repo, run_id, port));
        env.extend(session_store_env(&profile.command, worktree, session));
        Ok(env)
    }

    /// Mint the conversation a fresh launch of `session` will open, and record
    /// it against the session so a later resume can reopen that exact one
    /// (AGE-177). `None` for an agent that does not name conversations, which
    /// is what keeps its own resume recipe in use.
    ///
    /// Called on every fresh launch, so a rerun replaces the id: the previous
    /// conversation stays on disk under its own name, and the session stops
    /// pointing at it. A record that cannot be written gives up the pin rather
    /// than the launch — an id nothing remembers would resume nothing.
    fn open_conversation(&self, command: &str, run_id: &str, session: &str) -> Option<String> {
        let id = agency_core::sessionstore::mint(command)?;
        let write =
            self.registry.lock().unwrap().set_conversation(session, run_id, &id, now_secs());
        match write {
            Ok(()) => Some(id),
            Err(e) => {
                log::warn!(
                    "{session}: couldn't record the conversation id, launching without it: {e}"
                );
                None
            }
        }
    }

    /// The conversation `session` owns, if it has one. `None` for every run
    /// that predates this and for agents Agency cannot name a conversation to;
    /// both then resume the way they always did.
    fn conversation_of(&self, session: &str) -> Option<String> {
        self.registry.lock().unwrap().get_conversation(session).ok().flatten()
    }

    /// The launch command an agent id resolves to. An extra tab may run a
    /// different agent than its run does, so the two cannot share a lookup
    /// keyed on the run alone.
    fn profile_command(&self, agent: &str) -> String {
        self.registry
            .lock()
            .unwrap()
            .get_profile(agent)
            .ok()
            .flatten()
            .map(|p| p.command)
            .unwrap_or_else(|| agent.to_string())
    }

    /// The per-session conversation stores this run's agents keep inside the
    /// workspace's transcript directory (see [`agency_core::sessionstore`]):
    /// the run's own, and one per extra agent tab.
    ///
    /// Each is named for the session that wrote it, so unlike the directory
    /// around them they are Agency's to remove even when that directory is the
    /// user's own checkout, shared with whatever pi sessions they have had
    /// there themselves.
    fn session_stores(&self, run: &agency_core::registry::Run, repo: &Path) -> Vec<PathBuf> {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Vec::new();
        };
        let worktree = workspace_dir(repo, run);
        // Bound and dropped before the loop below: `profile_command` locks the
        // registry too, and a guard still alive there would deadlock.
        let extras = {
            let reg = self.registry.lock().unwrap();
            reg.list_run_sessions(&run.id).unwrap_or_default()
        };
        std::iter::once((run.id.clone(), run.agent.clone()))
            .chain(extras.into_iter().map(|s| (s.id, s.agent)))
            .filter_map(|(sid, agent)| {
                agency_core::sessionstore::dir(
                    &home,
                    &self.profile_command(&agent),
                    &worktree,
                    &sid,
                )
            })
            .collect()
    }

    /// Where this run's agent keeps its transcript for this workspace, for the
    /// agents whose layout `usage::session_dir` knows. `None` for the rest.
    fn agent_session_dir(&self, run: &agency_core::registry::Run, repo: &Path) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        agency_core::usage::session_dir(&home, &self.agent_command(run), &workspace_dir(repo, run))
    }

    /// Every agent that has worked in this run's workspace: the run's own,
    /// plus one per extra tab, deduplicated and in tab order.
    ///
    /// The tabs are what make this a list. A run is one agent only until you
    /// open a second tab in its worktree, and that tab may be a different
    /// agent entirely — which is the whole of AGE-184's second question: each
    /// agent keeps its transcripts under a root of its own, so anything that
    /// walks "the run's transcript" as a single directory silently means "the
    /// run's *first* agent's".
    fn workspace_agents(&self, run: &agency_core::registry::Run) -> Vec<String> {
        let rows = self.registry.lock().unwrap().list_run_sessions(&run.id).unwrap_or_default();
        let mut out = vec![run.agent.clone()];
        for s in rows {
            if !out.contains(&s.agent) {
                out.push(s.agent);
            }
        }
        out
    }

    /// The transcript directories this run's agents keep for its workspace,
    /// one per agent (see [`Self::workspace_agents`]). Only the ones that
    /// exist, and only for the agents whose store layout `usage::session_dir`
    /// knows.
    fn workspace_transcript_dirs(
        &self,
        run: &agency_core::registry::Run,
        repo: &Path,
    ) -> Vec<PathBuf> {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
        let worktree = workspace_dir(repo, run);
        self.workspace_agents(run)
            .iter()
            .filter_map(|agent| {
                agency_core::usage::session_dir(&home, &self.profile_command(agent), &worktree)
            })
            .filter(|d| d.exists())
            .collect()
    }

    /// The rescued transcript directories sitting in the archive for this run:
    /// its own agent's, plus one per extra tab that ran a different agent.
    /// Read off the directory names, so it works after the session rows are
    /// gone (an archived run has none).
    fn rescued_transcript_dirs(
        &self,
        run: &agency_core::registry::Run,
        repo: &Path,
    ) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let own = agency_core::record::transcript_dir(repo, &run.id);
        if own.exists() {
            out.push(own);
        }
        if let Ok(entries) = std::fs::read_dir(agency_core::record::dir(repo)) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let is_tab = name
                    .to_str()
                    .and_then(|n| agency_core::record::transcript_dir_agent(&run.id, n))
                    .is_some();
                if is_tab {
                    out.push(entry.path());
                }
            }
        }
        out
    }

    /// Move the transcript directories for this worktree into the archive,
    /// beside the record (AGE-152). Those directories are keyed by a worktree
    /// path that is about to stop existing; left behind, they are unreachable
    /// by the agent and swept by nothing, which is how every finished run used
    /// to leak one forever.
    ///
    /// One per agent that worked here (AGE-184). Tabs running the *same* agent
    /// as the run already rode along for free — claude writes one file per
    /// conversation in the workspace's directory and pi a store per session
    /// under it, so moving the tree took every tab's conversation with it. A
    /// tab running a *different* agent did not: its directory hangs off that
    /// agent's own root, nothing named it, and archiving the run orphaned it.
    /// `agents` is read before the session rows are deleted, which is why it
    /// is passed in rather than looked up here.
    ///
    /// Worktree runs only: a run in the project's own checkout shares its
    /// session directory with the user's own sessions there, and that
    /// directory is not Agency's to move. Best-effort: a failed rescue leaves
    /// the source untouched (`move_tree_verified` deletes only after the copy
    /// verifies) and the record falls back to naming it where it is.
    fn rescue_transcript(
        &self,
        run: &agency_core::registry::Run,
        repo: &Path,
        agents: &[String],
    ) -> Option<agency_core::record::TranscriptNote> {
        if !run.worktree {
            return None;
        }
        let worktree = workspace_dir(repo, run);
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let mut sessions = 0;
        let mut own: Option<PathBuf> = None;
        for agent in agents {
            let command = self.profile_command(agent);
            let Some(src) =
                agency_core::usage::session_dir(&home, &command, &worktree).filter(|d| d.exists())
            else {
                continue;
            };
            // The run's own agent keeps the directory it has always had, so
            // records written before tabs could be rescued still resolve.
            let dst = if *agent == run.agent {
                agency_core::record::transcript_dir(repo, &run.id)
            } else {
                match agency_core::record::transcript_dir_for(repo, &run.id, agent) {
                    Some(d) => d,
                    None => continue,
                }
            };
            if let Err(e) = agency_core::transcript::move_tree_verified(&src, &dst) {
                log::warn!("archive {}: couldn't rescue {agent}'s transcript: {e}", run.id);
                continue;
            }
            sessions += count_sessions(&dst);
            if *agent == run.agent {
                own = Some(dst);
            }
        }
        // The resume line is the run's own agent's: it is the conversation the
        // record's reader will want back, and a command for a tab's agent
        // pointing at a directory that is not the run's would be a guess.
        let resume = own.as_ref().and_then(|d| resume_command(&self.agent_command(run), d));
        (sessions > 0 || own.is_some())
            .then_some(agency_core::record::TranscriptNote::Rescued { sessions, resume })
    }

    /// Put the rescued conversations back in their agents' own stores, so the
    /// restored run's resume finds them. The worktree comes back at the same
    /// path, so each store directory's name is the one it had. Best-effort
    /// both ways: with no rescued copy this is a no-op (runs archived before
    /// rescues existed still have their original directory in place), and a
    /// failed move leaves the archive copy where it is.
    ///
    /// The extra tabs' agents are read back off the directory names rather
    /// than the session rows: archiving deletes those rows, so by the time a
    /// restore runs there is nothing left to ask which agents worked here.
    fn reinstate_transcript(&self, run: &agency_core::registry::Run, repo: &Path) {
        let mut moves: Vec<(PathBuf, String)> = Vec::new();
        let own = agency_core::record::transcript_dir(repo, &run.id);
        if own.exists() {
            moves.push((own, run.agent.clone()));
        }
        if let Ok(entries) = std::fs::read_dir(agency_core::record::dir(repo)) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                let Some(agent) = agency_core::record::transcript_dir_agent(&run.id, name) else {
                    continue;
                };
                moves.push((entry.path(), agent));
            }
        }
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
        let worktree = workspace_dir(repo, run);
        for (src, agent) in moves {
            let command = self.profile_command(&agent);
            let Some(dst) = agency_core::usage::session_dir(&home, &command, &worktree) else {
                continue;
            };
            if let Err(e) = agency_core::transcript::move_tree_verified(&src, &dst) {
                log::warn!("restore {}: couldn't reinstate {agent}'s transcript: {e}", run.id);
            }
        }
    }

    /// Write the run's archive record: what it was asked, what it committed,
    /// and where that work is now.
    ///
    /// This is what makes an archive worth keeping once its branch is gone.
    /// Everything it can only guess at is left out rather than guessed: an
    /// unresolvable commit range prints as unresolvable, not as "no commits".
    fn write_run_record(
        &self,
        run: &agency_core::registry::Run,
        repo: &Path,
        facts: &agency_core::cleanup::BranchFacts,
        plan: &agency_core::cleanup::CleanupPlan,
        archived_at: i64,
        usage: Option<String>,
        rescued: Option<agency_core::record::TranscriptNote>,
    ) -> Result<()> {
        use agency_core::record::{Commit, Outcome, RunRecord, TranscriptNote};
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), repo)
            .unwrap_or_else(|_| run.base.clone());
        // The same facts the plan was decided from, passed in rather than
        // re-probed: a second read could disagree with the first, and the
        // record would then describe an ending the teardown did not produce.
        // The plan is consulted before the facts, because the record has to
        // describe what happened rather than what git says now. A branch that
        // was kept is reported as kept even if it also looks merged — which is
        // reachable, since the auto-commit above can put a WIP commit on a
        // branch that had already landed.
        let outcome = if !run.worktree {
            Outcome::NoWorkspace { branch: run.branch.clone() }
        } else if plan.keeps_branch {
            Outcome::Kept { commits: facts.commits_ahead }
        } else if facts.merged {
            Outcome::Merged { base: base.clone() }
        } else if facts.pushed {
            Outcome::Pushed
        } else if facts.commits_known && facts.commits_ahead == 0 {
            Outcome::Empty
        } else {
            Outcome::Dropped { commits: facts.commits_ahead }
        };

        // The commits this run added, named from where its branch was cut.
        // `base_commit` is recorded at creation precisely because the merge
        // base stops being the fork point the moment the branch lands; runs
        // that predate it fall back to the merge base and lose the list only if
        // they also merged.
        let range_base = run
            .base_commit
            .clone()
            .or_else(|| agency_core::merge::fork_point(repo, &run.branch, &base));
        let logged = range_base
            .as_ref()
            .filter(|_| run.worktree)
            .and_then(|from| agency_core::merge::log_commits(repo, from, &run.branch, 200).ok());
        let diffstat = range_base
            .as_ref()
            .filter(|_| run.worktree)
            .and_then(|from| agency_core::git::range_stat(repo, from, &run.branch).ok());

        let issue = run.issue_id.as_ref().and_then(|id| self.issue_label(id));
        // The rescued conversation when the rescue happened; otherwise fall
        // back to naming the agent's own directory. Named, not copied: that
        // directory belongs to the agent's CLI and Agency does not manage it.
        // Pointing at one that is not there would be worse than saying
        // nothing, so it is checked first.
        let transcript = rescued.or_else(|| {
            self.agent_session_dir(run, repo)
                .filter(|d| d.exists())
                .map(|d| TranscriptNote::Named { dir: d.display().to_string() })
        });

        let record = RunRecord {
            id: run.id.clone(),
            heading: run.title.clone().unwrap_or_else(|| run.branch.clone()),
            agent: run.agent.clone(),
            model: run.model.clone(),
            prompt: run.prompt.clone(),
            branch: run.branch.clone(),
            base,
            issue,
            created_at: run.created_at,
            archived_at,
            outcome,
            commits_known: logged.is_some(),
            commits: logged
                .unwrap_or_default()
                .into_iter()
                .map(|(short, subject)| Commit { short, subject })
                .collect(),
            diffstat,
            usage,
            transcript,
        };
        let path = agency_core::record::path(repo, &run.id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // The records directory is new to any repo that has not archived a run
        // since this shipped, and an unexcluded one turns up in every diff.
        let _ = agency_core::worktree::ensure_agency_excludes(repo);
        std::fs::write(&path, agency_core::record::render(&record))?;
        Ok(())
    }

    /// `AGE-149` for a run dispatched from an issue, so the record can link
    /// back to it the way a note does. `None` if the issue has since gone.
    fn issue_label(&self, issue_id: &str) -> Option<String> {
        let reg = self.registry.lock().unwrap();
        let issue = reg.get_issue(issue_id).ok().flatten()?;
        let key = reg
            .get_project(&issue.project_id)
            .ok()
            .flatten()
            .and_then(|p| p.issue_key)
            .unwrap_or_else(|| "ISSUE".into());
        Some(format!("{key}-{}", issue.seq))
    }

    /// The archive record for a run, as markdown. `None` when there is none:
    /// the run predates records, or its project folder has moved.
    pub fn read_run_record(&self, id: &str) -> Result<Option<String>> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let path = agency_core::record::path(&repo, &run.id);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The run's conversation, parsed from its transcript: the rescued copy
    /// beside the record when there is one, else the agent's own session
    /// directory (which covers runs archived before rescues existed).
    ///
    /// `supported: false` when the agent's transcript format is not one we
    /// read; the UI must present that as "cannot see", never as "said
    /// nothing". Worktree runs only, like the rescue: a checkout run's
    /// session directory holds every conversation the user ever had in that
    /// folder, and this call cannot tell which of them was this run's.
    pub fn read_run_conversation(&self, id: &str) -> Result<ConversationInfo> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let Some(format) = agency_core::usage::format_for(&self.agent_command(&run)) else {
            return Ok(ConversationInfo { supported: false, sessions: Vec::new() });
        };
        if !run.worktree {
            return Ok(ConversationInfo { supported: true, sessions: Vec::new() });
        }
        // The run's own agent first, then one directory per extra tab that ran
        // a different agent (AGE-184). Each is parsed in its own dialect: a
        // pi tab's file read as claude renders as nothing said, which is the
        // one thing this view must never claim.
        let mut dirs: Vec<(PathBuf, agency_core::usage::Format)> = Vec::new();
        let rescued = agency_core::record::transcript_dir(&repo, &run.id);
        if rescued.exists() {
            dirs.push((rescued, format));
        } else if let Some(d) = self.agent_session_dir(&run, &repo).filter(|d| d.exists()) {
            dirs.push((d, format));
        }
        for agent in self.workspace_agents(&run).into_iter().filter(|a| *a != run.agent) {
            let Some(tab_format) = agency_core::usage::format_for(&self.profile_command(&agent))
            else {
                continue;
            };
            let archived = agency_core::record::transcript_dir_for(&repo, &run.id, &agent)
                .filter(|d| d.exists());
            let dir = archived.or_else(|| {
                let home = std::env::var_os("HOME").map(PathBuf::from)?;
                agency_core::usage::session_dir(
                    &home,
                    &self.profile_command(&agent),
                    &workspace_dir(&repo, &run),
                )
                .filter(|d| d.exists())
            });
            if let Some(dir) = dir {
                dirs.push((dir, tab_format));
            }
        }
        // An archived run has no session rows left, so its tabs' directories
        // are found by name instead — the same list the discard sweep walks.
        for dir in self.rescued_transcript_dirs(&run, &repo) {
            let Some(name) = dir.file_name().and_then(|n| n.to_str()) else { continue };
            let Some(agent) = agency_core::record::transcript_dir_agent(&run.id, name) else {
                continue;
            };
            if dirs.iter().any(|(d, _)| *d == dir) {
                continue;
            }
            if let Some(f) = agency_core::usage::format_for(&self.profile_command(&agent)) {
                dirs.push((dir, f));
            }
        }
        let sessions = dirs
            .into_iter()
            .flat_map(|(d, f)| agency_core::transcript::read_sessions(&d, f))
            .collect();
        Ok(ConversationInfo { supported: true, sessions })
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
        self.forget_session_state(id);
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
        // Delete takes the record too: keeping a file for a run the user asked
        // to be rid of is the opposite of what they pressed. Best-effort — a
        // record that will not delete must not strand the teardown, and the
        // run's row going is what actually removes it from the app.
        if let Ok(repo) = self.project_repo(&run.project_id) {
            let path = agency_core::record::path(&repo, &run.id);
            if let Err(e) = remove_if_present(&path) {
                log::warn!("discard_run {id}: couldn't remove the run record: {e}");
            }
            // The transcripts go with the record: the rescued copy beside it,
            // and the agent's own directory for this worktree, which is keyed
            // by a path that stops existing and which nothing else ever
            // sweeps (AGE-152 counted one leaked per discarded run, forever).
            // Worktree runs only — a checkout run's session directory also
            // holds the user's own sessions in that folder.
            //
            // One per agent that worked here, not one per run (AGE-184): an
            // extra tab may run a different agent, whose directory hangs off
            // its own root and which nothing else names.
            for dir in self.rescued_transcript_dirs(&run, &repo) {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    log::warn!("discard_run {id}: couldn't remove the rescued transcript: {e}");
                }
            }
            if run.kind == "agent" && run.worktree {
                for sdir in self.workspace_transcript_dirs(&run, &repo) {
                    if let Err(e) = std::fs::remove_dir_all(&sdir) {
                        log::warn!("discard_run {id}: couldn't remove the transcript dir: {e}");
                    }
                }
            } else if run.kind == "agent" {
                // A checkout run's directory is the user's, but the per-session
                // stores inside it are this run's alone (AGE-175) and nothing
                // else ever sweeps them.
                for store in self.session_stores(&run, &repo) {
                    if store.exists() {
                        if let Err(e) = std::fs::remove_dir_all(&store) {
                            log::warn!(
                                "discard_run {id}: couldn't remove {}: {e}",
                                store.display()
                            );
                        }
                    }
                }
            }
        }
        {
            let reg = self.registry.lock().unwrap();
            reg.delete_run_sessions(id)?;
            // The conversations go with the run, like the transcripts above.
            // Archiving deliberately keeps them: a restored run's transcripts
            // are reinstated under the same names, so it comes back into the
            // conversation it was in.
            reg.delete_conversations(id)?;
            reg.delete_run(id)?;
        }
        if let Some(issue_id) = &run.issue_id {
            self.maybe_rollback_issue(issue_id);
        }
        Ok(())
    }

    /// Archive a run: stop its sessions, run the optional archive cleanup
    /// script, write the run's record, remove the worktree, and stamp
    /// `archived_at`.
    ///
    /// Whether the branch survives is [`agency_core::cleanup`]'s decision, not
    /// this function's: a branch already contained in the base or on a remote
    /// is a second copy of commits that exist elsewhere, and keeping one per
    /// archived run is how a repo ends up with hundreds of `agent/*` refs
    /// nobody can tell apart. A branch carrying work nothing else has is kept,
    /// and only then is the run restorable.
    ///
    /// The record is what makes deleting the branch bearable: it is a markdown
    /// file in the project's `.agency/records/`, written before anything is
    /// removed, saying what the agent was asked and what became of the work.
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
        // Read while the session rows are still there: the rescue below needs
        // one transcript directory per agent that worked in this workspace,
        // and the tabs that name the other agents are about to be deleted.
        let agents = self.workspace_agents(&run);
        // Stop all sessions and drop attach handles. Extra tabs are purged for
        // good: the worktree they live in is about to disappear.
        self.attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.kill_run_sessions(id);
        self.shell_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&shell_session_name(id));
        self.kill_extra_sessions(id);
        {
            let reg = self.registry.lock().unwrap();
            reg.delete_run_sessions(id)?;
            // Every tab goes with the worktree, so a restored run has only its
            // own agent to come back as; leaving it stamped closed (AGE-184)
            // would restore a run with an empty tab strip.
            reg.set_run_primary_closed(id, None)?;
        }

        // Decided here, after the auto-commit above: a WIP commit made seconds
        // ago is on no remote and in no base, so a run that looked merged
        // before it correctly comes out of this holding work.
        let facts = self.branch_facts(&run);
        let plan = agency_core::cleanup::plan(&facts, Disposal::Archive);
        let archived_at = now_secs();
        // The spend is read before the transcript moves: the 2s usage poll
        // zeroes a run's in-memory total the moment its directory stops
        // existing, and a tick can land inside this function. Read after the
        // rescue, the record's cost line would vanish exactly when the
        // transcript it was parsed from was being saved.
        let usage =
            self.usage.lock().unwrap().get(&run.id).and_then(|(_, u)| {
                agency_core::usage::label(&agency_core::usage::UsageInfo::from(u))
            });
        // After the session kills above, so the files are quiescent; before
        // the worktree goes, so a crash mid-archive leaves the directory
        // where it always was rather than half-moved.
        if run.worktree {
            step(on_progress, "Saving the conversation", &run.branch);
        }
        let rescued = self.rescue_transcript(&run, &repo, &agents);
        // Written while the worktree, the branch and the commit range all still
        // exist — after this the range that names the run's own commits may be
        // gone. Best-effort: a record that cannot be written is not worth
        // failing an archive over, and it says so in the log.
        if let Err(e) =
            self.write_run_record(&run, &repo, &facts, &plan, archived_at, usage, rescued)
        {
            log::warn!("archive_run {id}: couldn't write the run record: {e}");
        }

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
            let manager = WorktreeManager::new(repo);
            if plan.deletes_branch {
                manager.remove(id)?;
            } else {
                manager.remove_keep_branch(id)?;
            }
        }
        step(on_progress, "Cleaning up", &run.branch);
        self.registry.lock().unwrap().set_archived(id, Some(archived_at))?;
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
            // Archiving deletes a branch whose commits are already on the base
            // or a remote, so most archived runs have no branch of their own
            // left. This used to stop here — "there is no branch to restore
            // this agent onto" — which meant Restore failed for every run that
            // ended the normal way, and only worked for the abandoned ones.
            // The work is on the base and the conversation was rescued into the
            // archive, so cut the branch again from the base and reinstate the
            // conversation into the worktree that comes back at the same path:
            // the run resumes where it left off, on top of what it merged.
            if agency_core::merge::branch_exists(&repo, &run.branch) {
                manager.restore(id)?;
            } else {
                let Some(start) = restore_start_point(&repo, &run) else {
                    bail!(
                        "'{}' is gone, and so is the branch it was based on, so there is nothing \
                         left to cut a worktree from. Its record is still in the archive.",
                        run.branch
                    );
                };
                manager.recreate_on(id, &run.branch, &start)?;
            }
            if let Err(e) = manager.copy_essentials(id, &config.files.copy) {
                log::warn!("copying essentials into restored worktree {id}: {e}");
            }
        }
        // Reallocate the port block if another active run claimed it while this
        // one was archived (list_port_bases excludes archived rows, so a live
        // collision means a real conflict) — otherwise both export the same
        // AGENCY_PORT into their scripts.
        let mut port = run.port_base;
        if let Some(existing) = run.port_base {
            let taken: std::collections::HashSet<u16> =
                self.registry.lock().unwrap().list_port_bases()?.into_iter().collect();
            if taken.contains(&existing) {
                let fresh = self.allocate_port(config.ports.base, config.ports.block_size)?;
                self.registry.lock().unwrap().set_port_base(id, Some(fresh))?;
                port = Some(fresh);
            }
        }
        if run.worktree {
            let worktree = workspace_dir(&repo, &run);
            self.emit_mcp(
                &run.agent,
                &repo,
                &worktree,
                &config,
                preview_mcp_port_for(&config, port),
            );
            // `restore` cuts the worktree again from the kept branch, so the
            // generated files are gone with the old one; both come back here.
            self.emit_skills(
                &run.agent,
                &repo,
                &worktree,
                &run.branch,
                &self.issue_key_for(&run.project_id),
                &config,
                run.loop_config.as_ref(),
                port,
            );
        }
        // The conversation the archive rescued goes back into the agent's own
        // store: the worktree exists again at the same path, so the agent's
        // resume finds its sessions exactly as if the run had never been
        // archived.
        if run.worktree {
            self.reinstate_transcript(&run, &repo);
        }
        // The record describes a run that ended; this one is live again, and a
        // record left behind would be read as the account of a run still going.
        // It is written afresh whenever this one is archived again.
        if let Err(e) = remove_if_present(&agency_core::record::path(&repo, &run.id)) {
            log::warn!("restore_run {id}: couldn't remove the stale run record: {e}");
        }
        self.registry.lock().unwrap().set_archived(id, None)?;
        let refreshed = self.run_record(id)?;
        Ok(self.run_info(&refreshed))
    }

    /// The project's archived runs, each carrying what its archive still holds.
    ///
    /// Asked of git and the filesystem rather than remembered, so a branch
    /// deleted by hand outside Agency stops being offered for restore instead
    /// of failing when the restore is pressed.
    pub fn list_archived_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_archived_runs(project_id)?;
        let live = self.term.read().unwrap().list().unwrap_or_default();
        let repo = self.project_repo(project_id).ok();
        Ok(runs
            .iter()
            .map(|r| {
                let mut info = self.run_info_from(r, &live);
                let branch_kept = repo
                    .as_ref()
                    .is_some_and(|repo| agency_core::merge::branch_exists(repo, &r.branch));
                info.archived = Some(ArchivedInfo {
                    branch_kept,
                    // Only asked when it decides something: with the branch
                    // still here the restore uses it, and this is a second git
                    // call per row in a list that is drawn on every open.
                    restore_base: match (branch_kept, repo.as_ref()) {
                        (false, Some(repo)) => restore_start_point(repo, r),
                        _ => None,
                    },
                    has_record: repo
                        .as_ref()
                        .is_some_and(|repo| agency_core::record::path(repo, &r.id).exists()),
                    // Worktree runs only, like the rescue: a checkout run's
                    // session directory is every conversation the user ever
                    // had in that folder, not this run's.
                    has_conversation: r.worktree
                        && repo.as_ref().is_some_and(|repo| {
                            agency_core::usage::format_for(&self.agent_command(r)).is_some()
                                && (agency_core::record::transcript_dir(repo, &r.id).exists()
                                    || self.agent_session_dir(r, repo).is_some_and(|d| d.exists()))
                        }),
                });
                info
            })
            .collect())
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
            preview_port: self.preview.lock().unwrap().get(target).map(|s| s.port()),
            preview_tools_enabled: config.preview.agent_tools,
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
        // A first web script starts the preview servers here and now, so the
        // config the Run tab reloads right after this call already carries
        // `preview_port` instead of waiting out the sweep's next tick.
        self.sync_preview_servers();
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
            let profile = with_model(&profile, model);
            // Same GUI port as the run would use: start_run_session refuses a
            // second web-GUI session in one workspace, so it can't be taken.
            with_web_ui(&profile, gui_port_for(&config, run.port_base, agent), &self.data_dir)
        };
        // The extra tab may run a different agent than the one the worktree
        // was created for; make sure MCP config exists in its native format.
        // Skipped without a worktree, for the same reason as at creation: the
        // target file would be one in the user's own checkout.
        if run.worktree {
            self.emit_mcp(
                agent,
                &repo,
                &worktree,
                &config,
                preview_mcp_port_for(&config, run.port_base),
            );
            // Likewise the skills kit: the tab's agent may have a skills
            // convention the worktree's agent doesn't, so the kit it reads may
            // not be there yet.
            self.emit_skills(
                agent,
                &repo,
                &worktree,
                &run.branch,
                &self.issue_key_for(&run.project_id),
                &config,
                run.loop_config.as_ref(),
                run.port_base,
            );
        }
        // Same env recipe as the run itself, ports included: extra sessions are
        // collaborators in the same workspace, not new workspaces. The
        // conversation is the exception, and is the tab's own: sharing the
        // worktree is exactly how a tab used to take over the run's resume
        // (AGE-175).
        let env = self.agent_env(&profile, &worktree, &repo, &run.id, sid, run.port_base)?;
        let conversation = self.open_conversation(&profile.command, &run.id, sid);
        let (command, args) = fresh_agent_argv(
            &profile,
            &worktree,
            prompt,
            config.scripts.setup.as_deref(),
            conversation.as_deref(),
        );
        self.term.read().unwrap().start_session(
            &session_name(sid),
            &worktree,
            &command,
            &args,
            &env,
            220,
            50,
        )?;
        self.kick_web_ui(
            sid,
            agent,
            gui_port_for(&config, run.port_base, agent),
            &worktree,
            prompt,
        );
        Ok(())
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
        // One web-GUI session per workspace: a run has one GUI port, so a
        // second server in the same worktree would lose the bind and die on
        // "address in use" the moment it started. Refuse with the reason
        // instead of spawning a tab whose whole life is that error. The first
        // one is allowed on any run — `RunInfo::gui_session_id` names whichever
        // session it is, so a web agent opened as a tab gets its GUI pane too.
        if crate::agent_catalog::web_ui(&agent).is_some() {
            let taken = crate::agent_catalog::web_ui(&run.agent).is_some()
                || self
                    .registry
                    .lock()
                    .unwrap()
                    .list_run_sessions(run_id)?
                    .iter()
                    .any(|s| crate::agent_catalog::web_ui(&s.agent).is_some());
            if taken {
                bail!(
                    "{agent} serves its GUI on this workspace's port, which another session \
                     here already uses — start it as its own agent instead"
                );
            }
        }
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

    /// Close one of a run's agent tabs: kill its daemon session and forget it.
    /// The worktree, branch and every sibling session are untouched.
    ///
    /// The run's own tab counts (AGE-184). Closing it cannot delete a row —
    /// the run *is* that row — so it stamps `primary_closed_at` instead: the
    /// strip stops drawing the tab, `ensure_run_active` stops reviving it, and
    /// `lead_session_name` hands the run's status and its prompts to whichever
    /// tab is left. Archiving clears the stamp, because archiving takes every
    /// extra tab with it and the run's own agent is then all there is.
    ///
    /// Refused when it would leave the run with no agent at all: a workspace
    /// with an empty tab strip is a run you can only stare at, and "there is
    /// one left" is exactly when archive or delete is the thing meant.
    pub fn close_run_session(&self, id: &str) -> Result<()> {
        let (run_id, seq) = split_session_id(id);
        let run = self.run_record(run_id)?;
        let live = self.term.read().unwrap().list().unwrap_or_default();
        if self.drawn_tabs(&run, &live).into_iter().filter(|t| t != id).count() == 0 {
            bail!(
                "this is the last agent in this workspace. Archive or delete the agent instead, \
                 or open another tab first."
            );
        }
        if seq.is_none() {
            // A loop drives the run's own session: attempt after attempt is
            // spawned under that name, so closing it would only have it come
            // back. Stopping the loop is the gesture that means this.
            if run.loop_config.is_some() {
                bail!("this agent is running a loop. Stop the loop before closing its tab.");
            }
            self.attaches.lock().unwrap().remove(id);
            self.forget_session_state(id);
            let _ = self.term.read().unwrap().kill(&session_name(id));
            self.registry.lock().unwrap().set_run_primary_closed(id, Some(now_secs()))?;
            return Ok(());
        }
        self.attaches.lock().unwrap().remove(id);
        self.forget_session_state(id);
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.registry.lock().unwrap().delete_run_session(id)?;
        Ok(())
    }

    /// Reopen the run's own agent tab after it was closed (AGE-184).
    ///
    /// Through `ensure_run_active`, so the agent comes back into the
    /// conversation it was in rather than starting blank: closing a tab is not
    /// throwing the conversation away, and the transcript was never deleted.
    /// A run whose tab is already open is left alone.
    pub fn reopen_primary_session(&self, id: &str) -> Result<()> {
        let run = self.run_record(id)?;
        if run.primary_closed_at.is_none() {
            return Ok(());
        }
        self.registry.lock().unwrap().set_run_primary_closed(id, None)?;
        self.ensure_run_active(id)
    }

    /// The session ids the run's tab strip actually draws: its own agent
    /// unless that tab has been closed, plus every extra tab whose session is
    /// still alive. A tab whose agent has gone is dropped from the strip, so
    /// it cannot be what stops a close from leaving the strip empty.
    fn drawn_tabs(
        &self,
        run: &agency_core::registry::Run,
        live: &[(String, SessionStatus)],
    ) -> Vec<String> {
        let mut out = Vec::new();
        if run.primary_closed_at.is_none() {
            out.push(run.id.clone());
        }
        let rows = self.registry.lock().unwrap().list_run_sessions(&run.id).unwrap_or_default();
        for s in rows {
            let name = session_name(&s.id);
            if live.iter().any(|(n, _)| *n == name) {
                out.push(s.id);
            }
        }
        out
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
            self.forget_session_state(&s.id);
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
        // The user closed this tab (AGE-184). Every other caller of this
        // function treats a missing session as an accident to repair — the app
        // was quit, the daemon dropped it — so without this the run's own agent
        // would come straight back on the next poll that touched it, and the
        // close would look like it had not worked.
        if run.primary_closed_at.is_some() {
            return Ok(());
        }
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
            // The focus-respawn must not outrun the driver's cap check: a loop
            // already over its wall-clock or token cap spawned here would run a
            // whole attempt past the cap before the driver could stall it.
            // Leave the session down instead; the next tick stalls the loop
            // with its reason and the loop strip says why the pane is dead.
            let over_cap = fresh
                .loop_config
                .as_ref()
                .zip(fresh.loop_state.as_ref())
                .and_then(|(c, s)| crate::looper::exceeded_cap(c, s, now_secs()))
                .is_some();
            if awaiting && !over_cap && matches!(self.run_status(id)?, SessionStatus::Gone) {
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
            let profile = with_model(&profile, run.model.as_deref());
            with_web_ui(&profile, gui_port_for(&config, run.port_base, &run.agent), &self.data_dir)
        };
        let env = self.agent_env(&profile, &worktree, &repo, &run.id, id, run.port_base)?;
        let setup = config.scripts.setup.as_deref();
        // The conversation this session owns, if it has one to come back to.
        let recorded = self.conversation_of(id);
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
                    id,
                    recorded.as_deref(),
                )
            })
            .unwrap_or(crate::resume_probe::ResumeProbe::Unknown);
        let use_resume =
            profile.resume_args.is_some() && probe != crate::resume_probe::ResumeProbe::None;
        // Resuming reopens the conversation on record; starting fresh opens a
        // new one under a new name. Never the recorded name on a fresh launch:
        // claude refuses `--session-id` for an id already in use, and the run
        // would not come up at all.
        let conversation = if use_resume {
            recorded
        } else {
            self.open_conversation(&profile.command, &run.id, id)
        };
        let (command, args) = agent_argv(
            &profile,
            &worktree,
            &run.prompt,
            use_resume,
            setup,
            conversation.as_deref(),
        );
        // The fallback carries the run's prompt, hours old though it may be by
        // now: there is nothing else it could open with, and the task it names
        // is this run's whether the agent is starting it or restarting it. A
        // promptless fresh session would leave the user staring at a bare CLI
        // in a worktree with no idea what it was for (AGE-137).
        //
        // It opens no named conversation. Whether it ran at all is the daemon's
        // to know, not ours: recording a name for a conversation that may never
        // be created would point the next resume at nothing and abandon the one
        // the agent is in. Left unnamed, it is at worst not resumable by name —
        // and the resume it stands in for is the one the probe above has
        // already proved.
        let fallback = if use_resume {
            let (fresh_cmd, fresh_args) =
                fresh_agent_argv(&profile, &worktree, &run.prompt, setup, None);
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
        // Resume, not a fresh dispatch: adopt the folder so the GUI is not an
        // empty picker, but do not re-queue the opening prompt the existing
        // dsh session already has.
        self.kick_web_ui(
            id,
            &run.agent,
            gui_port_for(&config, run.port_base, &run.agent),
            &worktree,
            "",
        );
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
        // Rerunning is how a closed first tab comes back (AGE-184): it is the
        // one gesture that already means "start this run's own agent again",
        // and it is spelled out in the close confirmation.
        if run.primary_closed_at.is_some() {
            self.registry.lock().unwrap().set_run_primary_closed(id, None)?;
        }
        let repo = self.project_repo(&run.project_id)?;
        let config = agency_core::config::load(&repo);
        let worktree = workspace_dir(&repo, &run);
        let profile = {
            let reg = self.registry.lock().unwrap();
            let profile = reg
                .get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?;
            let profile = with_model(&profile, run.model.as_deref());
            with_web_ui(&profile, gui_port_for(&config, run.port_base, &run.agent), &self.data_dir)
        };
        let env = self.agent_env(&profile, &worktree, &repo, &run.id, id, run.port_base)?;
        // A rerun is a fresh launch by definition, so it takes the fresh argv
        // (prompt and all, never the resume recipe) that `create_run` took, and
        // a new conversation with it. The one it replaces stays on disk under
        // its own name; the run simply stops pointing at it.
        let conversation = self.open_conversation(&profile.command, &run.id, id);
        let (command, args) = fresh_agent_argv(
            &profile,
            &worktree,
            &run.prompt,
            config.scripts.setup.as_deref(),
            conversation.as_deref(),
        );
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
        self.kick_web_ui(
            id,
            &run.agent,
            gui_port_for(&config, run.port_base, &run.agent),
            &worktree,
            &run.prompt,
        );
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
        // Re-emit into the existing worktree on every attempt. A headless
        // attempt is a fresh agent process with no memory of the last one, and
        // the previous attempt may have edited or deleted the kit; refreshing
        // it after the wip commit above starts every attempt from the same
        // facts, the check command included. (MCP config is deliberately not
        // re-emitted here: it is written once at creation and the agent never
        // changes for a loop.)
        if run.worktree {
            self.emit_skills(
                &run.agent,
                &repo,
                &worktree,
                &run.branch,
                &self.issue_key_for(&run.project_id),
                &config,
                run.loop_config.as_ref(),
                run.port_base,
            );
        }
        let env = self.agent_env(&profile, &worktree, &repo, &run.id, &run.id, run.port_base)?;
        // Every attempt is a fresh session, so every attempt names its own
        // conversation. The last one's is what the run resumes into if the
        // user takes the loop's work over by hand once it ends.
        let conversation = self.open_conversation(&profile.command, &run.id, &run.id);
        let (command, args) = loop_argv(
            &profile,
            &worktree,
            &run.prompt,
            config.scripts.setup.as_deref(),
            conversation.as_deref(),
        )?;
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
                // Total tokens the run has burned, from the transcript reader
                // (refresh_usage keeps it current on the notifier tick). None
                // for agents whose transcript format we cannot read, so the
                // token cap never trips on a count we cannot see.
                let tokens = self.usage.lock().unwrap().get(&run.id).map(|(_, u)| u.tokens.total());
                let snap = crate::looper::LoopSnapshot { agent, check_in_flight, check, tokens };
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
                                    reason: None,
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
                            reason: None,
                        }),
                        crate::looper::LoopAction::NotifyStalled => notices.push(LoopNotice {
                            project_id: proj.id.clone(),
                            run_id: run.id.clone(),
                            label: label.clone(),
                            done: false,
                            attempt: next.attempt,
                            reason: next.stall_reason,
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

    /// Push `token`'s branch, registering a cancel token against its worktree so
    /// [`AppState::cancel_push`] can stop it. Ungated, like every read of the
    /// repo: a push writes refs on the remote, not the checkout.
    pub fn push_run(
        &self,
        token: &str,
        on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<()> {
        let wt = self.git_root(token)?;
        self.with_push_cancel(&wt, |cancel| {
            agency_core::git::push_with_progress(&wt, cancel, on_progress)
        })
    }

    /// [`AppState::push_run`] with `--force-with-lease`, cancellable over the
    /// same key: a force push after a rebase re-uploads the whole branch, so it
    /// is the same long upload and the panel offers the same Cancel. Ungated for
    /// the same reason as `push_run`.
    pub fn push_force_run(
        &self,
        token: &str,
        on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<()> {
        let wt = self.git_root(token)?;
        self.with_push_cancel(&wt, |cancel| {
            agency_core::git::push_force_with_progress(&wt, cancel, on_progress)
        })
    }

    /// [`AppState::push_run`]'s both-directions sibling (VS Code's "Sync
    /// Changes"), cancellable over the same key: its push half is the slow one
    /// and the panel's Cancel can't tell the two actions apart. Gated, unlike
    /// push, because the pull half rewrites the working tree.
    pub fn sync_run(
        &self,
        token: &str,
        on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<agency_core::git::SyncOutcome> {
        self.git_mutate(token, |wt| {
            self.with_push_cancel(wt, |cancel| agency_core::git::sync(wt, cancel, on_progress))
        })
    }

    /// Stop the push running against `token`'s worktree, if there is one, by
    /// killing the git it is waiting on. A no-op otherwise — the panel's Cancel
    /// is live during steps with nothing to kill too (the rebase before a
    /// rebase-and-sync, the fetch half of a sync).
    ///
    /// Takes the run token the push was started with rather than a path, for the
    /// same reason as [`AppState::cancel_clone`]: the frontend never reproduces
    /// the rule for finding a run's worktree, where a mismatch of one character
    /// would silently cancel nothing.
    pub fn cancel_push(&self, token: &str) {
        let Ok(worktree) = self.git_root(token) else { return };
        if let Some(cancel) = self.push_cancels.lock().unwrap().get(&worktree) {
            cancel.cancel();
        }
    }

    /// Run `f` with a cancel token registered for `worktree`, so `cancel_push`
    /// can reach the git it spawns.
    ///
    /// One token per worktree: two pushes of the same checkout at once would
    /// leave the second unstoppable (the first's exit unregisters the pair).
    /// Nothing in the UI offers that, and concurrent pushes of one checkout race
    /// in git anyway, so it isn't worth a registry of tokens per path.
    fn with_push_cancel<T>(
        &self,
        worktree: &Path,
        f: impl FnOnce(&agency_core::setup::CancelToken) -> T,
    ) -> T {
        let cancel = agency_core::setup::CancelToken::new();
        self.push_cancels.lock().unwrap().insert(worktree.to_path_buf(), cancel.clone());
        let out = f(&cancel);
        self.push_cancels.lock().unwrap().remove(worktree);
        out
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

    /// Rename a run's branch, in git and in the registry together, while the
    /// branch is still local.
    ///
    /// A merge writes the branch name into the base branch's history for good
    /// (`Merge branch 'agent/…'`), and a merged PR's head branch cannot be
    /// renamed afterwards either, so this is the last moment a name derived
    /// from the prompt can be fixed. Observed on AGE-139, whose issue body led
    /// with a link and so put a URL fragment in the branch name: the rename had
    /// to be done by hand in both places, because `git branch -m` alone leaves
    /// the registry pointing at a branch that no longer exists and the next
    /// merge targets that.
    ///
    /// Git moves first and is put back if the row will not take the new name,
    /// so the two cannot end up disagreeing. Returns the name actually applied,
    /// which carries the `agent/` prefix whether or not the caller typed it.
    pub fn rename_run_branch(&self, id: &str, requested: &str) -> Result<String> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        // Not ours to rename: this run works on whatever branch the user's own
        // checkout is on, shared with everything else they do there.
        if !run.worktree {
            bail!(
                "this agent works directly in the project checkout, on {}, which is the \
                 user's own branch rather than one Agency cut for the run; rename it in \
                 Source Control or in git if you want it renamed",
                run.branch
            );
        }
        require_branch_exists(&run, &repo)?;
        let new = agency_core::branchname::normalize(requested)?;
        if new == run.branch {
            return Ok(new);
        }
        // The same gate `create_run_spec` takes: this moves a branch, and that
        // one cuts branches.
        let _gate = self.worktree_gate.lock().unwrap();
        if agency_core::merge::branch_exists(&repo, &new) {
            bail!("this project already has a branch called {new}");
        }
        // A rename mid-merge would move the branch out from under the merge the
        // shared checkout is in the middle of, and `owns_merge` (which matches
        // MERGE_HEAD against the branch tip) would stop recognizing it as this
        // run's, stranding the resolution.
        if agency_core::merge::owns_merge(&repo, &run.branch) {
            bail!(
                "this branch is being merged right now; finish or abort that merge before \
                 renaming it"
            );
        }
        // A published name is not something a local rename takes back: the
        // remote keeps the old branch, and a PR opened from it goes on pointing
        // at that name whatever this end is called.
        if let Some(remote) = agency_core::merge::remote_copies(&repo, &run.branch).first() {
            bail!(
                "this branch is already published as {remote}, so renaming it here would \
                 leave that copy behind, along with any PR opened from it; delete the remote \
                 branch first if the published name is the problem"
            );
        }
        agency_core::merge::rename_branch(&repo, &run.branch, &new)?;
        let recorded = self.registry.lock().unwrap().set_run_branch(id, &new);
        if let Err(e) = recorded {
            // Put git back rather than leave the two disagreeing: the registry
            // is what merge, PR and teardown all read, so a half-done rename
            // would send every one of them at a branch that is not there.
            if let Err(back) = agency_core::merge::rename_branch(&repo, &new, &run.branch) {
                log::error!(
                    "renaming {new} back to {} after the registry refused the rename: {back}",
                    run.branch
                );
            }
            return Err(e.context("recording the new branch name"));
        }
        // The workspace skill states the branch the run owns, so an unrefreshed
        // copy would have the agent name a branch that no longer exists. An
        // archived run has no worktree to write into; emitting would recreate
        // the directory git worktree removed.
        let worktree = workspace_dir(&repo, &run);
        if worktree.exists() {
            let config = agency_core::config::load(&repo);
            self.emit_skills(
                &run.agent,
                &repo,
                &worktree,
                &new,
                &self.issue_key_for(&run.project_id),
                &config,
                run.loop_config.as_ref(),
                run.port_base,
            );
        }
        Ok(new)
    }

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

    /// Where a PR's merge conflicts actually are.
    ///
    /// GitHub reports *that* a PR conflicts and never which files, so this
    /// answers it locally: fetch both sides' remote-tracking refs and merge
    /// them in memory. Nothing is checked out, so it is safe to call from the
    /// review pane while the user is working in the same repo.
    pub fn pr_conflicts(&self, project_id: &str, number: u64) -> Result<PrConflicts> {
        let repo = self.project_repo(project_id)?;
        let pr = agency_core::gh::GhCli::default()
            .view_pr_detail(&repo, number)?
            .ok_or_else(|| anyhow!("PR #{number} not found"))?;
        if pr.head_ref_name.is_empty() || pr.base_ref_name.is_empty() {
            bail!("PR #{number} has no local head branch (cross-fork PRs aren't supported yet)");
        }
        let mut out = PrConflicts {
            base: pr.base_ref_name.clone(),
            head: pr.head_ref_name.clone(),
            files: Vec::new(),
            probed: false,
        };
        // Both halves are best effort: a repo behind a proxy, an old git or a
        // branch pushed from elsewhere all leave the UI reporting the conflict
        // without the file list, which is still the message the user needs.
        if agency_core::git::fetch_tracking(&repo, &[&pr.base_ref_name, &pr.head_ref_name]).is_err()
        {
            return Ok(out);
        }
        let head = format!("origin/{}", pr.head_ref_name);
        let base = format!("origin/{}", pr.base_ref_name);
        // Head first: the fix merges base *into* the PR's branch, so this is
        // the same merge the agent (or the user) is about to run.
        if let Ok(files) = agency_core::merge::conflicting_paths(&repo, &head, &base) {
            out.files = files;
            out.probed = true;
        }
        Ok(out)
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

    /// Rewrite the PR's title and/or description from the review pane. `None`
    /// leaves that field as it is, so a title-only edit doesn't rewrite a
    /// description the caller never looked at.
    pub fn edit_pr(
        &self,
        project_id: &str,
        number: u64,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().edit_pr(&repo, number, title, body)
    }

    /// Rewrite one of the viewer's own review comments. `comment_id` is a
    /// comment's `database_id` from `pr_review_threads`.
    pub fn edit_pr_comment(&self, project_id: &str, comment_id: u64, body: &str) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().update_review_comment(&repo, comment_id, body)
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

    /// Which of a run's agents a prompt Agency composes should be typed into.
    ///
    /// `session` is the caller's choice — an agent tab the user picked
    /// (AGE-184: a worktree with three agents in it sent every prompt to the
    /// first one, whose context was usually the least relevant). `None` means
    /// "whichever agent speaks for this run": its own, or the tab standing in
    /// for it once that has been closed.
    ///
    /// A named session is checked against the run rather than trusted: it
    /// arrives from the UI, and typing a merge-conflict prompt into another
    /// run's agent would send it off editing work it has never seen.
    fn send_target(&self, run_id: &str, session: Option<&str>) -> Result<String> {
        let Some(sid) = session else {
            // A row we cannot read is not an error here: the caller's next step
            // is a session check, and "that agent is not running" says more
            // about a run that has gone than "unknown run" does.
            let Ok(run) = self.run_record(run_id) else { return Ok(run_id.to_string()) };
            let live = self.term.read().unwrap().list().unwrap_or_default();
            // The daemon's name, back to a session id.
            let lead = lead_session_name(&run, &live);
            return Ok(lead.strip_prefix("agency-").unwrap_or(&lead).to_string());
        };
        if split_session_id(sid).0 != run_id {
            bail!("session {sid} does not belong to this agent");
        }
        if sid != run_id && self.registry.lock().unwrap().get_run_session(sid)?.is_none() {
            bail!("agent tab {sid} is not open any more");
        }
        Ok(sid.to_string())
    }

    /// Hand the PR's failing checks to the agent's live session so it can
    /// investigate — same delivery path as review comments. Returns whether the
    /// text went out now or is queued behind the agent's current turn.
    pub fn send_check_feedback(&self, id: &str) -> Result<bool> {
        let status = self.pr_status(id)?;
        let failing: Vec<_> =
            status.checks.iter().filter(|c| c.bucket == "fail" || c.bucket == "cancel").collect();
        if failing.is_empty() {
            bail!("no failing checks to send");
        }
        let target = self.send_target(id, None)?;
        if !matches!(
            self.term.read().unwrap().status(&session_name(&target)),
            Ok(SessionStatus::Running)
        ) {
            bail!("agent session {target} is not running");
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
        self.queue_send(&target, "check feedback", msg)
    }

    /// Hand a conflicted merge to one of the run's agents by typing a prompt
    /// into its live session, the same delivery path as review comments and CI
    /// feedback. Replaces the old one-shot resolver process, which spawned a
    /// second, context-free agent that had no idea what the branch was for.
    ///
    /// `session` names the agent tab to hand it to; `None` means the one that
    /// speaks for the run. A worktree can host several agents, and this used to
    /// go to the run's own every time — which after a day's work is often the
    /// one with the least to do with the branch being merged (AGE-184).
    pub fn send_merge_conflict(&self, id: &str, session: Option<&str>) -> anyhow::Result<bool> {
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
        let target = self.send_target(id, session)?;
        if !matches!(
            self.term.read().unwrap().status(&session_name(&target)),
            Ok(SessionStatus::Running)
        ) {
            bail!("agent session {target} is not running");
        }
        let msg = compose_merge_conflict(&repo, &run.branch, &base, &conflict_status(&repo));
        self.queue_send(&target, "merge conflict", msg)
    }

    /// Update focus/active-run state. On an unfocused→focused edge, hand back a
    /// pending notification target the return itself answers for (consuming
    /// it), so the caller can deep-link the UI to the run the user came back
    /// for. A notification posted while Agency was in front is left where it
    /// is: the user is already here, and only a click on it means anything
    /// (AGE-166).
    pub fn set_ui_state(
        &self,
        focused: bool,
        active_run: Option<String>,
    ) -> Option<(String, String)> {
        let mut ui = self.ui.lock().unwrap();
        let was_focused = ui.focused;
        ui.focused = focused;
        ui.active_run = active_run;
        if !(focused && !was_focused) {
            return None;
        }
        take_pending_open(&mut ui, crate::notifier::OpenTrigger::Focus)
    }

    /// The run to open because the user clicked its notification.
    pub fn take_notification_target(&self) -> Option<(String, String)> {
        let mut ui = self.ui.lock().unwrap();
        take_pending_open(&mut ui, crate::notifier::OpenTrigger::Click)
    }

    /// Record the run a just-shown notification is about (see [`PendingOpen`]).
    pub fn note_notification(&self, project_id: &str, run_id: &str, from_background: bool) {
        let mut ui = self.ui.lock().unwrap();
        ui.pending_open = Some(PendingOpen {
            project_id: project_id.to_string(),
            run_id: run_id.to_string(),
            from_background,
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

    /// Hand the unsent comments to the agent's live session and mark them sent.
    /// Returns whether they went out now or are queued behind the agent's turn.
    pub fn send_review_comments(&self, run_id: &str) -> Result<bool> {
        let unsent = self.registry.lock().unwrap().list_unsent_review_comments(run_id)?;
        if unsent.is_empty() {
            bail!("no unsent review comments");
        }
        let target = self.send_target(run_id, None)?;
        if !matches!(
            self.term.read().unwrap().status(&session_name(&target)),
            Ok(SessionStatus::Running)
        ) {
            bail!("agent session {target} is not running");
        }
        let delivered = self.queue_send(&target, "review comments", compose_feedback(&unsent))?;
        // Marked sent once the queue has accepted them, not once they are
        // written: from here the message is Agency's to deliver, and leaving the
        // button live would only invite a second copy of the same comments.
        self.registry.lock().unwrap().mark_review_comments_sent(run_id)?;
        Ok(delivered)
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
        // A graph build is not a daemon session, so it used to outlive the
        // quit: graphify kept running with the agent CLI it spawns per document
        // still billing, and no window left to say so or to stop it.
        self.stop_all_knowledge_builds();
        let sessions = self.term.read().unwrap().list();
        if let Ok(sessions) = sessions {
            for (id, _) in sessions {
                let _ = self.term.read().unwrap().kill(&id);
            }
        }
        let _ = self.term.read().unwrap().shutdown();
    }

    /// Signal every running graph build to stop, on the way out. Signal only:
    /// the monitor threads that would reap them do not outlive this process.
    fn stop_all_knowledge_builds(&self) {
        let mut children = Vec::new();
        {
            let mut builds = self.kg_builds.lock().unwrap();
            for build in builds.values_mut().filter(|b| b.running) {
                build.stopped = true;
                children.extend(build.child.clone());
            }
        }
        for child in &children {
            signal_build_stop(child);
        }
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
        let now_ms = crate::activity::now_ms();

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
                hushed: false,
                agent: SessionStatus::Gone,
                pane_hash: 0,
                user_input_pending: false,
            });
            let runs = self.registry.lock().unwrap().list_runs(&proj.id)?;
            for run in runs {
                // The run's own session, or the tab standing in for it once
                // that has been closed: a run whose first agent is gone is
                // still working, and the watch has to see the agent that is.
                let lead = lead_session_name(&run, &live);
                let agent = self.term.read().unwrap().status(&lead).unwrap_or(SessionStatus::Gone);
                let run_scripts = run_scripts_of(&run.id);
                let pane = self.term.read().unwrap().capture(&lead, 50).unwrap_or_default();
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
                // Keyed by the session the user actually typed into, which is
                // the one standing in for the run once its own tab is closed
                // (AGE-184) — the idle nudge is gated on the human having
                // driven a turn, and reading the wrong session would gate it
                // on a session nobody can type into any more.
                let typed_into = lead.strip_prefix("agency-").unwrap_or(&lead);
                let user_input_pending = self.input_seen.lock().unwrap().contains(typed_into);
                // Any run with a loop config, active OR terminal: suppression
                // must not depend on when the driver persists the terminal
                // transition, or the final attempt's exit edge (which lands on
                // the same tick) would toast "Agent exited" next to the loop's
                // own complete/stalled notification.
                let is_loop = run.loop_config.is_some();
                // The same standing the board reads, asked here so a run the
                // user has settled or snoozed stops toasting too: it is the
                // same question either way, so it is the same derivation. A
                // settle that new output has already consumed reports nothing,
                // which is why a run raising its hand still notifies.
                let hushed =
                    self.read_activity(&run, now_ms).1.standing.is_some_and(|k| k.suppresses());
                out.push(notifier::RunSnapshot {
                    id: run.id,
                    project_id: proj.id.clone(),
                    label,
                    is_terminal: run.kind == "terminal",
                    is_loop,
                    hushed,
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

/// Lines of a graph build's output handed to the settings panel on each poll.
/// The log itself keeps more (a failure's reason has to survive the progress
/// lines printed after it); this is what a panel that is 110 pixels tall can
/// usefully scroll.
const KG_LOG_SHOWN: usize = 40;

/// How long a stopped build gets to exit on its own before it is killed. A
/// build that ignores the polite signal still has to end, or the panel is left
/// on "Building" with a Stop button that does nothing.
const KG_STOP_GRACE: Duration = Duration::from_secs(5);

/// Drain one of a build's pipes into its log until EOF.
fn read_build_output(
    mut pipe: impl std::io::Read + Send + 'static,
    stream: Stream,
    log: Arc<Mutex<agency_core::buildlog::BuildLog>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                // EOF, or a pipe that broke: either way there is nothing more
                // to read and the exit status is what says how it went.
                Ok(0) | Err(_) => break,
                Ok(n) => log.lock().unwrap().push(stream, &buf[..n]),
            }
        }
        log.lock().unwrap().finish(stream);
    })
}

/// Wait for a build to exit. Polled rather than blocked on `wait`, because the
/// `Child` has to stay lockable for [`signal_build_stop`] the whole time — and
/// because a build the user stopped that is still alive after the grace period
/// gets killed here.
fn wait_for_build(
    child: &Arc<Mutex<std::process::Child>>,
    builds: &Arc<Mutex<HashMap<PathBuf, KgBuild>>>,
    repo: &Path,
) -> std::io::Result<std::process::ExitStatus> {
    let mut deadline: Option<Instant> = None;
    let mut killed = false;
    loop {
        match child.lock().unwrap().try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => return Err(e),
        }
        if !killed && builds.lock().unwrap().get(repo).is_some_and(|b| b.stopped) {
            let at = *deadline.get_or_insert_with(|| Instant::now() + KG_STOP_GRACE);
            if Instant::now() >= at {
                let _ = child.lock().unwrap().kill();
                killed = true;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Ask a running build to stop, whole process group first: the agent CLI
/// graphify spawns per document is what is actually spending the user's plan,
/// and signalling only the process Agency started leaves it running.
fn signal_build_stop(child: &Arc<Mutex<std::process::Child>>) {
    #[cfg(unix)]
    {
        let pgid = child.lock().unwrap().id();
        let signalled = std::process::Command::new("/bin/kill")
            .args(["-TERM", &format!("-{pgid}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if signalled {
            return;
        }
    }
    let _ = child.lock().unwrap().kill();
}

/// Whether the program a command line runs is installed. Command overrides and
/// the graphify defaults are full command lines, not bare binaries, and some of
/// them carry `VAR=value` in front of the program (see `command_binary`).
fn first_token_on_path(command_line: &str) -> bool {
    agency_core::config::command_binary(command_line).map(|c| command_on_path(&c)).unwrap_or(false)
}

/// True when `command` resolves to an executable file: checked directly when it
/// contains a path separator, otherwise searched across the PATH directories.
fn command_on_path(command: &str) -> bool {
    crate::agent_diag::resolve_on_path(command).is_some()
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
        agent_argv, branch_leaf_from_first_prompt, command_on_path, graphify_server, id_source,
        id_suffix, is_auto_cut_branch, new_task_id, pick_port, preview_mcp_port_for,
        require_branch_exists, require_gitless_known, require_own_branch, slugify,
        split_session_id, validate_race, RaceAttempt,
    };
    use agency_core::config::KnowledgeConfig;
    use agency_core::profile::AgentProfile;
    use std::collections::HashSet;

    /// AGE-143: the preview MCP server exists exactly where the user has said
    /// "this project serves a web app" — a web run script — and nowhere else.
    /// The port is derived from config, not allocated, so the URL emitted into
    /// an agent's MCP config survives app restarts.
    #[test]
    fn preview_server_exists_only_for_web_scripts_and_takes_the_blocks_last_port() {
        use agency_core::config::{AgencyConfig, RunScript};
        let web = RunScript {
            name: "dev".into(),
            command: "pnpm dev --port $AGENCY_PORT".into(),
            web: true,
            nonconcurrent: false,
        };
        let build = RunScript { name: "build".into(), web: false, ..web.clone() };

        let mut config = AgencyConfig::default();
        assert_eq!(preview_mcp_port_for(&config, Some(5240)), None, "no scripts at all");

        config.scripts.runs = vec![build.clone()];
        assert_eq!(preview_mcp_port_for(&config, Some(5240)), None, "no *web* script");

        config.scripts.runs = vec![build, web];
        assert_eq!(preview_mcp_port_for(&config, Some(5240)), Some(5249));
        assert_eq!(preview_mcp_port_for(&config, None), None, "no port block, no server");

        config.preview.agent_tools = false;
        assert_eq!(preview_mcp_port_for(&config, Some(5240)), None, "the off switch is real");
    }

    /// The GUI port is derived from config like the preview port, never
    /// allocated, so a relaunch serves where the pane already points. It must
    /// stay off the block's first port ($AGENCY_PORT, the workspace's dev
    /// server) and its last (the preview MCP server) or the agent's own app
    /// would fight its own GUI.
    #[test]
    fn gui_port_is_the_blocks_second_to_last_and_only_for_web_agents() {
        use agency_core::config::AgencyConfig;
        let config = AgencyConfig::default(); // block_size 10
        assert_eq!(super::gui_port_for(&config, Some(5240), "dsh"), Some(5248));
        assert_ne!(
            super::gui_port_for(&config, Some(5240), "dsh"),
            agency_core::preview::mcp_port(5240, config.ports.block_size),
            "the GUI port collides with the preview server's"
        );
        assert_eq!(super::gui_port_for(&config, Some(5240), "claude"), None);
        assert_eq!(super::gui_port_for(&config, None, "dsh"), None, "no block, no port");
        let mut small = AgencyConfig::default();
        small.ports.block_size = 2;
        assert_eq!(
            super::gui_port_for(&small, Some(5240), "dsh"),
            None,
            "a two-port block holds the app and preview ports only"
        );
    }

    /// The web recipe goes ahead of the profile's own arguments: dsh's
    /// launcher hands everything after its own flags to the booted app, so a
    /// user argument in front of `web` would be read as the app selection.
    /// `--patch` rides on the `web` subcommand itself (collapse sidebar), so it
    /// sits immediately after `web`, ahead of `--no-open` / `--port`.
    #[test]
    fn with_web_ui_prepends_the_gui_recipe_and_leaves_terminal_agents_alone() {
        let data = tempfile::tempdir().unwrap();
        let dsh = AgentProfile {
            name: "dsh".into(),
            command: "dsh".into(),
            args: vec!["--trusted-host".into(), "example.test".into()],
            env: vec![],
            resume_args: None,
            loop_args: Some(vec!["--profile".into(), "headless".into(), "{{prompt}}".into()]),
        };
        let launched = super::with_web_ui(&dsh, Some(5248), data.path());
        assert_eq!(&launched.args[0], "web");
        assert_eq!(&launched.args[1], "--patch");
        assert!(
            std::path::Path::new(&launched.args[2]).is_file(),
            "patch file missing: {}",
            launched.args[2]
        );
        assert_eq!(
            &launched.args[3..],
            ["--no-open", "--port", "5248", "--trusted-host", "example.test"]
        );
        // The loop recipe must never boot the server: headless opens no port.
        assert_eq!(launched.loop_args, dsh.loop_args);

        // No port to pin: the server still boots, on the CLI's own default.
        let no_port = super::with_web_ui(&dsh, None, data.path());
        assert_eq!(&no_port.args[0], "web");
        assert_eq!(&no_port.args[1], "--patch");
        assert_eq!(&no_port.args[3], "--no-open");
        assert!(!no_port.args.iter().any(|a| a.contains("{{port}}")));

        let claude = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        };
        assert_eq!(super::with_web_ui(&claude, Some(5248), data.path()), claude);
    }

    /// The whole interactive argv a dsh run is launched with, not just the web
    /// prefix: the recipe plus prompt delivery, which for this agent is not
    /// argv at all (`AfterGuiReady` / `web_ui::handshake`). A prompt appended
    /// here would be read as the app selection and the session would die on a
    /// usage error.
    #[test]
    fn a_web_agents_fresh_argv_is_the_server_recipe_and_nothing_else() {
        let data = tempfile::tempdir().unwrap();
        let dsh = AgentProfile {
            name: "dsh".into(),
            command: "dsh".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: Some(vec!["--profile".into(), "headless".into(), "{{prompt}}".into()]),
        };
        let profile = super::with_web_ui(&dsh, Some(5248), data.path());
        let wt = std::path::Path::new("/tmp/does-not-exist");
        let (command, args) =
            super::fresh_agent_argv(&profile, wt, "fix the login bug", None, None);
        assert_eq!(command, "dsh");
        assert_eq!(&args[0], "web");
        assert_eq!(&args[1], "--patch");
        assert!(std::path::Path::new(&args[2]).is_file());
        assert_eq!(&args[3..], ["--no-open", "--port", "5248"]);

        // The loop recipe takes the prompt the interactive launch cannot.
        let (loop_cmd, loop_args) =
            super::loop_argv(&profile, wt, "fix the login bug", None, None).unwrap();
        assert_eq!(loop_cmd, "dsh");
        assert_eq!(loop_args, vec!["--profile", "headless", "fix the login bug"]);
    }

    /// A loop attempt is headless and opens no port, so a looping run must not
    /// advertise a GUI — the pane would wait forever on a server nothing
    /// launched. Terminals never have one either.
    #[test]
    fn only_an_agent_run_that_is_not_looping_offers_a_gui() {
        use agency_core::loops::{LoopConfig, LoopState, LoopStatus};
        let base = agency_core::registry::Run {
            id: "r1".into(),
            project_id: "p1".into(),
            agent: "dsh".into(),
            prompt: String::new(),
            base: "main".into(),
            branch: "agent/r1".into(),
            created_at: 0,
            port_base: Some(5240),
            archived_at: None,
            title: None,
            kind: "agent".into(),
            merge_target: None,
            race_id: None,
            loop_config: None,
            loop_state: None,
            issue_id: None,
            worktree: true,
            model: None,
            base_commit: None,
            primary_closed_at: None,
            standing: None,
            pin_rank: None,
        };
        assert!(super::wants_web_ui(&base));

        let terminal = agency_core::registry::Run { kind: "terminal".into(), ..base.clone() };
        assert!(!super::wants_web_ui(&terminal));

        let cfg = LoopConfig {
            check_command: "true".into(),
            max_attempts: 3,
            check_timeout_secs: 60,
            max_wall_secs: None,
            max_tokens: None,
        };
        let driving = agency_core::registry::Run {
            loop_config: Some(cfg.clone()),
            loop_state: Some(LoopState::new(0)),
            ..base.clone()
        };
        assert!(!super::wants_web_ui(&driving), "a driving loop has no server to show");

        let mut done = LoopState::new(0);
        done.status = LoopStatus::Complete;
        let finished =
            agency_core::registry::Run { loop_config: Some(cfg), loop_state: Some(done), ..base };
        assert!(super::wants_web_ui(&finished), "a finished loop is interactive again");
    }

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
            ..Default::default()
        };
        let server = graphify_server(Path::new("/repo"), &custom, |_| false, |_| true).unwrap();
        assert_eq!(server.command.as_deref(), Some("my-server"));
        assert_eq!(server.args, vec!["/my graphs/g.json".to_string()]);
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
        let (cmd, args) = agent_argv(&p, no_worktree(), "do the thing", true, None, None);
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["--continue".to_string()]);
    }

    /// AGE-177: a session with a conversation on record reopens that one by
    /// name. `--continue` is the recipe that crossed two runs sharing a
    /// directory, so it is replaced rather than joined — claude given both
    /// would be asked two questions at once.
    #[test]
    fn a_recorded_conversation_replaces_the_generic_resume_recipe() {
        let id = "9674f5a1-334c-49a5-9952-89e592b0bc5b";
        let claude = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec!["--permission-mode".into(), "acceptEdits".into(), "{{prompt}}".into()],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let (cmd, args) = agent_argv(&claude, no_worktree(), "go", true, None, Some(id));
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["--permission-mode", "acceptEdits", "--resume", id]);
        assert!(!args.contains(&"--continue".to_string()));

        // A fresh launch opens a new conversation under that name, ahead of
        // the prompt, and never `--resume`: there is nothing to reopen yet.
        let (_cmd, args) = agent_argv(&claude, no_worktree(), "go", false, None, Some(id));
        assert_eq!(args, vec!["--permission-mode", "acceptEdits", "--session-id", id, "go"]);
    }

    /// Pi is pinned by its store instead, so its recipe is untouched: `-c`
    /// there already means "the most recent conversation in *this session's*
    /// store". Agents with no verified lever keep theirs too.
    #[test]
    fn agents_pinned_another_way_keep_their_own_resume_recipe() {
        let id = "9674f5a1-334c-49a5-9952-89e592b0bc5b";
        let pi = AgentProfile {
            name: "pi".into(),
            command: "pi".into(),
            args: vec![],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let (_cmd, args) = agent_argv(&pi, no_worktree(), "go", true, None, Some(id));
        assert_eq!(args, vec!["--continue".to_string()]);
        let (_cmd, args) = agent_argv(&pi, no_worktree(), "go", false, None, Some(id));
        assert_eq!(args, vec!["go".to_string()], "no id flags for a store-pinned agent");

        let codex = AgentProfile {
            name: "codex".into(),
            command: "codex".into(),
            resume_args: Some(vec!["resume".into(), "--last".into()]),
            ..pi
        };
        let (_cmd, args) = agent_argv(&codex, no_worktree(), "go", true, None, Some(id));
        assert_eq!(args, vec!["resume".to_string(), "--last".to_string()]);
    }

    /// A headless attempt names its conversation too, and the flag lands ahead
    /// of the prompt like every other argument Agency adds.
    #[test]
    fn loop_attempts_name_their_conversation() {
        let id = "9674f5a1-334c-49a5-9952-89e592b0bc5b";
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
        let (_cmd, args) =
            super::loop_argv(&claude, no_worktree(), "fix the tests", None, Some(id)).unwrap();
        assert_eq!(
            args,
            vec!["-p", "--session-id", id, "fix the tests", "--permission-mode", "acceptEdits"]
        );
    }

    /// AGE-100: the resume recipe replaces the prompt, not the whole of the
    /// user's arguments. A profile that runs with `--permission-mode
    /// acceptEdits` on its first launch has to keep it on every resume, or the
    /// session quietly changes terms halfway through the work.
    #[test]
    fn agent_argv_keeps_custom_profile_args_on_resume() {
        let p = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec![
                "--permission-mode".into(),
                "acceptEdits".into(),
                "--add-dir".into(),
                "/tmp/shared".into(),
                "{{prompt}}".into(),
            ],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let (cmd, args) = agent_argv(&p, no_worktree(), "do the thing", true, None, None);
        assert_eq!(cmd, "claude");
        assert_eq!(
            args,
            vec!["--permission-mode", "acceptEdits", "--add-dir", "/tmp/shared", "--continue"]
        );

        // Args the user never templated survive too: a profile with no
        // `{{prompt}}` token at all still gets its flags on resume.
        let untemplated = AgentProfile { args: vec!["--verbose".into()], ..p.clone() };
        let (_cmd, args) = agent_argv(&untemplated, no_worktree(), "go", true, None, None);
        assert_eq!(args, vec!["--verbose".to_string(), "--continue".to_string()]);

        // The flags come first because `codex resume --last` is a subcommand:
        // anything written after it would be read as the subcommand's.
        let codex = AgentProfile {
            name: "codex".into(),
            command: "codex".into(),
            args: vec!["--sandbox".into(), "workspace-write".into()],
            resume_args: Some(vec!["resume".into(), "--last".into()]),
            ..p
        };
        let (_cmd, args) = agent_argv(&codex, no_worktree(), "go", true, None, None);
        assert_eq!(args, vec!["--sandbox", "workspace-write", "resume", "--last"]);
    }

    /// A prompt written as a flag's value takes the flag with it, or `copilot
    /// -i --continue` would resume with "--continue" as its opening prompt.
    /// Only the flag the agent's own recipe names goes: a boolean flag ahead of
    /// a positional prompt is a flag the user wants on every launch.
    #[test]
    fn agent_argv_drops_the_flag_the_prompt_was_the_value_of() {
        let copilot = AgentProfile {
            name: "copilot".into(),
            command: "copilot".into(),
            args: vec!["--banner".into(), "-i".into(), "{{prompt}}".into()],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let (_cmd, args) = agent_argv(&copilot, no_worktree(), "go", true, None, None);
        assert_eq!(args, vec!["--banner".to_string(), "--continue".to_string()]);

        let opencode = AgentProfile {
            name: "opencode".into(),
            command: "opencode".into(),
            args: vec!["--prompt".into(), "{{prompt}}".into()],
            ..copilot.clone()
        };
        let (_cmd, args) = agent_argv(&opencode, no_worktree(), "go", true, None, None);
        assert_eq!(args, vec!["--continue".to_string()]);

        // claude takes its prompt positionally, so the flag before it is the
        // user's own and stays.
        let claude = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec!["--dangerously-skip-permissions".into(), "{{prompt}}".into()],
            ..copilot
        };
        let (_cmd, args) = agent_argv(&claude, no_worktree(), "go", true, None, None);
        assert_eq!(
            args,
            vec!["--dangerously-skip-permissions".to_string(), "--continue".to_string()]
        );
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
        let (cmd, args) = agent_argv(&p, no_worktree(), "hello", true, None, None);
        assert_eq!(cmd, "cursor-agent");
        assert_eq!(args, vec!["hello".to_string()]);
    }

    /// AGE-137: a rerun and the resume fallback both take the fresh argv, and
    /// since AGE-79 moved prompt delivery out of the profile's args the
    /// catalogue ships `args: []`. Rendering those args and stopping there
    /// opened `claude` in the worktree with the task text nowhere; it only ever
    /// looked right for a profile carrying `{{prompt}}` by hand.
    #[test]
    fn agent_argv_fresh_delivers_the_prompt_to_a_catalogue_profile() {
        let claude = AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec![],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let (cmd, args) = agent_argv(&claude, no_worktree(), "do the thing", false, None, None);
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["do the thing".to_string()]);
        // One fresh recipe: whichever door a caller comes in by, same argv.
        assert_eq!(
            super::fresh_agent_argv(&claude, no_worktree(), "do the thing", None, None),
            (cmd, args)
        );

        // A flag-valued CLI gets its own recipe, not a bare positional.
        let opencode =
            AgentProfile { name: "opencode".into(), command: "opencode".into(), ..claude.clone() };
        let (_cmd, args) = agent_argv(&opencode, no_worktree(), "go", false, None, None);
        assert_eq!(args, vec!["--prompt".to_string(), "go".to_string()]);

        // And the resume branch still leaves the prompt out: the session it
        // rejoins already has it (AGE-100).
        let (_cmd, args) = agent_argv(&claude, no_worktree(), "do the thing", true, None, None);
        assert_eq!(args, vec!["--continue".to_string()]);
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
        let (_cmd, args) = agent_argv(&p, no_worktree(), "fresh prompt", false, None, None);
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
        let (cmd, args) =
            super::fresh_agent_argv(&templated, no_worktree(), "review it", None, None);
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["--flag".to_string(), "review it".to_string()]);

        let plain = AgentProfile { args: vec!["--flag".into()], ..templated.clone() };
        let (_cmd, args) = super::fresh_agent_argv(&plain, no_worktree(), "review it", None, None);
        assert_eq!(args, vec!["--flag".to_string(), "review it".to_string()]);

        // An empty prompt is the promptless launch every ordinary tab uses.
        let (_cmd, args) = super::fresh_agent_argv(&plain, no_worktree(), "", None, None);
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
            super::fresh_agent_argv(&profile(name), no_worktree(), prompt, None, None).1
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
        let (_cmd, args) = super::fresh_agent_argv(&hand_rolled, no_worktree(), "go", None, None);
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
        let (_cmd, args) = super::fresh_agent_argv(&copilot, dir.path(), "", None, None);
        assert!(args.is_empty(), "{args:?}");

        agency_core::mcp::emit_for_agent(
            "copilot",
            dir.path(),
            dir.path(),
            &[agency_core::mcp::McpServer {
                name: "kg".into(),
                command: Some("graphify".into()),
                ..Default::default()
            }],
        )
        .unwrap();

        // Fresh launch: flags first, prompt still last (behind `-i`, per AGE-79).
        let (cmd, args) = super::fresh_agent_argv(&copilot, dir.path(), "go", None, None);
        assert_eq!(cmd, "copilot");
        assert_eq!(args, vec![flag.clone(), arg.clone(), "-i".to_string(), "go".to_string()]);

        // Resume launch: the recipe keeps its own args and gains the config.
        let (_cmd, args) = agent_argv(&copilot, dir.path(), "go", true, None, None);
        assert_eq!(args, vec!["--continue".to_string(), flag.clone(), arg.clone()]);

        // A loop recipe gets it ahead of wherever it places the prompt.
        let looping = AgentProfile {
            loop_args: Some(vec!["-p".into(), "{{prompt}}".into()]),
            ..copilot.clone()
        };
        let (_cmd, args) = super::loop_argv(&looping, dir.path(), "go", None, None).unwrap();
        assert_eq!(args, vec!["-p".to_string(), flag.clone(), arg.clone(), "go".to_string()]);

        // The user's own flag wins: Agency doesn't add a rival copy.
        let hand_rolled =
            AgentProfile { args: vec![flag.clone(), "@/my/own.json".into()], ..copilot.clone() };
        let (_cmd, args) = super::fresh_agent_argv(&hand_rolled, dir.path(), "", None, None);
        assert_eq!(args, vec![flag.clone(), "@/my/own.json".to_string()]);

        // Agents that read the emitted file unaided get no extra flags.
        let claude = AgentProfile { name: "claude".into(), command: "claude".into(), ..copilot };
        let (_cmd, args) = super::fresh_agent_argv(&claude, dir.path(), "", None, None);
        assert!(args.is_empty(), "{args:?}");
    }

    #[test]
    fn pr_review_prompt_switches_on_post_comments() {
        use super::PrWorkspace;
        let wt = PrWorkspace::Worktree;
        let quiet = super::pr_review_prompt(
            12,
            "Add widgets",
            "https://x/pull/12",
            "main",
            wt.clone(),
            false,
        );
        assert!(quiet.contains("Review GitHub pull request #12: Add widgets"));
        assert!(quiet.contains("git diff main...HEAD"));
        assert!(quiet.contains("Do not post anything to GitHub"));
        assert!(!quiet.contains("gh api"));
        // Both modes promise the follow-up fixing session the review is for.
        assert!(quiet.contains("stay available"));

        let posting =
            super::pr_review_prompt(12, "Add widgets", "https://x/pull/12", "main", wt, true);
        assert!(posting.contains("repos/$SLUG/pulls/12/reviews"));
        assert!(posting.contains("\"event\": \"COMMENT\""));
        assert!(!posting.contains("Do not post anything to GitHub"));
        assert!(posting.contains("stay available"));
    }

    /// A review that had to run in the project's own checkout is standing in
    /// the user's working copy, and an agent that does not know that will read
    /// unrelated uncommitted changes as part of the PR, or switch branches out
    /// from under them. The worktree case must not carry the warning: it would
    /// be a lie about an isolated workspace.
    #[test]
    fn pr_review_prompt_warns_only_when_it_is_the_users_checkout() {
        use super::PrWorkspace;
        let own = super::pr_review_prompt(
            12,
            "W",
            "https://x/pull/12",
            "main",
            PrWorkspace::Worktree,
            false,
        );
        assert!(!own.contains("project's own checkout"), "{own}");

        let shared = super::pr_review_prompt(
            12,
            "W",
            "https://x/pull/12",
            "main",
            PrWorkspace::Checkout,
            false,
        );
        assert!(shared.contains("project's own checkout"), "{shared}");
        assert!(shared.contains("Do not switch branches, stash, reset, or revert"), "{shared}");
        // The checkout can lag the PR head, so the diff must come from the PR.
        assert!(shared.contains("gh pr diff 12"), "{shared}");
        // Still the same review, with the same closing promise.
        assert!(shared.contains("Review GitHub pull request #12"), "{shared}");
        assert!(shared.contains("stay available"), "{shared}");
    }

    #[test]
    fn pr_conflict_prompt_names_the_files_and_forbids_a_force_push() {
        let files = vec!["src/a.rs".to_string(), "ui/b.tsx".to_string()];
        let p = super::pr_conflict_prompt(
            9,
            "Add widgets",
            "https://x/pull/9",
            "main",
            "feat/w",
            super::PrWorkspace::Worktree,
            &files,
        );
        assert!(p.starts_with(
            "Resolve the merge conflicts blocking GitHub pull request #9: Add widgets"
        ));
        // The probe already ran to draw the UI, so the agent starts from its answer.
        assert!(p.contains("These 2 files conflict: src/a.rs, ui/b.tsx."), "{p}");
        assert!(p.contains("git merge origin/main"), "{p}");
        assert!(p.contains("`feat/w`") && p.contains("`main`"), "{p}");
        // A rebase or force-push on a published branch is how this fix turns
        // into a worse problem than the conflict it cleared.
        assert!(p.contains("Do not rebase and do not force-push"), "{p}");
        assert!(p.contains("https://x/pull/9"), "{p}");
        // A worktree of its own is the agent's to work in; only the shared
        // checkout gets the warning about someone else's uncommitted work.
        assert!(!p.contains("project's own checkout"), "{p}");
    }

    #[test]
    fn pr_conflict_prompt_warns_when_the_merge_lands_in_the_users_checkout() {
        let p = super::pr_conflict_prompt(
            9,
            "Add widgets",
            "https://x/pull/9",
            "main",
            "feat/w",
            super::PrWorkspace::Checkout,
            &["a.rs".to_string()],
        );
        assert!(p.contains("the project's own checkout, not an isolated worktree"), "{p}");
        // git refuses a merge over someone else's uncommitted work, and moving
        // it out of the way is not this agent's call to make.
        assert!(p.contains("stop and tell me rather than stashing"), "{p}");
    }

    #[test]
    fn pr_conflict_prompt_survives_an_unprobed_conflict() {
        let p = super::pr_conflict_prompt(
            9,
            "Add widgets",
            "https://x/pull/9",
            "main",
            "feat/w",
            super::PrWorkspace::Worktree,
            &[],
        );
        assert!(p.contains("Run the merge to find out which files collide."), "{p}");
        assert!(!p.contains("files conflict"), "no empty file list: {p}");
        assert!(p.contains("git merge origin/main"), "{p}");
    }

    #[test]
    fn pr_conflict_prompt_counts_one_file_in_the_singular() {
        let p = super::pr_conflict_prompt(
            9,
            "T",
            "u",
            "main",
            "feat/w",
            super::PrWorkspace::Worktree,
            &["a.rs".to_string()],
        );
        assert!(p.contains("One file conflicts: a.rs."), "{p}");
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
        let (_cmd, args) = super::fresh_agent_argv(&pinned, no_worktree(), "go", None, None);
        assert_eq!(args, vec!["--model", "opus", "go"]);
        // Resume: the same session must come back on the same model, and on
        // exactly one `--model` — it rides in on the profile's args (AGE-100),
        // so the resume recipe must not carry a second copy.
        let (_cmd, args) = agent_argv(&pinned, no_worktree(), "go", true, None, None);
        assert_eq!(args, vec!["--model", "opus", "--continue"]);
        // Loop: at the end, so {{prompt}} stays where the recipe put it.
        let (_cmd, args) = super::loop_argv(&pinned, no_worktree(), "go", None, None).unwrap();
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
        let (cmd, args) = super::loop_argv(&p, no_worktree(), "fix the tests", None, None).unwrap();
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["-p", "fix the tests", "--permission-mode", "acceptEdits"]);

        let no_recipe = AgentProfile { loop_args: None, ..p.clone() };
        assert!(super::loop_argv(&no_recipe, no_worktree(), "x", None, None).is_err());

        // An empty recipe is no recipe — Settings saves None for an empty
        // field, but a hand-edited profile must not slip through.
        let empty_recipe = AgentProfile { loop_args: Some(vec![]), ..p };
        assert!(super::loop_argv(&empty_recipe, no_worktree(), "x", None, None).is_err());
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
        let (cmd, args) = super::loop_argv(&p, no_worktree(), "fix the tests", None, None).unwrap();
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

    /// AGE-139 in miniature: the dispatched prompt is the issue title followed
    /// by its body, and that body led with a link, so the branch cut from the
    /// prompt carried the URL into a merge commit. The title is what names the
    /// branch now.
    #[test]
    fn an_issue_run_takes_its_id_from_the_title_not_the_body() {
        let title = "AGE-139 Competitive reading";
        let prompt = "Work on issue AGE-139: Competitive reading\n\nhttps://github.com/some/repo\n\nis this the same thing as our product?";
        let id = new_task_id(id_source(Some(title), prompt));
        assert!(id.starts_with("age-139-competitive-reading-"), "unexpected id: {id}");
        assert!(!id.contains("https"), "the body's URL reached the branch name: {id}");
    }

    #[test]
    fn a_run_with_no_title_still_takes_its_id_from_the_prompt() {
        assert_eq!(id_source(None, "fix the login page"), "fix the login page");
        // A blank title is what a promptless run carries; it names nothing.
        assert_eq!(id_source(Some("  "), "fix the login page"), "fix the login page");
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

    /// A worktree run, for the pure helpers that only read the row.
    fn agent_run(id: &str) -> agency_core::registry::Run {
        agency_core::registry::Run {
            id: id.to_string(),
            project_id: "proj".to_string(),
            agent: "claude".to_string(),
            prompt: "do a thing".to_string(),
            base: "main".to_string(),
            branch: format!("agent/{id}"),
            created_at: 0,
            port_base: None,
            archived_at: None,
            title: None,
            kind: "agent".to_string(),
            merge_target: None,
            race_id: None,
            loop_config: None,
            loop_state: None,
            issue_id: None,
            worktree: true,
            model: None,
            base_commit: None,
            primary_closed_at: None,
            standing: None,
            pin_rank: None,
        }
    }

    fn live(names: &[&str]) -> Vec<(String, super::SessionStatus)> {
        names.iter().map(|n| ((*n).to_string(), super::SessionStatus::Running)).collect()
    }

    #[test]
    fn a_run_speaks_through_its_own_session_until_that_tab_is_closed() {
        let run = agent_run("fix-a1");
        let sessions = live(&["agency-fix-a1", "agency-fix-a1--2", "agency-fix-a1--3"]);
        assert_eq!(super::lead_session_name(&run, &sessions), "agency-fix-a1");
    }

    /// AGE-184: with the run's own tab closed, the leftmost tab still alive
    /// carries the run — its status dot, its notifier watch, its prompts.
    #[test]
    fn a_closed_first_tab_hands_the_run_to_its_lowest_numbered_tab() {
        let mut run = agent_run("fix-a1");
        run.primary_closed_at = Some(1);
        let sessions = live(&["agency-fix-a1--3", "agency-fix-a1--2"]);
        assert_eq!(super::lead_session_name(&run, &sessions), "agency-fix-a1--2");
        // Double digits sort as numbers, not as text.
        let sessions = live(&["agency-fix-a1--10", "agency-fix-a1--9"]);
        assert_eq!(super::lead_session_name(&run, &sessions), "agency-fix-a1--9");
    }

    #[test]
    fn a_closed_first_tab_with_nothing_left_alive_reads_as_gone() {
        let mut run = agent_run("fix-a1");
        run.primary_closed_at = Some(1);
        // The run's own name, whose status is Gone — which is the truth about
        // a run with no agent running in it.
        assert_eq!(super::lead_session_name(&run, &[]), "agency-fix-a1");
        // Neither a shell nor a run script is an agent tab.
        let sessions = live(&["agency-shell-fix-a1", "agency-run-fix-a1#dev"]);
        assert_eq!(super::lead_session_name(&run, &sessions), "agency-fix-a1");
    }

    #[test]
    fn a_neighbouring_runs_tabs_never_speak_for_this_one() {
        let mut run = agent_run("fix-a1");
        run.primary_closed_at = Some(1);
        // `fix-a12--2` starts with this run's name; the `--` boundary is what
        // keeps it out.
        let sessions = live(&["agency-fix-a12--2"]);
        assert_eq!(super::lead_session_name(&run, &sessions), "agency-fix-a1");
    }

    fn attempt(agent: &str, model: Option<&str>) -> RaceAttempt {
        RaceAttempt { agent: agent.to_string(), model: model.map(str::to_string) }
    }

    /// The point of AGE-118: an agent can appear more than once in a race, so
    /// long as the attempts are the unit being counted.
    #[test]
    fn a_race_can_be_one_agent_on_two_models() {
        let attempts = [attempt("claude", Some("opus")), attempt("claude", Some("sonnet"))];
        assert!(validate_race("build it", &attempts).is_ok());
    }

    #[test]
    fn a_race_needs_two_attempts_and_a_prompt() {
        assert!(validate_race("build it", &[attempt("claude", None)]).is_err());
        assert!(validate_race("build it", &[]).is_err());
        assert!(
            validate_race("   ", &[attempt("claude", None), attempt("codex", None)]).is_err(),
            "an empty prompt is nothing to send at launch"
        );
    }

    /// Refused before the first workspace is cut, not on the attempt itself:
    /// create_run_spec would reject it too, by which point its rivals are
    /// already running.
    #[test]
    fn a_race_refuses_a_model_an_agent_cannot_be_told() {
        let attempts = [attempt("claude", Some("opus")), attempt("crush", Some("opus"))];
        assert!(validate_race("build it", &attempts).is_err());
        // The same agent left on its own default is fine: nothing has to reach
        // a command line for it.
        let attempts = [attempt("claude", Some("opus")), attempt("crush", None)];
        assert!(validate_race("build it", &attempts).is_ok());
    }

    #[test]
    fn new_task_id_disambiguates_same_prompt() {
        let a = new_task_id("same prompt");
        let b = new_task_id("same prompt");
        assert_ne!(a, b);
    }

    /// AGE-183: the first-prompt rename rebuilds the leaf from the prompt but
    /// keeps the four-character suffix already on the run id, so the worktree
    /// dir and daemon session (which stay on that id) still match the branch.
    #[test]
    fn branch_leaf_from_first_prompt_keeps_the_run_suffix() {
        let leaf = branch_leaf_from_first_prompt(
            "agent-36a2",
            "interesting/memorable names? I'm thinking about like",
        )
        .unwrap();
        assert!(leaf.ends_with("-36a2"), "unexpected leaf: {leaf}");
        assert!(leaf.starts_with("interesting-memorable-names"), "unexpected leaf: {leaf}");
        assert!(!leaf.starts_with("agent-"), "fallback slug leaked into the leaf: {leaf}");
    }

    #[test]
    fn branch_leaf_from_first_prompt_skips_the_empty_fallback() {
        assert!(branch_leaf_from_first_prompt("agent-36a2", "").is_none());
        assert!(branch_leaf_from_first_prompt("agent-36a2", "!!! ???").is_none());
    }

    #[test]
    fn id_suffix_reads_the_four_char_disambiguator() {
        assert_eq!(id_suffix("add-a-login-page-a3k2"), Some("a3k2"));
        assert_eq!(id_suffix("agent-36a2"), Some("36a2"));
        // No hyphen at all, and a trailing segment that is not the 4-char shape.
        assert_eq!(id_suffix("nope"), None);
        assert_eq!(id_suffix("add-a-login-page-toolong"), None);
    }

    #[test]
    fn is_auto_cut_branch_matches_only_the_empty_prompt_fallback() {
        assert!(is_auto_cut_branch("agent-36a2", "agent/agent-36a2"));
        assert!(!is_auto_cut_branch("agent-36a2", "agent/interesting-memorable-names-36a2"));
        assert!(!is_auto_cut_branch("agent-36a2", "agent/agent-other"));
    }

    /// A branch derived from a real prompt is a good name, so it is not the
    /// fallback and is not the first prompt's to overwrite — even though it is
    /// still exactly `agent/<id>`, the shape this used to test for alone.
    #[test]
    fn is_auto_cut_branch_spares_a_prompt_derived_name() {
        assert!(!is_auto_cut_branch(
            "narrate-the-weekly-review-note-docs-week-q3w7",
            "agent/narrate-the-weekly-review-note-docs-week-q3w7"
        ));
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
            base_commit: None,
            primary_closed_at: None,
            standing: None,
            pin_rank: None,
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
            base_commit: None,
            primary_closed_at: None,
            standing: None,
            pin_rank: None,
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

    /// AGE-169: reviewing a PR failed outright whenever the PR's branch was
    /// checked out anywhere, which is the normal state right after you push a
    /// branch from your own checkout and open the PR from it. The checkout is
    /// where the review belongs in that case, and only a third tree is refused.
    #[test]
    fn pr_workspace_sends_the_review_where_the_branch_already_is() {
        use super::{pr_workspace, PrWorkspace};
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .status()
                .unwrap()
                .success());
        };

        // Nobody holds it: the review gets a worktree of its own, and there is
        // no tree to look for a host run in.
        git(&["branch", "features/free"]);
        let free = pr_workspace(repo.path(), "features/free");
        assert_eq!(free, PrWorkspace::Worktree);
        assert_eq!(free.holder(repo.path()), None);

        // The user's own checkout holds it: the review runs there instead of
        // failing, since that is the only tree a fix could be committed in.
        let head = agency_core::merge::current_branch(repo.path()).unwrap();
        let checkout = pr_workspace(repo.path(), &head);
        assert_eq!(checkout, PrWorkspace::Checkout);
        assert_eq!(checkout.holder(repo.path()), Some(repo.path().to_path_buf()));

        // A third tree holds it. That tree is the holder, so an agent run of
        // ours living there can still host the review.
        let other = repo.path().join("other-wt");
        git(&["worktree", "add", "-q", "-b", "features/busy", other.to_str().unwrap()]);
        // Compared with `same_dir`, not `==`: git answers with the resolved
        // path ("/private/var/…" for a macOS temp dir) while the caller holds
        // the unresolved one, which is the whole reason the run lookup uses it.
        let held = pr_workspace(repo.path(), "features/busy");
        let PrWorkspace::Held(dir) = &held else { panic!("a third tree holds it: {held:?}") };
        assert!(super::same_dir(dir, &other), "{dir:?}");
        assert!(held.holder(repo.path()).is_some_and(|d| super::same_dir(&d, &other)));

        // With nothing of ours there it is refused, but by naming the directory
        // and what to do, not by repeating git's "already used by worktree".
        let err = super::branch_held_elsewhere("features/busy", &other).to_string();
        assert!(err.contains("features/busy"), "names the branch: {err}");
        assert!(err.contains("other-wt"), "names the tree holding it: {err}");
        assert!(err.contains("another branch"), "says what to do next: {err}");
        assert!(!err.contains("fatal:"), "no raw git error: {err}");
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
