use agency_core::profile::AgentProfile;
use agency_core::registry::{Project, Registry};
use agency_core::supervisor::{spawn_agent, AgentHandle, AgentStatus};
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

pub struct Session {
    pub handle: AgentHandle,
    pub repo_path: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    pub task_id: String,
    pub branch: String,
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

fn new_task_id() -> String {
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

pub struct AppState {
    registry: Mutex<Registry>,
    sessions: Mutex<HashMap<String, Session>>,
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
            sessions: Mutex::new(HashMap::new()),
            resolvers: Mutex::new(HashMap::new()),
        })
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

    pub fn add_project(&self, name: &str, repo_path: &Path) -> Result<Project> {
        self.registry.lock().unwrap().add_project(name, repo_path)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.registry.lock().unwrap().list_projects()
    }

    pub fn remove_project(&self, id: &str) -> Result<()> {
        self.registry.lock().unwrap().remove_project(id)
    }

    pub fn start_task<F>(
        &self,
        project_id: &str,
        prompt: &str,
        profile_name: &str,
        base: &str,
        on_output: F,
    ) -> Result<TaskInfo>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        // Resolve project repo path.
        let repo_path = {
            let reg = self.registry.lock().unwrap();
            reg.get_project(project_id)?
                .ok_or_else(|| anyhow!("unknown project: {project_id}"))?
                .repo_path
        };

        // Resolve the profile and provider settings; build the effective env.
        let mut profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(profile_name)?
                .ok_or_else(|| anyhow!("unknown profile: {profile_name}"))?
        };

        // Provider env first, then the profile's own env on top.
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        profile.env = env;

        let task_id = new_task_id();
        let manager = WorktreeManager::new(repo_path.clone());
        let worktree = manager.create(&task_id, base)?;

        let handle = spawn_agent(&profile, &worktree.path, prompt, on_output)?;
        let branch = worktree.branch.clone();

        self.sessions.lock().unwrap().insert(
            task_id.clone(),
            Session { handle, repo_path },
        );

        Ok(TaskInfo { task_id, branch })
    }

    pub fn send_input(&self, task_id: &str, data: &[u8]) -> Result<()> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(task_id)
            .ok_or_else(|| anyhow!("unknown task: {task_id}"))?;
        session.handle.write_input(data)
    }

    pub fn task_status(&self, task_id: &str) -> Result<AgentStatus> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(task_id)
            .ok_or_else(|| anyhow!("unknown task: {task_id}"))?;
        Ok(session.handle.status())
    }

    pub fn worktree_path(&self, task_id: &str) -> anyhow::Result<std::path::PathBuf> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("unknown task: {task_id}"))?;
        Ok(session
            .repo_path
            .join(".agency")
            .join("worktrees")
            .join(task_id))
    }

    pub fn stop_task(&self, task_id: &str) -> Result<()> {
        // Remove (and drop) the session first so the PTY/handle is released.
        let session = self
            .sessions
            .lock()
            .unwrap()
            .remove(task_id)
            .ok_or_else(|| anyhow!("unknown task: {task_id}"))?;
        let repo_path = session.repo_path.clone();
        drop(session); // close PTY before removing the worktree
        WorktreeManager::new(repo_path).remove(task_id)?;
        Ok(())
    }

    fn repo_path_for(&self, task_id: &str) -> anyhow::Result<std::path::PathBuf> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("unknown task: {task_id}"))?;
        Ok(session.repo_path.clone())
    }

    pub fn merge_task(&self, task_id: &str) -> anyhow::Result<agency_core::merge::MergeOutcome> {
        let repo = self.repo_path_for(task_id)?;
        let base = agency_core::merge::detect_base(&repo)?;
        let branch = format!("agent/{task_id}");
        agency_core::merge::merge(&repo, &branch, &base)
    }

    pub fn abort_merge_task(&self, task_id: &str) -> anyhow::Result<()> {
        let repo = self.repo_path_for(task_id)?;
        agency_core::merge::abort_merge(&repo)
    }

    pub fn resolve_merge<F>(
        &self,
        task_id: &str,
        resolver_profile: &str,
        on_output: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let repo = self.repo_path_for(task_id)?;
        let base = agency_core::merge::detect_base(&repo).unwrap_or_else(|_| "main".to_string());
        let branch = format!("agent/{task_id}");
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

        let handle = spawn_agent(&profile, &repo, &prompt, on_output)?;
        self.resolvers.lock().unwrap().insert(task_id.to_string(), handle);
        Ok(())
    }

    pub fn resolver_input(&self, task_id: &str, data: &[u8]) -> anyhow::Result<()> {
        let resolvers = self.resolvers.lock().unwrap();
        let handle = resolvers
            .get(task_id)
            .ok_or_else(|| anyhow!("no resolver for task: {task_id}"))?;
        handle.write_input(data)
    }

    pub fn resolver_status(&self, task_id: &str) -> anyhow::Result<agency_core::supervisor::AgentStatus> {
        let resolvers = self.resolvers.lock().unwrap();
        let handle = resolvers
            .get(task_id)
            .ok_or_else(|| anyhow!("no resolver for task: {task_id}"))?;
        Ok(handle.status())
    }
}
