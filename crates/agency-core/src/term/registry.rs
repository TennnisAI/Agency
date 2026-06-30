//! Owns all live sessions and the idle-exit accounting.
use crate::term::protocol::SessionStatus;
use crate::term::session::{Fallback, Session};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct Registry {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl Registry {
    pub fn new() -> Arc<Registry> {
        Arc::new(Registry { sessions: Mutex::new(HashMap::new()) })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        id: String,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
        fallback: Option<Fallback>,
    ) -> Result<()> {
        let session = Session::start(id.clone(), cwd, command, args, env, cols, rows, fallback)?;
        self.sessions.lock().unwrap().insert(id, session);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<(String, SessionStatus)> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, s)| (id.clone(), s.status()))
            .collect()
    }

    pub fn kill(&self, id: &str) {
        // Removing the Arc drops the Session (and its PtyProcess), killing the child.
        self.sessions.lock().unwrap().remove(id);
    }

    pub fn kill_all(&self) {
        self.sessions.lock().unwrap().clear();
    }

    pub fn running_count(&self) -> usize {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| matches!(s.status(), SessionStatus::Running))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleeper(reg: &Registry, id: &str) {
        reg.start(
            id.to_string(),
            std::env::temp_dir().as_path(),
            "/bin/sh",
            &["-c".to_string(), "sleep 5".to_string()],
            &[],
            80,
            24,
            None,
        )
        .unwrap();
    }

    #[test]
    fn start_list_kill() {
        let reg = Registry::new();
        sleeper(&reg, "a");
        sleeper(&reg, "b");
        assert_eq!(reg.list().len(), 2);
        assert_eq!(reg.running_count(), 2);
        reg.kill("a");
        assert_eq!(reg.list().len(), 1);
        assert!(reg.get("a").is_none());
        assert!(reg.get("b").is_some());
        reg.kill_all();
        assert_eq!(reg.list().len(), 0);
    }
}
