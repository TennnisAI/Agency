use agency_core::profile::AgentProfile;
use agency_core::registry::{IssueStatus, Project, Registry};
use agency_core::supervisor::AgentHandle;
use agency_core::term::client::{Subscription, TermClient};
use agency_core::term::SessionStatus;
use agency_core::worktree::WorktreeManager;
use anyhow::{anyhow, bail, Result};
use crate::notifier;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use uuid;

const SETTING_LM_STUDIO_URL: &str = "lm_studio_base_url";
const DEFAULT_LM_STUDIO_URL: &str = "http://localhost:1234/v1";
// Agent the "New Agent" menu/shortcut spawns. Empty = auto (project's last-used).
const SETTING_DEFAULT_AGENT: &str = "default_agent";
const SETTING_NOTIF: &str = "notification_settings";
const SETTING_MCP: &str = "mcp_servers";
/// Set to "1" once the user finishes agent-profile onboarding (or is migrated).
const SETTING_AGENT_ONBOARDING: &str = "agent_onboarding_completed";

const MERGE_RESOLVER_SKILL: &str = include_str!("../../../skills/merge-resolver/SKILL.md");

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    pub lm_studio_base_url: String,
    /// Agent id the "New Agent" menu/shortcut spawns. `None` = auto (fall back
    /// to the project's last-used agent).
    pub default_agent: Option<String>,
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
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
    pub port: Option<u16>,
    pub kind: String,
    pub race_id: Option<String>,
    pub loop_config: Option<agency_core::loops::LoopConfig>,
    pub loop_state: Option<agency_core::loops::LoopState>,
    pub issue_id: Option<String>,
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
    /// Local issue this run is dispatched from (see start_issue_*): stored on
    /// the run so merge/PR/discard can drive the issue's status.
    issue_id: Option<String>,
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

/// Decide the (command, args) to launch for an agent run. With `use_resume` and a
/// resume recipe present, launch the resume args (no prompt). Otherwise launch a
/// fresh session from the rendered prompt. The optional setup script wraps the
/// command in both cases (same as create_run/rerun).
fn agent_argv(
    profile: &AgentProfile,
    prompt: &str,
    use_resume: bool,
    setup: Option<&str>,
) -> (String, Vec<String>) {
    let base_args: Vec<String> = match (use_resume, &profile.resume_args) {
        (true, Some(resume)) => resume.clone(),
        _ => profile
            .render_args(prompt)
            .into_iter()
            .filter(|a| !a.is_empty())
            .collect(),
    };
    agency_core::scripts::wrap_setup(setup, &profile.command, &base_args)
}

/// The (command, args) for one headless loop attempt: the profile's loop
/// recipe with `{{prompt}}` filled in, wrapped by the optional setup script.
/// Errors when the profile has no loop recipe — such agents can't loop.
fn loop_argv(
    profile: &AgentProfile,
    prompt: &str,
    setup: Option<&str>,
) -> Result<(String, Vec<String>)> {
    let recipe = profile.loop_args.as_ref().filter(|r| !r.is_empty()).ok_or_else(|| {
        anyhow!("agent '{}' has no loop recipe — set the profile's loop args first", profile.name)
    })?;
    let mut args: Vec<String> = recipe.iter().map(|a| a.replace("{{prompt}}", prompt)).collect();
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

fn run_session_name(id: &str) -> String {
    format!("agency-run-{id}")
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
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string())
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
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
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
    resolvers: Mutex<HashMap<String, AgentHandle>>,
    ui: Mutex<UiState>,
    /// Run ids that have received user input since their last "waiting for input"
    /// notification. Drives idle-notification gating (see `notifier::step`).
    input_seen: Mutex<HashSet<String>>,
    /// Per-run busy/idle state, written by the notifier tick from its pane-hash
    /// diff and read into `RunInfo` (see `crate::activity`). In-memory only:
    /// it re-derives within one tick of an app start.
    activity: Mutex<HashMap<String, crate::activity::ActivityEntry>>,
    /// Run ids that have ever been given a turn (typed Enter, or created with a
    /// non-empty prompt). Unlike `input_seen` this is never consumed — it
    /// separates "waiting on the user" from "idle, never prompted" in
    /// `activity::classify`. In-memory: forgotten runs just show idle.
    prompted: Mutex<HashSet<String>>,
    /// Serializes merges. The merge sequence (status check → checkout →
    /// merge) runs in the shared primary checkout and is not atomic, so a
    /// second concurrent merge (double-click, another run's Approve) must
    /// fail fast instead of interleaving.
    merge_gate: Mutex<()>,
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
        let onboarding_done = registry
            .get_setting(SETTING_AGENT_ONBOARDING)?
            .as_deref()
            == Some("1");
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
            resolvers: Mutex::new(HashMap::new()),
            ui: Mutex::new(UiState { focused: true, active_run: None, pending_open: None }),
            input_seen: Mutex::new(HashSet::new()),
            activity: Mutex::new(HashMap::new()),
            prompted: Mutex::new(HashSet::new()),
            merge_gate: Mutex::new(()),
            worktree_gate: Mutex::new(()),
            checks: Mutex::new(HashMap::new()),
            loop_gate: Mutex::new(()),
            loops_active: std::sync::atomic::AtomicBool::new(true),
            loop_generation: std::sync::atomic::AtomicU64::new(0),
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
        Ok(self
            .registry
            .lock()
            .unwrap()
            .list_profiles()?
            .into_iter()
            .map(|p| p.name)
            .collect())
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

    /// Catalog entries with enabled/installed flags for the UI picker.
    pub fn list_agent_catalog(&self) -> Result<Vec<crate::agent_catalog::CatalogEntryInfo>> {
        let builtins = crate::agent_catalog::builtins();
        // Read the enabled flags under the lock, then drop it before probing
        // PATH (a per-command filesystem scan) so the registry mutex isn't held
        // across ~10 syscall-heavy lookups.
        let enabled: Vec<bool> = {
            let reg = self.registry.lock().unwrap();
            builtins
                .iter()
                .map(|e| Ok(reg.get_profile(e.id)?.is_some()))
                .collect::<Result<_>>()?
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
        self.registry
            .lock()
            .unwrap()
            .set_setting(SETTING_AGENT_ONBOARDING, "1")?;
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
        })
    }

    pub fn save_settings(&self, s: &ProviderSettings) -> Result<()> {
        validate_provider_url(&s.lm_studio_base_url)?;
        let reg = self.registry.lock().unwrap();
        reg.set_setting(SETTING_LM_STUDIO_URL, &s.lm_studio_base_url)?;
        reg.set_setting(SETTING_DEFAULT_AGENT, s.default_agent.as_deref().unwrap_or(""))?;
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
        validate_repo(repo_path)?;
        self.registry.lock().unwrap().add_project(name, repo_path)
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

    pub fn commit_repo(
        &self,
        repo_path: &Path,
        add_gitignore: bool,
        on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<()> {
        agency_core::setup::initial_commit_with_progress(repo_path, add_gitignore, on_progress)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.registry.lock().unwrap().list_projects()
    }

    pub fn close_project(&self, id: &str) -> Result<()> {
        // Kill live terminals; keep project + run records (and extra-session
        // rows) so reopen can re-run and revive the tabs.
        let runs = self.registry.lock().unwrap().list_runs(id)?;
        for run in &runs {
            self.attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.read().unwrap().kill(&session_name(&run.id));
            self.run_attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.read().unwrap().kill(&run_session_name(&run.id));
            self.shell_attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.read().unwrap().kill(&shell_session_name(&run.id));
            self.kill_extra_sessions(&run.id);
        }
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        let runs = self.registry.lock().unwrap().list_runs(id)?;
        let repo = self.project_repo(id).ok();
        for run in &runs {
            self.attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.read().unwrap().kill(&session_name(&run.id));
            self.run_attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.read().unwrap().kill(&run_session_name(&run.id));
            self.shell_attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.read().unwrap().kill(&shell_session_name(&run.id));
            self.kill_extra_sessions(&run.id);
            if let Some(repo) = &repo {
                let _ = WorktreeManager::new(repo.clone()).remove(&run.id);
            }
            let reg = self.registry.lock().unwrap();
            reg.delete_run_sessions(&run.id)?;
            reg.delete_run(&run.id)?;
        }
        {
            let reg = self.registry.lock().unwrap();
            reg.delete_project_issues(id)?;
            reg.remove_project(id)?;
        }
        Ok(())
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
        let name = session_name(&run.id);
        let status = self.term.read().unwrap().status(&name).unwrap_or(SessionStatus::Gone);
        let wt = self
            .project_repo(&run.project_id)
            .ok()
            .map(|repo| repo.join(".agency").join("worktrees").join(&run.id));
        let stat = wt
            .filter(|p| p.exists())
            .and_then(|p| agency_core::git::diff_stat(&p, &run.base).ok())
            .unwrap_or(agency_core::git::DiffStat { added: 0, deleted: 0, files: 0 });
        RunInfo {
            id: run.id.clone(),
            project_id: run.project_id.clone(),
            agent: run.agent.clone(),
            prompt: run.prompt.clone(),
            title: run.title.clone(),
            branch: run.branch.clone(),
            status,
            activity: self.activity.lock().unwrap().get(&run.id).map(|e| {
                // Loops drive themselves — a quiet attempt isn't waiting on
                // the user, so it classifies as idle at most.
                let turn_driven = run.loop_config.is_none()
                    && self.prompted.lock().unwrap().contains(&run.id);
                crate::activity::classify(e, turn_driven, crate::activity::now_ms())
            }),
            added: stat.added,
            deleted: stat.deleted,
            files: stat.files,
            port: run.port_base,
            kind: run.kind.clone(),
            race_id: run.race_id.clone(),
            loop_config: run.loop_config.clone(),
            loop_state: run.loop_state.clone(),
            issue_id: run.issue_id.clone(),
        }
    }

    // ── run lifecycle ──────────────────────────────────────────────────────────

    fn allocate_port(&self, base: u16, block_size: u16) -> Result<u16> {
        let used: std::collections::HashSet<u16> =
            self.registry.lock().unwrap().list_port_bases()?.into_iter().collect();
        pick_port(&used, base, block_size).ok_or_else(|| anyhow!("no free port block available"))
    }

    pub fn create_run(&self, project_id: &str, prompt: &str, agent: &str, base: &str, merge_target: Option<&str>) -> Result<RunInfo> {
        self.create_run_with_progress(project_id, prompt, agent, base, merge_target, |_| {})
    }

    /// Like [`create_run`], but streams workspace-setup progress to `on_progress`
    /// (worktree checkout + essential-file copy). Used by the async `create_run`
    /// Tauri command so the UI shows movement instead of freezing on a large repo.
    pub fn create_run_with_progress(
        &self,
        project_id: &str,
        prompt: &str,
        agent: &str,
        base: &str,
        merge_target: Option<&str>,
        mut on_progress: impl FnMut(agency_core::setup::CloneProgress),
    ) -> Result<RunInfo> {
        self.create_run_spec(
            NewRunSpec {
                project_id,
                prompt,
                agent,
                base,
                merge_target,
                race_id: None,
                title: None,
                existing_branch: None,
                loop_config: None,
                issue_id: None,
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
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(spec.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {agent}", agent = spec.agent))?
        };
        let id = new_task_id(spec.prompt);
        let manager = WorktreeManager::new(repo.clone());
        // Default: cut agent/<id> from the base. PR-review runs instead check
        // out the PR's existing head branch.
        let worktree = match &spec.existing_branch {
            Some(branch) => manager.create_on_branch_with_progress(&id, branch, on_progress)?,
            None => manager.create_with_progress(&id, spec.base, on_progress)?,
        };
        // Untracked essentials (.env etc.) don't come with a worktree; copy the
        // configured list plus auto-detected root .env files. Best-effort: a bad
        // entry shouldn't block the run.
        on_progress(agency_core::setup::CloneProgress {
            phase: "Copying files".into(),
            percent: None,
            detail: String::new(),
        });
        if let Err(e) = manager.copy_essentials(&id, &config.files.copy) {
            log::warn!("copying essentials into worktree {id}: {e}");
        }
        self.emit_mcp(spec.agent, &repo, &worktree.path, &config);

        // Looping runs are spawned below via spawn_loop_attempt — the same
        // path the driver uses for every respawn — so there is exactly one
        // place that builds a headless attempt.
        if spec.loop_config.is_none() {
            let mut env = self.provider_env()?;
            env.extend(profile.env.iter().cloned());
            env.extend(agency_core::scripts::script_env(&worktree.path, &repo, &id, Some(port)));
            let mut args: Vec<String> = profile
                .render_args(spec.prompt)
                .into_iter()
                .filter(|a| !a.is_empty())
                .collect();
            // Deliver a non-empty prompt as the agent's initial positional prompt
            // (claude/codex/cursor-agent/opencode all accept one) unless the
            // profile places it explicitly with a {{prompt}} token. The default
            // flow passes "" and behaves exactly as before: the user types into
            // the live terminal.
            if !spec.prompt.trim().is_empty() && !profile.args.iter().any(|a| a.contains("{{prompt}}")) {
                args.push(spec.prompt.to_string());
            }
            let (command, args) =
                agency_core::scripts::wrap_setup(config.scripts.setup.as_deref(), &profile.command, &args);
            if let Err(e) = self.term.read().unwrap().start_session(
                &session_name(&id),
                &worktree.path,
                &command,
                &args,
                &env,
                220,
                50,
            ) {
                // Roll back the worktree + branch we just cut: the run record is
                // inserted below, so on a spawn failure nothing references them —
                // leaving them would orphan a worktree/branch on every failure.
                let _ = manager.remove(&id);
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
            base: spec.base.to_string(),
            branch: worktree.branch.clone(),
            created_at: now_secs(),
            port_base: Some(port),
            archived_at: None,
            title,
            kind: "agent".to_string(),
            merge_target: spec.merge_target.map(|s| s.to_string()),
            race_id: spec.race_id.clone(),
            loop_state: spec
                .loop_config
                .as_ref()
                .map(|_| agency_core::loops::LoopState::new(now_secs())),
            loop_config: spec.loop_config.clone(),
            issue_id: spec.issue_id.clone(),
        };
        {
            let reg = self.registry.lock().unwrap();
            reg.insert_run(&run)?;
            // Remember the agent type so new-task shortcuts default to what
            // this project actually uses. Best-effort bookkeeping.
            let _ = reg.set_project_default_agent(spec.project_id, spec.agent);
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
        base: &str,
        merge_target: Option<&str>,
    ) -> Result<Vec<RunInfo>> {
        self.create_race_inner(project_id, prompt, agents, base, merge_target, None, None)
    }

    fn create_race_inner(
        &self,
        project_id: &str,
        prompt: &str,
        agents: &[String],
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
        let race_id = uuid::Uuid::new_v4().to_string();
        let mut out = Vec::new();
        for agent in agents {
            out.push(self.create_run_spec(NewRunSpec {
                project_id,
                prompt,
                agent,
                base,
                merge_target,
                race_id: Some(race_id.clone()),
                title: title.clone(),
                existing_branch: None,
                loop_config: None,
                issue_id: issue_id.clone(),
            }, &mut |_| {})?);
        }
        Ok(out)
    }

    /// Create a looping run: one workspace whose agent is re-invoked headless
    /// (fresh context every attempt, state on disk) until the check command
    /// exits 0 or the attempt cap is spent. The loop driver in the watcher
    /// thread owns the session from here.
    pub fn create_loop(
        &self,
        project_id: &str,
        prompt: &str,
        agent: &str,
        base: &str,
        merge_target: Option<&str>,
        check_command: &str,
        max_attempts: u32,
    ) -> Result<RunInfo> {
        self.create_loop_inner(project_id, prompt, agent, base, merge_target, check_command, max_attempts, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_loop_inner(
        &self,
        project_id: &str,
        prompt: &str,
        agent: &str,
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
        self.create_run_spec(NewRunSpec {
            project_id,
            prompt,
            agent,
            base,
            merge_target,
            race_id: None,
            title,
            existing_branch: None,
            loop_config: Some(cfg),
            issue_id,
        }, &mut |_| {})
    }

    /// The prompt, run title, and default base for dispatching a local issue,
    /// labeled with the project's issue key ("AGE-14 Fix login"). One place
    /// composes these so run, race, and loop dispatch can't drift.
    fn issue_dispatch(&self, issue_id: &str) -> Result<(agency_core::registry::Issue, String, String)> {
        let (issue, key) = {
            let reg = self.registry.lock().unwrap();
            let issue = reg
                .get_issue(issue_id)?
                .ok_or_else(|| anyhow!("unknown issue: {issue_id}"))?;
            let key = reg
                .get_project(&issue.project_id)?
                .and_then(|p| p.issue_key)
                .unwrap_or_else(|| "ISSUE".to_string());
            (issue, key)
        };
        let label = format!("{key}-{}", issue.seq);
        let prompt = if issue.body.trim().is_empty() {
            format!("Work on issue {label}: {title}", title = issue.title)
        } else {
            format!("Work on issue {label}: {title}\n\n{body}", title = issue.title, body = issue.body)
        };
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
        base: Option<&str>,
        merge_target: Option<&str>,
    ) -> Result<RunInfo> {
        let (issue, prompt, title) = self.issue_dispatch(issue_id)?;
        let base = self.issue_base(&issue.project_id, base)?;
        let info = self.create_run_spec(NewRunSpec {
            project_id: &issue.project_id,
            prompt: &prompt,
            agent,
            base: &base,
            merge_target,
            race_id: None,
            title: Some(title),
            existing_branch: None,
            loop_config: None,
            issue_id: Some(issue.id.clone()),
        }, &mut |_| {})?;
        self.registry
            .lock()
            .unwrap()
            .advance_issue_status(&issue.id, IssueStatus::InProgress, now_secs())?;
        Ok(info)
    }

    /// Race several agents on one issue; every attempt links the issue.
    pub fn start_issue_race(
        &self,
        issue_id: &str,
        agents: &[String],
        base: Option<&str>,
        merge_target: Option<&str>,
    ) -> Result<Vec<RunInfo>> {
        let (issue, prompt, title) = self.issue_dispatch(issue_id)?;
        let base = self.issue_base(&issue.project_id, base)?;
        let out = self.create_race_inner(
            &issue.project_id,
            &prompt,
            agents,
            &base,
            merge_target,
            Some(title),
            Some(issue.id.clone()),
        )?;
        self.registry
            .lock()
            .unwrap()
            .advance_issue_status(&issue.id, IssueStatus::InProgress, now_secs())?;
        Ok(out)
    }

    /// Loop an agent on one issue until the check command passes.
    pub fn start_issue_loop(
        &self,
        issue_id: &str,
        agent: &str,
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
            &base,
            merge_target,
            check_command,
            max_attempts,
            Some(title),
            Some(issue.id.clone()),
        )?;
        self.registry
            .lock()
            .unwrap()
            .advance_issue_status(&issue.id, IssueStatus::InProgress, now_secs())?;
        Ok(info)
    }

    pub fn list_issues(&self, project_id: &str) -> Result<Vec<agency_core::registry::Issue>> {
        self.registry.lock().unwrap().list_issues(project_id)
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
        self.registry
            .lock()
            .unwrap()
            .create_issue(project_id, title.trim(), body, status, now_secs())
    }

    pub fn update_issue(
        &self,
        id: &str,
        patch: &agency_core::registry::IssuePatch,
    ) -> Result<agency_core::registry::Issue> {
        if patch.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            bail!("an issue needs a title");
        }
        self.registry
            .lock()
            .unwrap()
            .update_issue(id, patch, now_secs())?
            .ok_or_else(|| anyhow!("unknown issue: {id}"))
    }

    pub fn delete_issue(&self, id: &str) -> Result<()> {
        self.registry.lock().unwrap().delete_issue(id)
    }

    /// The abandonment rule: when a linked run is discarded or archived and
    /// it was the issue's last active run, the issue falls back to `todo`
    /// (only from in_progress/in_review — see rollback_issue_to_todo).
    fn maybe_rollback_issue(&self, issue_id: &str) {
        let reg = self.registry.lock().unwrap();
        match reg.runs_for_issue(issue_id) {
            Ok(runs) if runs.is_empty() => {
                if let Err(e) = reg.rollback_issue_to_todo(issue_id, now_secs()) {
                    log::warn!("rolling back issue {issue_id}: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("checking linked runs of issue {issue_id}: {e}"),
        }
    }

    /// Spawn a workspace for a GitHub issue: the issue becomes the run's
    /// prompt (delivered to the agent at launch) and its title.
    pub fn create_run_from_issue(&self, project_id: &str, number: u64, agent: &str) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let issue = agency_core::gh::GhCli::default().view_issue(&repo, number)?;
        let base = agency_core::merge::detect_base(&repo)?;
        let prompt = format!(
            "Work on GitHub issue #{number}: {title}\n\n{body}\n\nIssue link: {url}",
            title = issue.title,
            body = issue.body,
            url = issue.url,
        );
        self.create_run_spec(NewRunSpec {
            project_id,
            prompt: &prompt,
            agent,
            base: &base,
            merge_target: None,
            race_id: None,
            title: Some(format!("#{number} {}", issue.title)),
            existing_branch: None,
            loop_config: None,
            issue_id: None,
        }, &mut |_| {})
    }

    /// Check an existing PR's head branch out into a workspace for review.
    /// The local branch is fast-forwarded from origin first; a diverged local
    /// branch fails loudly rather than being clobbered.
    pub fn create_run_from_pr(&self, project_id: &str, number: u64, agent: &str) -> Result<RunInfo> {
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
        self.create_run_spec(NewRunSpec {
            project_id,
            prompt: &prompt,
            agent,
            base: &pr.base_ref_name,
            merge_target: Some(&pr.base_ref_name),
            race_id: None,
            title: Some(format!("PR #{number} {}", pr.title)),
            existing_branch: Some(pr.head_ref_name.clone()),
            loop_config: None,
            issue_id: None,
        }, &mut |_| {})
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
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
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
        Ok(KnowledgeConfigDto {
            graph: k.graph,
            serve_installed: first_token_on_path(&serve_effective),
            build_installed: first_token_on_path(&build_effective),
            serve_command: k.serve_command,
            build_command: k.build_command,
            serve_default,
            build_default,
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
            // Build an argv (not a whitespace split): the default serve command
            // embeds the primary repo's absolute graph.json path, which breaks
            // graphify injection for any repo path containing a space.
            let argv = config
                .knowledge
                .serve_command
                .as_deref()
                .map(agency_core::config::split_command)
                .unwrap_or_else(|| agency_core::config::default_serve_argv(repo));
            let mut parts = argv.into_iter();
            if let Some(cmd) = parts.next() {
                if command_on_path(&cmd) {
                    auto.push(agency_core::mcp::McpServer {
                        name: "graphify".to_string(),
                        command: Some(cmd),
                        args: parts.collect(),
                        ..Default::default()
                    });
                } else {
                    log::warn!("knowledge graph enabled but '{cmd}' is not installed; skipping MCP injection");
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
        if let Err(e) = agency_core::mcp::emit_for_agent(agent, worktree, &servers) {
            log::warn!("emitting MCP config for {agent} into {}: {e}", worktree.display());
        }
    }

    /// After a clean merge, rebuild the project's knowledge graph in the
    /// background so the next agent workspace starts with a fresh graph.
    fn maybe_rebuild_knowledge_graph(&self, repo: &Path) {
        let config = agency_core::config::load(repo);
        if !config.knowledge.graph {
            return;
        }
        let build = config
            .knowledge
            .build_command
            .clone()
            .unwrap_or_else(|| agency_core::config::default_build_command().to_string());
        if let Some(cmd) = build.split_whitespace().next() {
            if !command_on_path(cmd) {
                log::warn!("knowledge graph enabled but '{cmd}' is not installed; skipping rebuild");
                return;
            }
        }
        log::info!("rebuilding knowledge graph after merge: {build}");
        let spawned = std::process::Command::new("sh")
            .args(["-lc", &build])
            .current_dir(repo)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        // Reap the child on its own thread — dropping it without waiting leaves a
        // zombie `sh` per merge until the app exits.
        if let Ok(mut child) = spawned {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    /// Best-effort `git fetch` for a project's `origin`, so ahead/behind stops
    /// going stale. Returns `Ok(false)` when the project has no remote (nothing
    /// to fetch). Runs no git under any lock — the network call happens after
    /// the repo path is resolved and the registry lock released.
    pub fn fetch_project(&self, project_id: &str) -> Result<bool> {
        let repo = self.project_repo(project_id)?;
        if !agency_core::git::has_origin(&repo) {
            return Ok(false);
        }
        agency_core::git::fetch(&repo)?;
        Ok(true)
    }

    pub fn list_project_branches(&self, project_id: &str) -> Result<agency_core::git::ProjectBranches> {
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
        self.term
            .read()
            .unwrap()
            .start_session(&session_name(&id), &repo, &shell, &args, &[], 220, 50)?;

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
        };
        self.registry.lock().unwrap().insert_run(&run)?;
        Ok(self.run_info(&run))
    }

    pub fn list_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_runs(project_id)?;
        Ok(runs.iter().map(|r| self.run_info(r)).collect())
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
                let status =
                    self.term.read().unwrap().status(&session_name(&run.id)).unwrap_or(SessionStatus::Gone);
                let name = run
                    .title
                    .clone()
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| if run.prompt.is_empty() { run.branch.clone() } else { run.prompt.clone() });
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
    pub fn create_install_terminal(&self, project_id: &str, agent: &str, command: &str) -> Result<RunInfo> {
        let script = format!("{command}\nexec \"$SHELL\" -l");
        self.spawn_terminal(project_id, &format!("install {agent}"), script)
    }

    /// Register a remote MCP `server` with Claude at user scope, then open a
    /// terminal so the user can complete the interactive OAuth handshake (which
    /// needs a browser Agency can't drive headlessly). Registering at user scope
    /// means the session persists across every worktree, so the server is flagged
    /// `user_scope` and Agency stops emitting a project-scoped copy of it.
    ///
    /// `claude mcp add` is run synchronously and its exit status checked *before*
    /// flagging: a failed registration (e.g. the Claude CLI isn't installed) must
    /// not silently flag the server, which would remove it from every worktree
    /// while never actually registering it. Only Claude is supported today.
    pub fn authenticate_mcp_server(&self, project_id: &str, agent: &str, name: &str) -> Result<RunInfo> {
        if agent != "claude" {
            bail!("Authenticate currently supports Claude only; add the server to {agent} via its own CLI");
        }
        let repo = self.project_repo(project_id)?;
        let mut servers = self.list_mcp_servers()?;
        let idx = servers
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| anyhow!("unknown MCP server: {name}"))?;
        let server = servers[idx].clone();
        let url = server
            .url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| anyhow!("Authenticate is for remote (url) servers; '{name}' is a stdio server"))?;

        // Register at user scope unless it already is — `claude mcp add` errors on
        // a duplicate name, and re-flagging an already-flagged server is a no-op.
        if !server.user_scope {
            let transport = match server.effective_transport() {
                agency_core::mcp::McpTransport::Sse => "sse",
                _ => "http",
            };
            // Pass argv directly (no shell) so user-supplied values need no quoting.
            let mut args: Vec<String> = vec![
                "mcp".into(), "add".into(),
                "--scope".into(), "user".into(),
                "--transport".into(), transport.into(),
                name.into(), url.clone(),
            ];
            for (k, v) in &server.headers {
                args.push("--header".into());
                args.push(format!("{k}: {v}"));
            }
            let out = std::process::Command::new("claude")
                .args(&args)
                .current_dir(&repo)
                .output()
                .map_err(|e| anyhow!("running `claude mcp add` (is the Claude CLI installed and on PATH?): {e}"))?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                bail!("`claude mcp add` failed: {}", stderr.trim());
            }
            // Registration succeeded — now it's safe to flag.
            servers[idx].user_scope = true;
            self.save_mcp_servers(&servers)?;
        }

        // Drop the user into a terminal to finish the interactive OAuth step.
        let hint = format!(
            "echo; echo 'Registered \"{name}\" with Claude (user scope). To finish OAuth: run  claude  then  /mcp  and choose Authenticate.'; echo"
        );
        let script = format!("{hint}\nexec \"$SHELL\" -l");
        self.spawn_terminal(project_id, &format!("authenticate {name}"), script)
    }

    /// Clear a server's `user_scope` flag so Agency resumes emitting it into
    /// per-worktree config. The recovery path when a registration failed (or the
    /// user wants Agency to manage the server again); the Claude user-scope
    /// registration, if any, is left in place — remove it with `claude mcp remove`.
    pub fn deauthenticate_mcp_server(&self, name: &str) -> Result<()> {
        let mut servers = self.list_mcp_servers()?;
        let s = servers
            .iter_mut()
            .find(|s| s.name == name)
            .ok_or_else(|| anyhow!("unknown MCP server: {name}"))?;
        if s.user_scope {
            s.user_scope = false;
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
        self.term
            .read()
            .unwrap()
            .start_session(&session_name(&id), &repo, &shell, &args, &env, 220, 50)?;

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
        self.attaches.lock().unwrap().remove(id);
        self.input_seen.lock().unwrap().remove(id);
        self.prompted.lock().unwrap().remove(id);
        let run = self.run_record(id)?;
        // End an active loop first (best-effort): once delete_run removes the
        // row, nothing could ever stop a session the driver respawned into
        // the deleted worktree.
        if has_active_loop(&run) {
            if let Err(e) = self.stop_loop(id) {
                log::warn!("discard_run {id}: couldn't end loop: {e}");
            }
        }
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.run_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&run_session_name(id));
        self.shell_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&shell_session_name(id));
        self.kill_extra_sessions(id);
        if run.kind == "agent" {
            if let Ok(repo) = self.project_repo(&run.project_id) {
                let _ = WorktreeManager::new(repo).remove(id);
            }
        }
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
    pub fn archive_run(&self, id: &str) -> Result<()> {
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
        if run.kind == "agent" {
            WorktreeManager::new(repo.clone())
                .commit_all_if_dirty(id, "WIP: uncommitted changes auto-committed by Agency on archive")
                .map_err(|e| anyhow!("couldn't preserve uncommitted changes before archiving: {e}"))?;
        }

        // Stop all sessions and drop attach handles. Extra tabs are purged for
        // good: the worktree they live in is about to disappear.
        self.attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.run_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&run_session_name(id));
        self.shell_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&shell_session_name(id));
        self.kill_extra_sessions(id);
        self.registry.lock().unwrap().delete_run_sessions(id)?;

        // Best-effort archive cleanup script, before the worktree disappears.
        let config = agency_core::config::load(&repo);
        if let Some(script) = config.scripts.archive.as_deref() {
            let worktree = repo.join(".agency").join("worktrees").join(&run.id);
            if worktree.exists() {
                let env = agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base);
                let _ = agency_core::scripts::run_blocking(script, &worktree, &env);
            }
        }

        WorktreeManager::new(repo).remove_keep_branch(id)?;
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
        manager.restore(id)?;
        let config = agency_core::config::load(&repo);
        if let Err(e) = manager.copy_essentials(id, &config.files.copy) {
            log::warn!("copying essentials into restored worktree {id}: {e}");
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
        let worktree = repo.join(".agency").join("worktrees").join(id);
        self.emit_mcp(&run.agent, &repo, &worktree, &config);
        self.registry.lock().unwrap().set_archived(id, None)?;
        let refreshed = self.run_record(id)?;
        Ok(self.run_info(&refreshed))
    }

    pub fn list_archived_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_archived_runs(project_id)?;
        Ok(runs.iter().map(|r| self.run_info(r)).collect())
    }

    pub fn stop_run(&self, id: &str) -> Result<()> {
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
        self.run_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&run_session_name(id));
        Ok(())
    }

    pub fn run_script_configured(&self, id: &str) -> Result<bool> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        Ok(agency_core::config::load(&repo).scripts.run.is_some())
    }

    pub fn start_run_script(&self, id: &str) -> Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let config = agency_core::config::load(&repo);
        let run_cmd = config
            .scripts
            .run
            .clone()
            .ok_or_else(|| anyhow!("no run script configured in .agency/agency.toml"))?;
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);

        // nonconcurrent: stop every other run-script session first.
        if config.scripts.run_mode == agency_core::config::RunMode::Nonconcurrent {
            let others = self.registry.lock().unwrap().list_runs(&run.project_id)?;
            for other in others {
                if other.id != run.id {
                    self.run_attaches.lock().unwrap().remove(&other.id);
                    let _ = self.term.read().unwrap().kill(&run_session_name(&other.id));
                }
            }
        }

        let mut env = self.provider_env()?;
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));

        // Restart cleanly if a previous run session is still around.
        let _ = self.term.read().unwrap().kill(&run_session_name(id));
        self.term.read().unwrap().start_session(
            &run_session_name(id),
            &worktree,
            "sh",
            &["-lc".to_string(), run_cmd],
            &env,
            220,
            50,
        )
    }

    pub fn stop_run_script(&self, id: &str) -> Result<()> {
        self.run_attaches.lock().unwrap().remove(id);
        let _ = self.term.read().unwrap().kill(&run_session_name(id));
        Ok(())
    }

    pub fn run_script_status(&self, id: &str) -> Result<SessionStatus> {
        Ok(self
            .term
            .read()
            .unwrap()
            .status(&run_session_name(id))
            .unwrap_or(SessionStatus::Gone))
    }

    pub fn run_script_preview(&self, id: &str, lines: usize) -> Result<String> {
        Ok(self.term.read().unwrap().capture(&run_session_name(id), lines).unwrap_or_default())
    }

    pub fn attach_run_script<F>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<()>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        let sub = self.term.read().unwrap().subscribe(&run_session_name(id), cols, rows, on_output)?;
        self.run_attaches.lock().unwrap().insert(id.to_string(), sub);
        Ok(())
    }

    pub fn detach_run_script(&self, id: &str) {
        // Dropping the Subscription sends Unsubscribe; the run-script session keeps
        // running server-side. See detach_run.
        self.run_attaches.lock().unwrap().remove(id);
    }

    pub fn run_script_input(&self, id: &str, data: &[u8]) -> Result<()> {
        self.term.read().unwrap().input(&run_session_name(id), data)
    }

    pub fn resize_run_script(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        self.term.read().unwrap().resize(&run_session_name(id), cols, rows)
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
        // Agents run in their worktree; terminals map to the repo root.
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
        Ok(self
            .term
            .read()
            .unwrap()
            .status(&shell_session_name(id))
            .unwrap_or(SessionStatus::Gone))
    }

    pub fn shell_preview(&self, id: &str, lines: usize) -> Result<String> {
        Ok(self.term.read().unwrap().capture(&shell_session_name(id), lines).unwrap_or_default())
    }

    pub fn attach_shell<F>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<()>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        let sub = self.term.read().unwrap().subscribe(&shell_session_name(id), cols, rows, on_output)?;
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
    /// worktree. Always a fresh, promptless launch — resume recipes pick the
    /// cwd's most recent conversation, which in a shared worktree may belong
    /// to a sibling tab, so extras never resume.
    fn launch_run_session(&self, sid: &str, run: &agency_core::registry::Run, agent: &str) -> Result<()> {
        let repo = self.project_repo(&run.project_id)?;
        let config = agency_core::config::load(&repo);
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
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
            reg.get_profile(agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {agent}"))?
        };
        // The extra tab may run a different agent than the one the worktree
        // was created for; make sure MCP config exists in its native format.
        self.emit_mcp(agent, &repo, &worktree, &config);
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        // Same env recipe as the run itself, ports included: extra sessions
        // are collaborators in the same workspace, not new workspaces.
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
        let (command, args) = agent_argv(&profile, "", false, config.scripts.setup.as_deref());
        self.term
            .read()
            .unwrap()
            .start_session(&session_name(sid), &worktree, &command, &args, &env, 220, 50)
    }

    /// Open an additional agent tab in an existing run's worktree. `agent`
    /// defaults to the run's own agent profile.
    pub fn start_run_session(&self, run_id: &str, agent: Option<&str>) -> Result<RunSessionInfo> {
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
        self.launch_run_session(&session.id, &run, &session.agent)?;
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
            return self.launch_run_session(id, &run, &session.agent);
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
                &session_name(id), &repo, &shell, &["-l".to_string()], &[], 220, 50,
            )?;
            return Ok(());
        }

        let config = agency_core::config::load(&repo);
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?
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
            .map(|h| crate::resume_probe::resume_probe(std::path::Path::new(&h), &profile.command, &worktree))
            .unwrap_or(crate::resume_probe::ResumeProbe::Unknown);
        let use_resume =
            profile.resume_args.is_some() && probe != crate::resume_probe::ResumeProbe::None;
        let (command, args) = agent_argv(&profile, &run.prompt, use_resume, setup);
        let fallback = if use_resume {
            let (fresh_cmd, fresh_args) = agent_argv(&profile, &run.prompt, false, setup);
            Some(agency_core::term::protocol::FallbackSpec {
                command: fresh_cmd,
                args: fresh_args,
                grace_ms: 3000,
            })
        } else {
            None
        };
        self.term.read().unwrap().start_session_with_fallback(
            &session_name(id), &worktree, &command, &args, &env, 220, 50, fallback,
        )?;
        Ok(())
    }

    pub fn rerun(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
        if has_active_loop(&run) {
            bail!("this run is looping — stop the loop before rerunning it manually");
        }
        let repo = self.project_repo(&run.project_id)?;
        let config = agency_core::config::load(&repo);
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?
        };
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
        let args = profile.render_args(&run.prompt);
        let (command, args) =
            agency_core::scripts::wrap_setup(config.scripts.setup.as_deref(), &profile.command, &args);
        let _ = self.term.read().unwrap().kill(&session_name(id));
        self.term.read().unwrap().start_session(&session_name(id), &worktree, &command, &args, &env, 220, 50)?;
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
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
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
            reg.get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?
        };
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
        let (command, args) = loop_argv(&profile, &run.prompt, config.scripts.setup.as_deref())?;
        let _ = self.term.read().unwrap().kill(&session_name(&run.id));
        self.term
            .read()
            .unwrap()
            .start_session(&session_name(&run.id), &worktree, &command, &args, &env, 220, 50)
    }

    /// Run the loop's check command in the worktree on its own thread; the
    /// result lands in the run's check slot for `drive_loops` to drain. The
    /// child is killed at the config's timeout (counted as a failed check).
    fn start_loop_check(&self, run: &agency_core::registry::Run, cfg: &agency_core::loops::LoopConfig) -> Result<()> {
        let repo = self.project_repo(&run.project_id)?;
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
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
                let (Some(cfg), Some(prev)) = (run.loop_config.clone(), run.loop_state.clone()) else {
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
                    run.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| run.prompt.clone())
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
    /// For agents this is the run's isolated worktree
    /// (`<repo>/.agency/worktrees/<id>`). For terminals — which have no worktree
    /// and run the user's shell in the project's main checkout — it is the
    /// project repo root, so Source Control / Files act on the live branch.
    pub fn worktree_path(&self, id: &str) -> Result<std::path::PathBuf> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        if run.kind == "terminal" {
            return Ok(repo);
        }
        Ok(repo.join(".agency").join("worktrees").join(id))
    }

    /// Resolve a git-root token to a working directory. A `project:<id>` token
    /// targets the project's main checkout (its active branch); any other token
    /// is a run id resolved via [`worktree_path`] (terminals already map to the
    /// repo root). Lets the git commands operate at project level, not just per
    /// run, without changing their IPC signatures.
    pub fn git_root(&self, token: &str) -> Result<std::path::PathBuf> {
        if let Some(pid) = token.strip_prefix("project:") {
            return self.project_repo_path(pid);
        }
        self.worktree_path(token)
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
        let branch = agency_core::git::branch_info(&wt)
            .map(|b| b.branch)
            .unwrap_or_default();
        Ok((branch, base))
    }

    // ── merge operations ───────────────────────────────────────────────────────

    /// Inspect what merging this run's branch would do, without touching the
    /// repo: which base it targets, how many commits the branch is ahead, and
    /// whether the agent's worktree still has uncommitted changes.
    pub fn merge_preview(&self, id: &str) -> anyhow::Result<MergePreview> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
        let commits_ahead = agency_core::merge::commits_ahead(&repo, &run.branch, &base)?;
        let commits_behind = agency_core::merge::commits_behind(&repo, &run.branch, &base)?;
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
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
        // try_lock, not lock: a second merge racing the first should fail
        // fast with a clear message, not queue up and re-merge afterwards.
        let _gate = self
            .merge_gate
            .try_lock()
            .map_err(|_| anyhow!("another merge is already in progress"))?;
        let run = self.run_record(id)?;
        // An active loop is still committing attempts onto this branch; merging
        // mid-flight would take a half-done attempt and keep drifting after.
        if has_active_loop(&run) {
            bail!("this run is looping — stop the loop before merging");
        }
        let repo = self.project_repo(&run.project_id)?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
        let outcome = agency_core::merge::merge(&repo, &run.branch, &base)?;
        if matches!(outcome, agency_core::merge::MergeOutcome::Clean { .. }) {
            self.maybe_rebuild_knowledge_graph(&repo);
            // A merged run completes its issue. Best-effort: the merge itself
            // already succeeded and must not report failure.
            if let Some(issue_id) = &run.issue_id {
                if let Err(e) = self
                    .registry
                    .lock()
                    .unwrap()
                    .advance_issue_status(issue_id, IssueStatus::Done, now_secs())
                {
                    log::warn!("closing issue {issue_id} after merge: {e}");
                }
            }
        }
        Ok(outcome)
    }

    pub fn abort_merge_task(&self, id: &str) -> anyhow::Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        agency_core::merge::abort_merge(&repo)
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
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
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
            if let Err(e) = self
                .registry
                .lock()
                .unwrap()
                .advance_issue_status(issue_id, IssueStatus::InReview, now_secs())
            {
                log::warn!("advancing issue {issue_id} to in_review: {e}");
            }
        }
        Ok(pr)
    }

    /// The run's PR (if any) plus its check rollup, polled by the UI.
    pub fn pr_status(&self, id: &str) -> Result<PrStatus> {
        let run = self.run_record(id)?;
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
    pub fn pr_detail(&self, project_id: &str, number: u64) -> Result<Option<agency_core::gh::PrDetail>> {
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
    pub fn merge_pr(&self, project_id: &str, number: u64, method: &str, delete_branch: bool) -> Result<()> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().merge_pr(&repo, number, method, delete_branch)
    }

    /// The PR's full multi-file diff, split per file for the diff renderer.
    pub fn pr_diff(&self, project_id: &str, number: u64) -> Result<Vec<agency_core::gh::PrFileDiff>> {
        let repo = self.project_repo(project_id)?;
        agency_core::gh::GhCli::default().pr_diff(&repo, number)
    }

    /// The PR's review threads (with resolution state) for inline rendering.
    pub fn pr_review_threads(&self, project_id: &str, number: u64) -> Result<Vec<agency_core::gh::ReviewThread>> {
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
        let detail = gh
            .view_pr_detail(&repo, number)?
            .ok_or_else(|| anyhow!("PR #{number} not found"))?;
        gh.submit_pr_review(&repo, number, &detail.head_ref_oid, event, body, comments)
    }

    /// Reply into an existing review thread. `in_reply_to` is a comment's
    /// `database_id` from `pr_review_threads`.
    pub fn reply_pr_comment(&self, project_id: &str, number: u64, in_reply_to: u64, body: &str) -> Result<()> {
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
        agency_core::git::push_branch(&repo, head)?;
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
        let repo = self.project_repo(&run.project_id)?;
        Ok(agency_core::gh::GhCli::default()
            .view_pr(&repo, &run.branch)?
            .map(|p| p.number))
    }

    /// Type the PR's failing checks into the agent's live session so it can
    /// investigate — same delivery path as review comments.
    pub fn send_check_feedback(&self, id: &str) -> Result<()> {
        let status = self.pr_status(id)?;
        let failing: Vec<_> = status
            .checks
            .iter()
            .filter(|c| c.bucket == "fail" || c.bucket == "cancel")
            .collect();
        if failing.is_empty() {
            bail!("no failing checks to send");
        }
        if !matches!(self.term.read().unwrap().status(&session_name(id)), Ok(SessionStatus::Running)) {
            bail!("agent session {id} is not running");
        }
        let mut msg = format!("CI feedback: {} check(s) failing on this branch's PR — ", failing.len());
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
        msg.push_str(". Please investigate the failures, fix them, commit, and push to update the PR.");
        self.term.read().unwrap().send_text(&session_name(id), &msg)?;
        Ok(())
    }

    pub fn resolve_merge<F>(
        &self,
        id: &str,
        resolver_profile: &str,
        on_output: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)
            .unwrap_or_else(|_| "main".to_string());
        let branch = run.branch.clone();
        let conflicts = agency_core::git::status(&repo)
            .map(|cs| {
                cs.into_iter()
                    .filter(|c| c.index == "U" || c.worktree == "U")
                    .map(|c| c.path)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let prompt = format!(
            "{skill}\n\n## This merge\n- Base branch: {base}\n- Feature branch: {branch}\n- Conflicted files: {files}\n",
            skill = MERGE_RESOLVER_SKILL,
            files = if conflicts.is_empty() { "(detect with `git status`)".to_string() } else { conflicts.join(", ") },
        );

        let mut profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(resolver_profile)?
                .ok_or_else(|| anyhow!("unknown resolver profile: {resolver_profile}"))?
        };

        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        profile.env = env;

        let handle = agency_core::supervisor::spawn_agent(&profile, &repo, &prompt, on_output)?;
        self.resolvers.lock().unwrap().insert(id.to_string(), handle);
        Ok(())
    }

    /// Tear down a merge resolver: dropping its handle kills the agent child
    /// (see `AgentHandle`'s Drop) and frees the map slot. Called when the merge
    /// modal closes or the resolver exits; a no-op if none is running, so it is
    /// safe to call unconditionally.
    pub fn resolver_close(&self, id: &str) {
        self.resolvers.lock().unwrap().remove(id);
    }

    pub fn resolver_input(&self, id: &str, data: &[u8]) -> anyhow::Result<()> {
        let resolvers = self.resolvers.lock().unwrap();
        let handle = resolvers
            .get(id)
            .ok_or_else(|| anyhow!("no resolver for task: {id}"))?;
        handle.write_input(data)
    }

    pub fn resolver_status(&self, id: &str) -> anyhow::Result<agency_core::supervisor::AgentStatus> {
        let resolvers = self.resolvers.lock().unwrap();
        let handle = resolvers
            .get(id)
            .ok_or_else(|| anyhow!("no resolver for task: {id}"))?;
        Ok(handle.status())
    }

    /// Resize the resolver PTY so its agent reflows to the visible terminal.
    /// A no-op when no resolver is running (resize events can arrive before the
    /// resolver is spawned or after it has exited).
    pub fn resolver_resize(&self, id: &str, cols: u16, rows: u16) -> anyhow::Result<()> {
        let resolvers = self.resolvers.lock().unwrap();
        if let Some(handle) = resolvers.get(id) {
            handle.resize(rows, cols)?;
        }
        Ok(())
    }

    /// Update focus/active-run state. On an unfocused→focused edge, hand back a
    /// still-fresh pending notification target (consuming it) so the caller can
    /// deep-link the UI to the run the user was just notified about.
    pub fn set_ui_state(&self, focused: bool, active_run: Option<String>) -> Option<(String, String)> {
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

    pub fn list_review_comments(&self, run_id: &str) -> Result<Vec<agency_core::registry::ReviewComment>> {
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
        if !matches!(self.term.read().unwrap().status(&session_name(run_id)), Ok(SessionStatus::Running)) {
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
        let projects = self.registry.lock().unwrap().list_projects()?;
        let mut out = Vec::new();
        for proj in projects {
            let runs = self.registry.lock().unwrap().list_runs(&proj.id)?;
            for run in runs {
                let agent = self.term.read().unwrap().status(&session_name(&run.id)).unwrap_or(SessionStatus::Gone);
                let run_script = self.term.read().unwrap().status(&run_session_name(&run.id)).unwrap_or(SessionStatus::Gone);
                let pane = self.term.read().unwrap().capture(&session_name(&run.id), 50).unwrap_or_default();
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&pane, &mut hasher);
                let pane_hash = std::hash::Hasher::finish(&hasher);
                let label = format!(
                    "{}: {}",
                    run.agent,
                    run.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| {
                        if run.prompt.is_empty() { run.branch.clone() } else { run.prompt.clone() }
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
                    run_script,
                    pane_hash,
                    user_input_pending,
                });
            }
        }
        Ok(out)
    }
}

/// True when `command` resolves to an executable file: checked directly when it
/// contains a path separator, otherwise searched across the PATH directories.
/// Whether the first whitespace token of a command line resolves to an
/// executable on PATH (or a runnable absolute/relative path). Command overrides
/// and the graphify defaults are full command lines, not bare binaries.
fn first_token_on_path(command_line: &str) -> bool {
    command_line
        .split_whitespace()
        .next()
        .map(command_on_path)
        .unwrap_or(false)
}

fn command_on_path(command: &str) -> bool {
    fn executable(p: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            p.is_file() && p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
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

/// A project path is addable as long as it is a git repository. A repo with no
/// commits is allowed (it lands "gated": the UI walks the user through the
/// first commit before any agent can spawn). Non-repos are rejected because the
/// UI runs `init_repo` *before* calling `add_project`.
fn validate_repo(repo_path: &Path) -> Result<()> {
    if !repo_path.exists() {
        bail!("{} does not exist", repo_path.display());
    }
    if matches!(
        agency_core::setup::repo_readiness(repo_path),
        agency_core::setup::RepoReadiness::NotARepo
    ) {
        bail!("{} is not a git repository", repo_path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{command_on_path, new_task_id, slugify, split_session_id, pick_port, agent_argv};
    use agency_core::profile::AgentProfile;
    use std::collections::HashSet;

    #[test]
    fn command_on_path_finds_shell_binaries() {
        assert!(command_on_path("sh"), "sh must be on PATH");
        assert!(command_on_path("/bin/sh"), "absolute path to sh");
        assert!(!command_on_path("definitely-not-a-real-binary-4k2x"));
        assert!(!command_on_path("/nonexistent/path/to/agent"));
    }

    #[test]
    fn agent_argv_uses_resume_args_when_available() {
        let p = AgentProfile {
            name: "claude".into(), command: "claude".into(),
            args: vec!["{{prompt}}".into()], env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let (cmd, args) = agent_argv(&p, "do the thing", true, None);
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["--continue".to_string()]);
    }

    #[test]
    fn agent_argv_falls_back_to_prompt_without_resume_args() {
        let p = AgentProfile {
            name: "cursor".into(), command: "cursor-agent".into(),
            args: vec!["{{prompt}}".into()], env: vec![],
            resume_args: None,
            loop_args: None,
        };
        let (cmd, args) = agent_argv(&p, "hello", true, None);
        assert_eq!(cmd, "cursor-agent");
        assert_eq!(args, vec!["hello".to_string()]);
    }

    #[test]
    fn agent_argv_fresh_ignores_resume_args() {
        let p = AgentProfile {
            name: "claude".into(), command: "claude".into(),
            args: vec!["{{prompt}}".into()], env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        };
        let (_cmd, args) = agent_argv(&p, "fresh prompt", false, None);
        assert_eq!(args, vec!["fresh prompt".to_string()]);
    }

    #[test]
    fn loop_argv_renders_prompt_and_requires_recipe() {
        let p = AgentProfile {
            name: "claude".into(), command: "claude".into(),
            args: vec![], env: vec![],
            resume_args: None,
            loop_args: Some(vec!["-p".into(), "{{prompt}}".into(), "--permission-mode".into(), "acceptEdits".into()]),
        };
        let (cmd, args) = super::loop_argv(&p, "fix the tests", None).unwrap();
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["-p", "fix the tests", "--permission-mode", "acceptEdits"]);

        let no_recipe = AgentProfile { loop_args: None, ..p.clone() };
        assert!(super::loop_argv(&no_recipe, "x", None).is_err());

        // An empty recipe is no recipe — Settings saves None for an empty
        // field, but a hand-edited profile must not slip through.
        let empty_recipe = AgentProfile { loop_args: Some(vec![]), ..p };
        assert!(super::loop_argv(&empty_recipe, "x", None).is_err());
    }

    #[test]
    fn loop_argv_appends_prompt_when_recipe_has_no_token() {
        // A recipe without {{prompt}} must still deliver the prompt (as the
        // final positional arg, like the interactive path) — never launch
        // promptless attempts that burn the loop budget doing nothing.
        let p = AgentProfile {
            name: "codex".into(), command: "codex".into(),
            args: vec![], env: vec![],
            resume_args: None,
            loop_args: Some(vec!["exec".into(), "--full-auto".into()]),
        };
        let (cmd, args) = super::loop_argv(&p, "fix the tests", None).unwrap();
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
    fn run_session_name_is_namespaced() {
        assert_eq!(super::run_session_name("fix-login-a3k2"), "agency-run-fix-login-a3k2");
    }

    use super::compose_feedback;
    use agency_core::registry::ReviewComment;

    fn rc(path: &str, a: u32, b: u32, body: &str) -> ReviewComment {
        ReviewComment {
            id: "x".into(), run_id: "r".into(), path: path.into(),
            line_start: a, line_end: b, body: body.into(), sent: false, created_at: 0,
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
        };
        assert_eq!(run.kind, "terminal");
        assert!(run.branch.is_empty());
        assert!(run.port_base.is_none());
        assert!(run.id.starts_with("terminal-"));
    }
}
