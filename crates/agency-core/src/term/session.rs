//! A single terminal session: a PTY child (with an optional early-exit
//! fallback), an emulator, and N subscribers.
use crate::term::emulator::Emulator;
use crate::term::protocol::{encode_json, encode_output, encode_snapshot, ServerMsg, SessionStatus};
use crate::term::pty::{spawn_pty, ProcStatus, PtyProcess};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

type Subs = Arc<Mutex<HashMap<u64, Sender<Vec<u8>>>>>;

/// A command to run in the same session if the primary exits within `grace`
/// before any input — so a failed resume falls back to a fresh agent.
#[derive(Clone, Debug)]
pub struct Fallback {
    pub command: String,
    pub args: Vec<String>,
    pub grace: Duration,
}

/// Respawn-able shared state. Held strongly by `Session`; the pty `on_exit`
/// closure holds only a `Weak` of this (see `spawn_into`) to avoid a cycle.
struct SpawnCtx {
    id: String,
    emu: Arc<Mutex<Emulator>>,
    subs: Subs,
    pty: Mutex<Option<PtyProcess>>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    cols: u16,
    rows: u16,
    input_seen: AtomicBool,
}

pub struct Session {
    ctx: Arc<SpawnCtx>,
}

/// Spawn `command` into the session. If `fallback` is set and the process exits
/// within `fallback.grace` with no input seen, the fallback command is spawned
/// into the SAME session (same emulator + subscribers), once.
fn spawn_into(
    ctx: &Arc<SpawnCtx>,
    command: String,
    args: Vec<String>,
    fallback: Option<Fallback>,
) -> Result<()> {
    let spawn_at = Instant::now();

    // on_output holds emu + subs STRONGLY (as before); neither references the
    // pty, so there is no cycle. Lock order is emu -> subs.
    let emu_o = ctx.emu.clone();
    let subs_o = ctx.subs.clone();
    let id_o = ctx.id.clone();
    let on_output = move |bytes: Vec<u8>| {
        let mut e = emu_o.lock().unwrap();
        e.feed(&bytes);
        let frame = encode_output(&id_o, &bytes);
        for tx in subs_o.lock().unwrap().values() {
            let _ = tx.send(frame.clone());
        }
    };

    // on_exit holds a WEAK ctx: a strong ref would form
    // ctx -> pty slot -> wait-thread -> on_exit -> ctx and defeat kill-on-drop.
    let weak: Weak<SpawnCtx> = Arc::downgrade(ctx);
    let on_exit = move |code: i32| {
        let Some(ctx) = weak.upgrade() else { return };
        if let Some(fb) = &fallback {
            if spawn_at.elapsed() < fb.grace && !ctx.input_seen.load(Ordering::SeqCst) {
                // Primary died fast with no input -> run the fallback once.
                let _ = spawn_into(&ctx, fb.command.clone(), fb.args.clone(), None);
                return;
            }
        }
        let frame = encode_json(&ServerMsg::Exited { id: ctx.id.clone(), code });
        for tx in ctx.subs.lock().unwrap().values() {
            let _ = tx.send(frame.clone());
        }
    };

    let pty = spawn_pty(&command, &args, &ctx.cwd, &ctx.env, ctx.cols, ctx.rows, on_output, on_exit)?;
    *ctx.pty.lock().unwrap() = Some(pty);
    Ok(())
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        id: String,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
        fallback: Option<Fallback>,
    ) -> Result<Arc<Session>> {
        let ctx = Arc::new(SpawnCtx {
            id: id.clone(),
            emu: Arc::new(Mutex::new(Emulator::new(cols, rows))),
            subs: Arc::new(Mutex::new(HashMap::new())),
            pty: Mutex::new(None),
            cwd: cwd.to_path_buf(),
            env: env.to_vec(),
            cols,
            rows,
            input_seen: AtomicBool::new(false),
        });
        spawn_into(&ctx, command.to_string(), args.to_vec(), fallback)?;
        Ok(Arc::new(Session { ctx }))
    }

    pub fn input(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.ctx.input_seen.store(true, Ordering::SeqCst);
        }
        if let Some(p) = self.ctx.pty.lock().unwrap().as_ref() {
            let _ = p.write_input(bytes);
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let Some(p) = self.ctx.pty.lock().unwrap().as_ref() {
            let _ = p.resize(rows, cols);
        }
        self.ctx.emu.lock().unwrap().resize(cols, rows);
    }

    pub fn capture(&self, lines: usize) -> String {
        self.ctx.emu.lock().unwrap().capture(lines)
    }

    pub fn status(&self) -> SessionStatus {
        match self.ctx.pty.lock().unwrap().as_ref().map(|p| p.status()) {
            Some(ProcStatus::Running) | None => SessionStatus::Running,
            Some(ProcStatus::Exited(code)) => SessionStatus::Exited { code },
            Some(ProcStatus::Crashed) => SessionStatus::Exited { code: -1 },
        }
    }

    /// Register a subscriber and hand it the current screen as a snapshot,
    /// atomically: the emulator lock is held across snapshot + registration +
    /// snapshot-send so no output frame can jump ahead of the snapshot.
    pub fn subscribe(&self, client_id: u64, out: Sender<Vec<u8>>) {
        let emu = self.ctx.emu.lock().unwrap();
        let snap = emu.snapshot();
        let frame = encode_snapshot(&self.ctx.id, snap.cols, snap.rows, snap.cx, snap.cy, &snap.data);
        let mut subs = self.ctx.subs.lock().unwrap();
        let _ = out.send(frame);
        subs.insert(client_id, out);
    }

    pub fn unsubscribe(&self, client_id: u64) {
        self.ctx.subs.lock().unwrap().remove(&client_id);
    }

    pub fn kill(&self) {
        // Real teardown is dropping the Session (SpawnCtx -> pty slot -> child kill).
        if let Some(p) = self.ctx.pty.lock().unwrap().as_ref() {
            let _ = p.write_input(&[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::protocol::{decode_server, ServerFrame, ServerMsg};
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
    fn snapshot_restores_alt_screen_mode_for_a_reattaching_client() {
        // A full-screen TUI enters the alt screen and hides the cursor. A client
        // that attaches later (navigated away and back) must be put back into that
        // mode by the snapshot, or its cursor-relative redraws desync.
        let s = Session::start(
            "alt1".into(),
            std::env::temp_dir().as_path(),
            "/bin/sh",
            &["-c".into(), "printf '\\033[?1049h\\033[?25lPAINTED'; sleep 2".into()],
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
            ServerFrame::Snapshot { data, .. } => {
                let text = String::from_utf8_lossy(&data);
                assert!(text.contains("\x1b[?1049h"), "alt-screen enter missing: {text:?}");
                assert!(text.contains("\x1b[?25l"), "cursor-hide missing: {text:?}");
                assert!(text.contains("PAINTED"), "content missing: {text:?}");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn snapshot_restores_mouse_reporting_for_a_reattaching_client() {
        // The scroll-wheel bug: an agent TUI turns on mouse tracking once at
        // startup, so the client forwards wheel events to it. A client that
        // attaches later has to be told, or it falls back to sending Up/Down
        // arrows per wheel notch and the agent walks its prompt history — which
        // is why it only ever misfired on the first scroll after switching back.
        let s = Session::start(
            "mouse1".into(),
            std::env::temp_dir().as_path(),
            "/bin/sh",
            &[
                "-c".into(),
                "printf '\\033[?1049h\\033[?1002h\\033[?1006hPAINTED'; sleep 2".into(),
            ],
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
            ServerFrame::Snapshot { data, .. } => {
                let text = String::from_utf8_lossy(&data);
                assert!(text.contains("\x1b[?1002h"), "mouse tracking missing: {text:?}");
                assert!(text.contains("\x1b[?1006h"), "SGR encoding missing: {text:?}");
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

    #[test]
    fn fallback_runs_in_same_session_when_primary_exits_fast() {
        let s = Session::start(
            "fb1".into(),
            std::env::temp_dir().as_path(),
            "/bin/sh",
            &["-c".into(), "printf NOPE; exit 1".into()],
            &[],
            80, 24,
            Some(Fallback {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "printf FRESH; sleep 3".into()],
                grace: Duration::from_secs(2),
            }),
        ).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut cap = String::new();
        while std::time::Instant::now() < deadline {
            cap = s.capture(10);
            if cap.contains("FRESH") { break; }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(cap.contains("NOPE"), "primary output missing: {cap:?}");
        assert!(cap.contains("FRESH"), "fallback output missing: {cap:?}");
        assert!(matches!(s.status(), SessionStatus::Running), "fallback should be running");
    }

    #[test]
    fn no_fallback_when_primary_keeps_running() {
        let s = Session::start(
            "fb2".into(),
            std::env::temp_dir().as_path(),
            "/bin/sh",
            &["-c".into(), "printf ALIVE; sleep 3".into()],
            &[],
            80, 24,
            Some(Fallback {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "printf SHOULD_NOT_RUN; sleep 3".into()],
                grace: Duration::from_secs(1),
            }),
        ).unwrap();
        std::thread::sleep(Duration::from_millis(800));
        let cap = s.capture(10);
        assert!(cap.contains("ALIVE"));
        assert!(!cap.contains("SHOULD_NOT_RUN"), "fallback should not have run: {cap:?}");
        assert!(matches!(s.status(), SessionStatus::Running));
    }

    #[test]
    fn exit_after_grace_emits_exited_and_no_fallback() {
        let (tx, rx) = mpsc::channel();
        let s = Session::start(
            "fb3".into(),
            std::env::temp_dir().as_path(),
            "/bin/sh",
            // stays alive long enough for subscribe to register, then exits past grace
            &["-c".into(), "printf BYE; sleep 1; exit 0".into()],
            &[],
            80, 24,
            Some(Fallback {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "printf SHOULD_NOT_RUN".into()],
                grace: Duration::from_millis(300),
            }),
        ).unwrap();
        s.subscribe(1, tx);
        let got = drain_until(&rx, |f| matches!(f, ServerFrame::Msg(ServerMsg::Exited { .. })));
        assert!(matches!(got, ServerFrame::Msg(ServerMsg::Exited { .. })));
        assert!(!s.capture(10).contains("SHOULD_NOT_RUN"));
    }
}
