use agency_core::profile::AgentProfile;
use agency_core::registry::{Project, Registry};
use agency_core::supervisor::AgentHandle;
use agency_core::term::client::{Subscription, TermClient};
use agency_core::term::SessionStatus;
use agency_core::worktree::WorktreeManager;
use anyhow::{anyhow, bail, Result};
use crate::notifier;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use uuid;

const SETTING_ANTHROPIC_KEY: &str = "anthropic_api_key";
const SETTING_LM_STUDIO_URL: &str = "lm_studio_base_url";
const DEFAULT_LM_STUDIO_URL: &str = "http://localhost:1234/v1";
const SETTING_NOTIF: &str = "notification_settings";

const MERGE_RESOLVER_SKILL: &str = include_str!("../../../skills/merge-resolver/SKILL.md");

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    pub anthropic_api_key: String,
    pub lm_studio_base_url: String,
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
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
    pub port: Option<u16>,
    pub kind: String,
}

/// What an "Approve & merge" would do, computed before running it so the UI can
/// explain the outcome instead of silently merging. `commits_ahead == 0` means
/// the branch has no new commits (merge is a no-op); `worktree_dirty` flags
/// uncommitted agent work that a branch merge would leave behind.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePreview {
    pub base: String,
    pub branch: String,
    pub commits_ahead: usize,
    pub worktree_dirty: bool,
    pub dirty_files: Vec<String>,
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

fn run_session_name(id: &str) -> String {
    format!("agency-run-{id}")
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
/// The id doubles as the worktree dir, branch (`agent/<id>`) and tmux session
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
/// shell exits 0 on a stray EOF that nothing in tmux/the backend is known to send.
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

fn agent_profile(name: &str, command: &str) -> AgentProfile {
    AgentProfile { name: name.into(), command: command.into(), args: vec![], env: vec![] }
}

#[derive(Default)]
struct UiState {
    focused: bool,
    active_run: Option<String>,
}

pub struct AppState {
    registry: Mutex<Registry>,
    attaches: Mutex<HashMap<String, Subscription>>,
    run_attaches: Mutex<HashMap<String, Subscription>>,
    term: TermClient,
    resolvers: Mutex<HashMap<String, AgentHandle>>,
    ui: Mutex<UiState>,
    /// Run ids that have received user input since their last "waiting for input"
    /// notification. Drives idle-notification gating (see `notifier::step`).
    input_seen: Mutex<HashSet<String>>,
}

impl AppState {
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn new(db_path: &Path, data_dir: &Path) -> Result<AppState> {
        let registry = Registry::open(db_path)?;
        // Seed the built-in shell profile once.
        if registry.get_profile("shell")?.is_none() {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            registry.upsert_profile(&AgentProfile {
                name: "shell".to_string(),
                command: shell,
                args: vec!["-l".to_string()],
                env: vec![],
            })?;
        }
        // Ensure built-in agent profiles exist (added for existing DBs too).
        for (name, command) in [("claude", "claude"), ("pi", "pi"), ("hermes", "hermes")] {
            if registry.get_profile(name)?.is_none() {
                registry.upsert_profile(&agent_profile(name, command))?;
            }
        }
        let state = AppState {
            registry: Mutex::new(registry),
            attaches: Mutex::new(HashMap::new()),
            run_attaches: Mutex::new(HashMap::new()),
            term: TermClient::connect_or_spawn(termd_socket(data_dir), termd_bin())?,
            resolvers: Mutex::new(HashMap::new()),
            ui: Mutex::new(UiState { focused: true, active_run: None }),
            input_seen: Mutex::new(HashSet::new()),
        };
        // Rehydrate: any run the daemon still hosts is adopted as-is; the watch
        // loop (watch_snapshot) then reports live status. Nothing to spawn here —
        // surviving sessions are already running in the daemon.
        if let Ok(sessions) = state.term.list() {
            log::info!("termd: adopted {} surviving session(s)", sessions.len());
        }
        Ok(state)
    }

    fn provider_env(&self) -> Result<Vec<(String, String)>> {
        let s = self.get_settings()?;
        let mut env = Vec::new();
        if !s.anthropic_api_key.is_empty() {
            env.push(("ANTHROPIC_API_KEY".into(), s.anthropic_api_key));
        }
        env.push(("OPENAI_BASE_URL".into(), s.lm_studio_base_url));
        env.push(("OPENAI_API_KEY".into(), "lm-studio".into()));
        Ok(env)
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

    pub fn get_settings(&self) -> Result<ProviderSettings> {
        let reg = self.registry.lock().unwrap();
        Ok(ProviderSettings {
            anthropic_api_key: reg.get_setting(SETTING_ANTHROPIC_KEY)?.unwrap_or_default(),
            lm_studio_base_url: reg
                .get_setting(SETTING_LM_STUDIO_URL)?
                .unwrap_or_else(|| DEFAULT_LM_STUDIO_URL.to_string()),
        })
    }

    pub fn save_settings(&self, s: &ProviderSettings) -> Result<()> {
        validate_provider_url(&s.lm_studio_base_url)?;
        let reg = self.registry.lock().unwrap();
        reg.set_setting(SETTING_ANTHROPIC_KEY, &s.anthropic_api_key)?;
        reg.set_setting(SETTING_LM_STUDIO_URL, &s.lm_studio_base_url)?;
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

    pub fn commit_repo(&self, repo_path: &Path, add_gitignore: bool) -> Result<()> {
        agency_core::setup::initial_commit(repo_path, add_gitignore)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.registry.lock().unwrap().list_projects()
    }

    pub fn close_project(&self, id: &str) -> Result<()> {
        // Kill live terminals; keep project + run records so reopen can re-run.
        let runs = self.registry.lock().unwrap().list_runs(id)?;
        for run in &runs {
            self.attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.kill(&session_name(&run.id));
            self.run_attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.kill(&run_session_name(&run.id));
        }
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        let runs = self.registry.lock().unwrap().list_runs(id)?;
        let repo = self.project_repo(id).ok();
        for run in &runs {
            self.attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.kill(&session_name(&run.id));
            self.run_attaches.lock().unwrap().remove(&run.id);
            let _ = self.term.kill(&run_session_name(&run.id));
            if let Some(repo) = &repo {
                let _ = WorktreeManager::new(repo.clone()).remove(&run.id);
            }
            self.registry.lock().unwrap().delete_run(&run.id)?;
        }
        self.registry.lock().unwrap().remove_project(id)?;
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
        let status = self.term.status(&name).unwrap_or(SessionStatus::Gone);
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
            added: stat.added,
            deleted: stat.deleted,
            files: stat.files,
            port: run.port_base,
            kind: run.kind.clone(),
        }
    }

    // ── run lifecycle ──────────────────────────────────────────────────────────

    fn allocate_port(&self, base: u16, block_size: u16) -> Result<u16> {
        let used: std::collections::HashSet<u16> =
            self.registry.lock().unwrap().list_port_bases()?.into_iter().collect();
        pick_port(&used, base, block_size).ok_or_else(|| anyhow!("no free port block available"))
    }

    pub fn create_run(&self, project_id: &str, prompt: &str, agent: &str, base: &str, merge_target: Option<&str>) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let config = agency_core::config::load(&repo);
        let port = self.allocate_port(config.ports.base, config.ports.block_size)?;
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {agent}"))?
        };
        let id = new_task_id(prompt);
        let worktree = WorktreeManager::new(repo.clone()).create(&id, base)?;

        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        env.extend(agency_core::scripts::script_env(&worktree.path, &repo, &id, Some(port)));
        let args: Vec<String> = profile
            .render_args(prompt)
            .into_iter()
            .filter(|a| !a.is_empty())
            .collect();
        let (command, args) =
            agency_core::scripts::wrap_setup(config.scripts.setup.as_deref(), &profile.command, &args);

        self.term
            .start_session(&session_name(&id), &worktree.path, &command, &args, &env, 220, 50)?;

        let run = agency_core::registry::Run {
            id: id.clone(),
            project_id: project_id.to_string(),
            agent: agent.to_string(),
            prompt: prompt.to_string(),
            base: base.to_string(),
            branch: worktree.branch.clone(),
            created_at: now_secs(),
            port_base: Some(port),
            archived_at: None,
            title: None,
            kind: "agent".to_string(),
            merge_target: merge_target.map(|s| s.to_string()),
        };
        self.registry.lock().unwrap().insert_run(&run)?;
        Ok(self.run_info(&run))
    }

    pub fn list_project_branches(&self, project_id: &str) -> Result<agency_core::git::ProjectBranches> {
        let repo = self.project_repo(project_id)?;
        agency_core::git::list_branches(&repo)
    }

    /// Create a standalone shell terminal session in the project repo root.
    /// Unlike `create_run` it has no worktree, branch, or agent profile — it just
    /// runs the user's login shell, reusing the tmux/attach/resize pipeline.
    pub fn create_terminal(&self, project_id: &str) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let id = new_task_id("terminal");
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        // Login shell so the user's prompt/profile loads.
        let args = vec!["-l".to_string()];
        pty_debug(&format!("create_terminal id={id} shell={shell} args={args:?}"));
        self.term
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
        };
        self.registry.lock().unwrap().insert_run(&run)?;
        Ok(self.run_info(&run))
    }

    pub fn list_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_runs(project_id)?;
        Ok(runs.iter().map(|r| self.run_info(r)).collect())
    }

    pub fn run_status(&self, id: &str) -> Result<SessionStatus> {
        Ok(self.term.status(&session_name(id)).unwrap_or(SessionStatus::Gone))
    }

    pub fn attach_run<F>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<()>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        pty_debug(&format!("attach_run id={id}"));
        let sub = self.term.subscribe(&session_name(id), cols, rows, on_output)?;
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
        self.term.input(&session_name(id), data)?;
        // Arm the idle ("waiting for input") notification for this run: it only
        // fires after the user has driven a turn, and at most once per turn.
        self.input_seen.lock().unwrap().insert(id.to_string());
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

    /// Resize the session's PTY so the emulator reflows to the visible terminal.
    /// Sent straight to the daemon by id; harmless if the session isn't live yet
    /// (resize events can race ahead of the session coming up).
    pub fn resize_run(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        self.term.resize(&session_name(id), cols, rows)
    }

    pub fn run_preview(&self, id: &str, lines: usize) -> Result<String> {
        Ok(self.term.capture(&session_name(id), lines).unwrap_or_default())
    }

    pub fn discard_run(&self, id: &str) -> Result<()> {
        self.attaches.lock().unwrap().remove(id);
        self.input_seen.lock().unwrap().remove(id);
        let run = self.run_record(id)?;
        let _ = self.term.kill(&session_name(id));
        self.run_attaches.lock().unwrap().remove(id);
        let _ = self.term.kill(&run_session_name(id));
        if run.kind == "agent" {
            if let Ok(repo) = self.project_repo(&run.project_id) {
                let _ = WorktreeManager::new(repo).remove(id);
            }
        }
        self.registry.lock().unwrap().delete_run(id)?;
        Ok(())
    }

    /// Archive a run: stop its sessions, run the optional archive cleanup script,
    /// remove the worktree but KEEP the branch, and stamp `archived_at`. The run
    /// record is kept so it can be restored.
    pub fn archive_run(&self, id: &str) -> Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;

        // Stop both sessions and drop attach handles.
        self.attaches.lock().unwrap().remove(id);
        let _ = self.term.kill(&session_name(id));
        self.run_attaches.lock().unwrap().remove(id);
        let _ = self.term.kill(&run_session_name(id));

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
        Ok(())
    }

    /// Restore an archived run: re-create its worktree on the kept branch and
    /// clear `archived_at`. The agent is not auto-started.
    pub fn restore_run(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        WorktreeManager::new(repo).restore(id)?;
        self.registry.lock().unwrap().set_archived(id, None)?;
        let refreshed = self.run_record(id)?;
        Ok(self.run_info(&refreshed))
    }

    pub fn list_archived_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_archived_runs(project_id)?;
        Ok(runs.iter().map(|r| self.run_info(r)).collect())
    }

    pub fn stop_run(&self, id: &str) -> Result<()> {
        self.attaches.lock().unwrap().remove(id);
        let _ = self.term.kill(&session_name(id));
        self.run_attaches.lock().unwrap().remove(id);
        let _ = self.term.kill(&run_session_name(id));
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
                    let _ = self.term.kill(&run_session_name(&other.id));
                }
            }
        }

        let mut env = self.provider_env()?;
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));

        // Restart cleanly if a previous run session is still around.
        let _ = self.term.kill(&run_session_name(id));
        self.term.start_session(
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
        let _ = self.term.kill(&run_session_name(id));
        Ok(())
    }

    pub fn run_script_status(&self, id: &str) -> Result<SessionStatus> {
        Ok(self
            .term
            .status(&run_session_name(id))
            .unwrap_or(SessionStatus::Gone))
    }

    pub fn run_script_preview(&self, id: &str, lines: usize) -> Result<String> {
        Ok(self.term.capture(&run_session_name(id), lines).unwrap_or_default())
    }

    pub fn attach_run_script<F>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<()>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        let sub = self.term.subscribe(&run_session_name(id), cols, rows, on_output)?;
        self.run_attaches.lock().unwrap().insert(id.to_string(), sub);
        Ok(())
    }

    pub fn detach_run_script(&self, id: &str) {
        // Dropping the Subscription sends Unsubscribe; the run-script session keeps
        // running server-side. See detach_run.
        self.run_attaches.lock().unwrap().remove(id);
    }

    pub fn run_script_input(&self, id: &str, data: &[u8]) -> Result<()> {
        self.term.input(&run_session_name(id), data)
    }

    pub fn resize_run_script(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        self.term.resize(&run_session_name(id), cols, rows)
    }

    pub fn rerun(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
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
        let _ = self.term.kill(&session_name(id));
        self.term.start_session(&session_name(id), &worktree, &command, &args, &env, 220, 50)?;
        Ok(self.run_info(&run))
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
            worktree_dirty: !dirty_files.is_empty(),
            dirty_files,
        })
    }

    pub fn merge_task(&self, id: &str) -> anyhow::Result<agency_core::merge::MergeOutcome> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
        agency_core::merge::merge(&repo, &run.branch, &base)
    }

    pub fn abort_merge_task(&self, id: &str) -> anyhow::Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        agency_core::merge::abort_merge(&repo)
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

    pub fn set_ui_state(&self, focused: bool, active_run: Option<String>) {
        let mut ui = self.ui.lock().unwrap();
        ui.focused = focused;
        ui.active_run = active_run;
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
        let message = compose_feedback(&unsent);
        self.term.send_text(&session_name(run_id), &message)?;
        self.registry.lock().unwrap().mark_review_comments_sent(run_id)?;
        Ok(())
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
        let projects = self.registry.lock().unwrap().list_projects()?;
        let mut out = Vec::new();
        for proj in projects {
            let runs = self.registry.lock().unwrap().list_runs(&proj.id)?;
            for run in runs {
                let agent = self.term.status(&session_name(&run.id)).unwrap_or(SessionStatus::Gone);
                let run_script = self.term.status(&run_session_name(&run.id)).unwrap_or(SessionStatus::Gone);
                let pane = self.term.capture(&session_name(&run.id), 50).unwrap_or_default();
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&pane, &mut hasher);
                let pane_hash = std::hash::Hasher::finish(&hasher);
                let label = format!(
                    "{}: {}",
                    run.agent,
                    if run.prompt.is_empty() { run.branch.clone() } else { run.prompt.clone() }
                );
                let user_input_pending = self.input_seen.lock().unwrap().contains(&run.id);
                out.push(notifier::RunSnapshot { id: run.id, label, agent, run_script, pane_hash, user_input_pending });
            }
        }
        Ok(out)
    }
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
    use super::{new_task_id, slugify, pick_port};
    use std::collections::HashSet;

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
        // Shape check independent of tmux: a terminal Run carries kind="terminal",
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
        };
        assert_eq!(run.kind, "terminal");
        assert!(run.branch.is_empty());
        assert!(run.port_base.is_none());
        assert!(run.id.starts_with("terminal-"));
    }
}
