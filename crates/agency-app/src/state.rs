use agency_core::profile::AgentProfile;
use agency_core::registry::{Project, Registry};
use agency_core::supervisor::AgentHandle;
use agency_core::worktree::Worktree;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

#[allow(dead_code)]
pub struct Session {
    pub handle: AgentHandle,
    pub worktree: Worktree,
    pub project_id: String,
}

pub struct AppState {
    registry: Mutex<Registry>,
    #[allow(dead_code)]
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
}
