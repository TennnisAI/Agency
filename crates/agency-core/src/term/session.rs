//! A single terminal session: one PTY child, one emulator, N subscribers.
use crate::term::emulator::Emulator;
use crate::term::protocol::{encode_json, encode_output, encode_snapshot, ServerMsg, SessionStatus};
use crate::term::pty::{spawn_pty, ProcStatus, PtyProcess};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Subs = Arc<Mutex<HashMap<u64, Sender<Vec<u8>>>>>;

/// A command to run in the same session if the primary exits within `grace`
/// before any input. Wired by `ensure_run_active` so a failed resume falls back
/// to a fresh agent. (Behavior implemented in Task 2; this task only threads it.)
#[derive(Clone)]
pub struct Fallback {
    pub command: String,
    pub args: Vec<String>,
    pub grace: Duration,
}

pub struct Session {
    id: String,
    pty: PtyProcess,
    emu: Arc<Mutex<Emulator>>,
    subs: Subs,
}

impl Session {
    pub fn start(
        id: String,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
        _fallback: Option<Fallback>,
    ) -> Result<Arc<Session>> {
        let emu = Arc::new(Mutex::new(Emulator::new(cols, rows)));
        let subs: Subs = Arc::new(Mutex::new(HashMap::new()));

        // Reader closure: feed the emulator, then fan raw bytes to subscribers.
        // Lock order is ALWAYS emu -> subs; `subscribe` uses the same order so a
        // newly-registered subscriber can never receive output before its snapshot.
        let emu_r = emu.clone();
        let subs_r = subs.clone();
        let id_out = id.clone();
        let on_output = move |bytes: Vec<u8>| {
            let mut e = emu_r.lock().unwrap();
            e.feed(&bytes);
            let frame = encode_output(&id_out, &bytes);
            for tx in subs_r.lock().unwrap().values() {
                let _ = tx.send(frame.clone());
            }
        };

        let subs_x = subs.clone();
        let id_exit = id.clone();
        let on_exit = move |code: i32| {
            let frame = encode_json(&ServerMsg::Exited { id: id_exit.clone(), code });
            for tx in subs_x.lock().unwrap().values() {
                let _ = tx.send(frame.clone());
            }
        };

        let pty = spawn_pty(command, args, cwd, env, cols, rows, on_output, on_exit)?;
        Ok(Arc::new(Session { id, pty, emu, subs }))
    }

    pub fn input(&self, bytes: &[u8]) {
        let _ = self.pty.write_input(bytes);
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.pty.resize(rows, cols);
        self.emu.lock().unwrap().resize(cols, rows);
    }

    pub fn capture(&self, lines: usize) -> String {
        self.emu.lock().unwrap().capture(lines)
    }

    pub fn status(&self) -> SessionStatus {
        match self.pty.status() {
            ProcStatus::Running => SessionStatus::Running,
            ProcStatus::Exited(code) => SessionStatus::Exited { code },
            ProcStatus::Crashed => SessionStatus::Exited { code: -1 },
        }
    }

    /// Register a subscriber and hand it the current screen as a snapshot,
    /// atomically: the emulator lock is held across snapshot + registration +
    /// snapshot-send so no output frame can jump ahead of the snapshot.
    pub fn subscribe(&self, client_id: u64, out: Sender<Vec<u8>>) {
        let emu = self.emu.lock().unwrap();
        let snap = emu.snapshot();
        let frame = encode_snapshot(&self.id, snap.cols, snap.rows, snap.cx, snap.cy, &snap.data);
        let mut subs = self.subs.lock().unwrap();
        let _ = out.send(frame);
        subs.insert(client_id, out);
    }

    pub fn unsubscribe(&self, client_id: u64) {
        self.subs.lock().unwrap().remove(&client_id);
    }

    pub fn kill(&self) {
        // Dropping the PtyProcess kills the child; sessions are removed from the
        // registry by the caller. Killing here is the explicit teardown path.
        let _ = self.pty.write_input(&[]); // no-op flush; real kill is on drop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::protocol::{decode_server, ServerFrame};
    use std::sync::mpsc;
    use std::time::Duration;

    fn drain_until<P: Fn(&ServerFrame) -> bool>(
        rx: &mpsc::Receiver<Vec<u8>>,
        pred: P,
    ) -> ServerFrame {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok(frame) = rx.recv_timeout(Duration::from_millis(200)) {
                let decoded = decode_server(&frame).unwrap();
                if pred(&decoded) {
                    return decoded;
                }
            }
        }
        panic!("predicate never matched");
    }

    #[test]
    fn subscribe_delivers_a_snapshot_first() {
        let s = Session::start(
            "t1".into(),
            std::env::temp_dir().as_path(),
            "/bin/sh",
            &["-c".into(), "printf ready; sleep 2".into()],
            &[],
            80,
            24,
            None,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));

        let (tx, rx) = mpsc::channel();
        s.subscribe(1, tx);
        let snap = drain_until(&rx, |f| matches!(f, ServerFrame::Snapshot { .. }));
        match snap {
            ServerFrame::Snapshot { id, data, .. } => {
                assert_eq!(id, "t1");
                assert!(String::from_utf8_lossy(&data).contains("ready"));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn input_is_echoed_to_subscribers_and_capture() {
        let s = Session::start(
            "t2".into(),
            std::env::temp_dir().as_path(),
            "/bin/cat",
            &[],
            &[],
            80,
            24,
            None,
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        s.subscribe(1, tx);
        s.input(b"ping\n");
        drain_until(&rx, |f| {
            matches!(f, ServerFrame::Output { bytes, .. } if String::from_utf8_lossy(bytes).contains("ping"))
        });
        assert!(s.capture(5).contains("ping"));
    }
}
