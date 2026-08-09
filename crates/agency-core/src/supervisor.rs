use crate::profile::AgentProfile;
use anyhow::Result;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Idle,
    Exited(i32),
    Crashed,
}

pub struct AgentHandle {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    status: Arc<Mutex<AgentStatus>>,
    // Kills the spawned child when the handle is dropped. The reader thread holds
    // a cloned PTY reader, so simply dropping `master` does NOT close the PTY or
    // unblock that read — the child would
    // linger forever, leaking a process on every attach/detach cycle. Killing it
    // closes the slave, the reader hits EOF, and the thread exits.
    killer: Box<dyn ChildKiller + Send + Sync>,
    // Keep the master alive so the PTY stays open for the lifetime of the handle.
    master: Box<dyn MasterPty + Send>,
}

impl Drop for AgentHandle {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

impl AgentHandle {
    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    /// Resize the PTY. This delivers SIGWINCH to the child so it reflows to
    /// the new size.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    pub fn status(&self) -> AgentStatus {
        self.status.lock().unwrap().clone()
    }
}

/// Spawn `profile` in a PTY with working directory `cwd`, injecting `prompt`
/// into the rendered args. `on_output` is called with raw PTY bytes as they arrive.
pub fn spawn_agent<F>(
    profile: &AgentProfile,
    cwd: &Path,
    prompt: &str,
    on_output: F,
) -> Result<AgentHandle>
where
    F: Fn(Vec<u8>) + Send + 'static,
{
    let pty_system = native_pty_system();
    let pair =
        pty_system.openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })?;

    let mut cmd = CommandBuilder::new(&profile.command);
    cmd.args(profile.render_args(prompt));
    cmd.cwd(cwd);
    // Every PTY we open is rendered by the frontend xterm.js terminal, which speaks
    // xterm-256color. A Finder-launched .app inherits no TERM from launchd, so a
    // child would have no terminal type and fail with "open terminal
    // failed: terminal does not support clear". Default TERM here; a profile may
    // still override it via its own env below.
    cmd.env("TERM", "xterm-256color");
    for (k, v) in &profile.env {
        cmd.env(k, v);
    }

    let mut child = pair.slave.spawn_command(cmd)?;
    // The slave handle is no longer needed once the child holds it.
    drop(pair.slave);

    let killer = child.clone_killer();
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;

    let status = Arc::new(Mutex::new(AgentStatus::Running));

    // Reader thread: pump PTY output to the callback until EOF.
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => on_output(buf[..n].to_vec()),
            }
        }
    });

    // Wait thread: record the exit code when the child finishes.
    let status_for_wait = status.clone();
    std::thread::spawn(move || {
        let code = match child.wait() {
            Ok(es) => es.exit_code() as i32,
            Err(_) => {
                *status_for_wait.lock().unwrap() = AgentStatus::Crashed;
                return;
            }
        };
        *status_for_wait.lock().unwrap() = AgentStatus::Exited(code);
    });

    Ok(AgentHandle { writer: Arc::new(Mutex::new(writer)), status, killer, master: pair.master })
}
