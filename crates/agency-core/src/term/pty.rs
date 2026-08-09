//! One PTY-backed child process with reader + wait threads.
use anyhow::Result;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum ProcStatus {
    Running,
    Exited(i32),
    Crashed,
}

pub struct PtyProcess {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    status: Arc<Mutex<ProcStatus>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    // Wrapped in Mutex so PtyProcess is Sync (needed for Arc<Session>: Send).
    master: Mutex<Box<dyn MasterPty + Send>>,
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

impl PtyProcess {
    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master.lock().unwrap().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn status(&self) -> ProcStatus {
        self.status.lock().unwrap().clone()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_pty<F, X>(
    command: &str,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    cols: u16,
    rows: u16,
    on_output: F,
    on_exit: X,
) -> Result<PtyProcess>
where
    F: Fn(Vec<u8>) + Send + 'static,
    X: Fn(i32) + Send + 'static,
{
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;

    let mut cmd = CommandBuilder::new(command);
    cmd.args(args);
    cmd.cwd(cwd);
    // Finder-launched bundles inherit no TERM/COLORTERM; default them unless the
    // caller overrides.
    cmd.env("TERM", "xterm-256color");
    if !env.iter().any(|(k, _)| k == "COLORTERM") {
        cmd.env("COLORTERM", "truecolor");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }

    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let killer = child.clone_killer();
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let status = Arc::new(Mutex::new(ProcStatus::Running));

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => on_output(buf[..n].to_vec()),
            }
        }
    });

    let status_for_wait = status.clone();
    std::thread::spawn(move || {
        let code = match child.wait() {
            Ok(es) => es.exit_code() as i32,
            Err(_) => {
                *status_for_wait.lock().unwrap() = ProcStatus::Crashed;
                on_exit(-1);
                return;
            }
        };
        *status_for_wait.lock().unwrap() = ProcStatus::Exited(code);
        on_exit(code);
    });

    Ok(PtyProcess {
        writer: Arc::new(Mutex::new(writer)),
        status,
        killer,
        master: Mutex::new(pair.master),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn spawns_echo_and_reports_exit() {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let p = spawn_pty(
            "/bin/echo",
            &["hi-there".to_string()],
            std::env::temp_dir().as_path(),
            &[],
            80,
            24,
            move |bytes| {
                let _ = tx.send(bytes);
            },
            move |code| {
                let _ = etx.send(code);
            },
        )
        .unwrap();

        let mut seen = String::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(2)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("hi-there") {
                break;
            }
        }
        assert!(seen.contains("hi-there"), "got: {seen:?}");
        assert_eq!(erx.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
        let _ = p;
    }
}
