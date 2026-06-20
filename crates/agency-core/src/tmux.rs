use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SessionStatus {
    Running,
    Exited { code: i32 },
    Gone,
}

pub struct Tmux {
    bin: PathBuf,
}

impl Tmux {
    pub fn new(bin: PathBuf) -> Tmux {
        Tmux { bin }
    }

    /// Resolve the tmux binary. For now this is system `tmux`; a packaging task
    /// will point this at a binary bundled inside the app.
    pub fn resolved() -> Tmux {
        Tmux::new(PathBuf::from("tmux"))
    }

    fn cmd(&self, args: &[&str]) -> Result<std::process::Output> {
        Ok(Command::new(&self.bin).args(args).output()?)
    }

    fn ok(&self, args: &[&str]) -> Result<String> {
        let out = self.cmd(args)?;
        if !out.status.success() {
            bail!(
                "tmux {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub fn start_session(
        &self,
        name: &str,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<()> {
        let cwd_s = cwd.to_string_lossy().to_string();
        let mut a: Vec<String> = vec![
            "new-session".into(),
            "-d".into(),
            "-s".into(),
            name.into(),
            "-c".into(),
            cwd_s,
            "-x".into(),
            "220".into(),
            "-y".into(),
            "50".into(),
        ];
        for (k, v) in env {
            a.push("-e".into());
            a.push(format!("{k}={v}"));
        }
        // Terminate options; the rest is the command to run.
        a.push(command.to_string());
        a.extend(args.iter().cloned());
        // Chain set-option in the same tmux invocation so remain-on-exit is set
        // before the process can exit and tear down the server.
        a.push(";".into());
        a.push("set-option".into());
        a.push("-t".into());
        a.push(name.into());
        a.push("remain-on-exit".into());
        a.push("on".into());
        let aref: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
        self.ok(&aref)?;
        Ok(())
    }

    pub fn session_exists(&self, name: &str) -> Result<bool> {
        Ok(self.cmd(&["has-session", "-t", name])?.status.success())
    }

    pub fn session_status(&self, name: &str) -> Result<SessionStatus> {
        if !self.session_exists(name)? {
            return Ok(SessionStatus::Gone);
        }
        let out = self.ok(&[
            "list-panes",
            "-t",
            name,
            "-F",
            "#{pane_dead} #{pane_dead_status}",
        ])?;
        let first = out.lines().next().unwrap_or("");
        let mut parts = first.split_whitespace();
        let dead = parts.next().unwrap_or("0");
        if dead == "1" {
            let code = parts.next().unwrap_or("0").parse::<i32>().unwrap_or(0);
            Ok(SessionStatus::Exited { code })
        } else {
            Ok(SessionStatus::Running)
        }
    }

    pub fn capture(&self, name: &str, lines: usize) -> Result<String> {
        if !self.session_exists(name)? {
            return Ok(String::new());
        }
        let start = format!("-{lines}");
        self.ok(&["capture-pane", "-p", "-t", name, "-S", &start])
    }

    pub fn kill_session(&self, name: &str) -> Result<()> {
        if self.session_exists(name)? {
            self.ok(&["kill-session", "-t", name])?;
        }
        Ok(())
    }
}
