use agency_core::profile::AgentProfile;
use agency_core::registry::{Project, Registry};
use agency_core::supervisor::AgentHandle;
use agency_core::tmux::{SessionStatus, Tmux};
use agency_core::worktree::WorktreeManager;
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

const SETTING_ANTHROPIC_KEY: &str = "anthropic_api_key";
const SETTING_LM_STUDIO_URL: &str = "lm_studio_base_url";
const DEFAULT_LM_STUDIO_URL: &str = "http://localhost:1234/v1";

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
    pub branch: String,
    pub status: SessionStatus,
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
}

fn session_name(id: &str) -> String {
    format!("agency-{id}")
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

pub fn new_task_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:032x}", nanos ^ ((n as u128) << 96))
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub struct AppState {
    registry: Mutex<Registry>,
    pub attaches: Mutex<HashMap<String, AgentHandle>>,
    pub tmux: Tmux,
    resolvers: Mutex<HashMap<String, AgentHandle>>,
}

impl AppState {
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn new(db_path: &Path) -> Result<AppState> {
        let registry = Registry::open(db_path)?;
        if registry.list_profiles()?.is_empty() {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            registry.upsert_profile(&AgentProfile {
                name: "shell".to_string(),
                command: shell,
                args: vec!["-l".to_string()],
                env: vec![],
            })?;
            registry.upsert_profile(&AgentProfile {
                name: "claude".to_string(),
                command: "claude".to_string(),
                args: vec!["{{prompt}}".to_string()],
                env: vec![],
            })?;
        }
        Ok(AppState {
            registry: Mutex::new(registry),
            attaches: Mutex::new(HashMap::new()),
            tmux: Tmux::resolved(),
            resolvers: Mutex::new(HashMap::new()),
        })
    }

    pub fn provider_env(&self) -> Result<Vec<(String, String)>> {
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

    pub fn add_project(&self, name: &str, repo_path: &Path) -> Result<Project> {
        self.registry.lock().unwrap().add_project(name, repo_path)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.registry.lock().unwrap().list_projects()
    }

    pub fn remove_project(&self, id: &str) -> Result<()> {
        self.registry.lock().unwrap().remove_project(id)
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
        let status = self.tmux.session_status(&name).unwrap_or(SessionStatus::Gone);
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
            branch: run.branch.clone(),
            status,
            added: stat.added,
            deleted: stat.deleted,
            files: stat.files,
        }
    }

    // ── run lifecycle ──────────────────────────────────────────────────────────

    pub fn create_run(&self, project_id: &str, prompt: &str, agent: &str, base: &str) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {agent}"))?
        };
        let id = new_task_id();
        let worktree = WorktreeManager::new(repo).create(&id, base)?;

        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        let args = profile.render_args(prompt);

        self.tmux
            .start_session(&session_name(&id), &worktree.path, &profile.command, &args, &env)?;

        let run = agency_core::registry::Run {
            id: id.clone(),
            project_id: project_id.to_string(),
            agent: agent.to_string(),
            prompt: prompt.to_string(),
            base: base.to_string(),
            branch: worktree.branch.clone(),
            created_at: now_secs(),
        };
        self.registry.lock().unwrap().insert_run(&run)?;
        Ok(self.run_info(&run))
    }

    pub fn list_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_runs(project_id)?;
        Ok(runs.iter().map(|r| self.run_info(r)).collect())
    }

    pub fn run_status(&self, id: &str) -> Result<SessionStatus> {
        Ok(self.tmux.session_status(&session_name(id)).unwrap_or(SessionStatus::Gone))
    }

    pub fn discard_run(&self, id: &str) -> Result<()> {
        self.attaches.lock().unwrap().remove(id);
        let run = self.run_record(id)?;
        self.tmux.kill_session(&session_name(id)).ok();
        if let Ok(repo) = self.project_repo(&run.project_id) {
            let _ = WorktreeManager::new(repo).remove(id);
        }
        self.registry.lock().unwrap().delete_run(id)?;
        Ok(())
    }

    pub fn rerun(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?
        };
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        let args = profile.render_args(&run.prompt);
        self.tmux.kill_session(&session_name(id)).ok();
        self.tmux.start_session(&session_name(id), &worktree, &profile.command, &args, &env)?;
        Ok(self.run_info(&run))
    }

    // ── worktree path ──────────────────────────────────────────────────────────

    pub fn worktree_path(&self, id: &str) -> Result<std::path::PathBuf> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        Ok(repo.join(".agency").join("worktrees").join(id))
    }

    // ── merge operations ───────────────────────────────────────────────────────

    pub fn merge_task(&self, id: &str) -> anyhow::Result<agency_core::merge::MergeOutcome> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let base = agency_core::merge::detect_base(&repo)?;
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
        let base = agency_core::merge::detect_base(&repo).unwrap_or_else(|_| "main".to_string());
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
}
