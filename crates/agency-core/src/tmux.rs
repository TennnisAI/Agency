use crate::profile::AgentProfile;
use crate::supervisor::{spawn_agent, AgentHandle};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Name of the private tmux server socket used for all agency sessions.
/// Using a dedicated socket isolates agency from the user's own tmux server.
const SOCKET: &str = "agency";

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

    /// Run a tmux command against the private `agency` server socket.
    fn cmd(&self, args: &[&str]) -> Result<std::process::Output> {
        let full: Vec<&str> = ["-L", SOCKET].iter().copied().chain(args.iter().copied()).collect();
        Ok(Command::new(&self.bin).args(&full).output()?)
    }

    /// Like `cmd` but fails if tmux exits non-zero, returning stdout.
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
        // `--` terminates tmux option parsing; everything after is the command.
        a.push("--".into());
        a.push(command.to_string());
        a.extend(args.iter().cloned());
        // Set remain-on-exit globally in the same tmux invocation as new-session.
        // The `;` commands run in tmux's event loop before the child's exit event
        // can tear down the pane, so the option is guaranteed to be in effect —
        // no race.  Using `-g` means subsequent sessions on the private server
        // also inherit the setting automatically.
        a.extend([";", "set-option", "-g", "remain-on-exit", "on"].map(String::from));
        // Make tmux size each window to the most recently attached client, so the
        // session reflows when our attach PTY is resized to match the UI terminal.
        a.extend([";", "set-option", "-g", "window-size", "latest"].map(String::from));
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

    /// Gracefully detach every client attached to `name`. Call this before
    /// dropping our [`AgentHandle`], whose `Drop` SIGKILLs the `tmux attach`
    /// client process: abruptly killing the client can take the session's pane
    /// process down with it (the shell exits 0 on a stray EOF), whereas a clean
    /// `detach-client` lets the client leave without disturbing the pane. The
    /// client then exits on its own, so the handle's kill becomes a harmless
    /// fallback. Best-effort — a missing session or no clients is not an error.
    pub fn detach_clients(&self, name: &str) {
        let _ = self.cmd(&["detach-client", "-s", name]);
    }

    /// Type literal `text` into the session followed by Enter. Errors if the
    /// session does not exist.
    pub fn send_text(&self, name: &str, text: &str) -> Result<()> {
        if !self.session_exists(name)? {
            bail!("session {name} is not running");
        }
        self.ok(&["send-keys", "-t", name, "-l", text])?;
        self.ok(&["send-keys", "-t", name, "Enter"])?;
        Ok(())
    }

    /// Attach to an existing tmux session via a PTY, streaming its output through
    /// `on_output`. Returns an [`AgentHandle`] whose `write_input` sends keystrokes
    /// into the session. Dropping the handle detaches; the session keeps running.
    ///
    /// The attach targets the private `agency` socket (`-L agency`) so it reaches
    /// the same server that [`start_session`] uses.
    pub fn attach<F>(&self, name: &str, on_output: F) -> Result<AgentHandle>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let profile = AgentProfile {
            name: "tmux-attach".to_string(),
            command: self.bin.to_string_lossy().to_string(),
            args: vec![
                "-L".to_string(),
                SOCKET.to_string(),
                "attach-session".to_string(),
                "-t".to_string(),
                name.to_string(),
            ],
            env: vec![],
        };
        // cwd is irrelevant for an attach; use the temp dir which always exists.
        spawn_agent(&profile, &std::env::temp_dir(), "", on_output)
    }
}
