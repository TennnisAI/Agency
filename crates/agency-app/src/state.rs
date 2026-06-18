use agency_core::profile::AgentProfile;
use agency_core::registry::{Project, Registry};
use agency_core::supervisor::{spawn_agent, AgentHandle, AgentStatus};
use agency_core::worktree::WorktreeManager;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub struct Session {
    pub handle: AgentHandle,
    pub repo_path: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskInfo {
    pub task_id: String,
    pub branch: String,
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
    profiles: Mutex<Vec<AgentProfile>>,
}

impl AppState {
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn new(db_path: &Path) -> Result<AppState> {
        let registry = Registry::open(db_path)?;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let default_profile = AgentProfile {
            name: "shell".to_string(),
            command: shell,
            args: vec!["-l".to_string()],
            env: vec![],
        };
        Ok(AppState {
            registry: Mutex::new(registry),
            sessions: Mutex::new(HashMap::new()),
            profiles: Mutex::new(vec![default_profile]),
        })
    }

    pub fn register_profile(&self, profile: AgentProfile) {
        let mut profiles = self.profiles.lock().unwrap();
        if let Some(slot) = profiles.iter_mut().find(|p| p.name == profile.name) {
            *slot = profile;
        } else {
            profiles.push(profile);
        }
    }

    pub fn profile_names(&self) -> Vec<String> {
        self.profiles
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.name.clone())
            .collect()
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
        // Resolve project repo path (lock released before spawning).
        let repo_path = {
            let reg = self.registry.lock().unwrap();
            reg.get_project(project_id)?
                .ok_or_else(|| anyhow!("unknown project: {project_id}"))?
                .repo_path
        };

        // Resolve the profile (clone so we don't hold the lock during spawn).
        let profile = {
            let profiles = self.profiles.lock().unwrap();
            profiles
                .iter()
                .find(|p| p.name == profile_name)
                .cloned()
                .ok_or_else(|| anyhow!("unknown profile: {profile_name}"))?
        };

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
}
