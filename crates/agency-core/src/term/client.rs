//! App-side client: one connection, demuxed output callbacks + request/reply.
use crate::term::protocol::*;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type OutputCb = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

struct Shared {
    write: Mutex<UnixStream>,
    callbacks: Mutex<HashMap<String, OutputCb>>,
    reply_rx: Mutex<Receiver<ServerMsg>>,
    req: Mutex<()>, // serializes request/reply pairs
    next_seq: AtomicU64,
    alive: Arc<AtomicBool>,
}

pub struct TermClient {
    shared: Arc<Shared>,
}

pub struct Subscription {
    id: String,
    shared: Arc<Shared>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.shared.callbacks.lock().unwrap().remove(&self.id);
        let _ = send_msg(&self.shared, &ClientMsg::Unsubscribe { id: self.id.clone() });
    }
}

fn send_msg(shared: &Shared, msg: &ClientMsg) -> Result<()> {
    let payload = encode_json(msg);
    let mut w = shared.write.lock().unwrap();
    write_frame(&mut *w, &payload).map_err(Into::into)
}

impl TermClient {
    pub fn connect_or_spawn(socket_path: PathBuf, daemon_bin: PathBuf) -> Result<TermClient> {
        let client = Self::connect_once(&socket_path, &daemon_bin)?;
        // A daemon from a previous app version survived an upgrade: it either
        // reports a different version or fails the handshake outright. Its
        // sessions are incompatible anyway, so replace it: ask it to shut down
        // (Shutdown exists in every protocol version), wait for the socket to
        // die, spawn a fresh daemon. One recovery attempt, then give up.
        match client.daemon_version() {
            Ok(version) if version == PROTOCOL_VERSION => return Ok(client),
            Ok(version) => log::warn!(
                "termd protocol mismatch: app speaks {PROTOCOL_VERSION}, daemon speaks {version}; \
                 shutting the old daemon down (its sessions are lost) and spawning a fresh one"
            ),
            Err(e) => log::warn!(
                "termd handshake failed ({e}); shutting the old daemon down \
                 (its sessions are lost) and spawning a fresh one"
            ),
        }
        let _ = client.shutdown();
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(5);
        while connect(&socket_path).is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }

        let client = Self::connect_once(&socket_path, &daemon_bin)?;
        let version = client.daemon_version()?;
        if version != PROTOCOL_VERSION {
            return Err(anyhow!(
                "termd protocol mismatch persists after replacing the daemon: \
                 app speaks {PROTOCOL_VERSION}, daemon speaks {version}"
            ));
        }
        Ok(client)
    }

    fn connect_once(socket_path: &Path, daemon_bin: &Path) -> Result<TermClient> {
        let stream = match connect(socket_path) {
            Some(s) => s,
            None => {
                // Spawn the daemon (it self-daemonizes) then retry with backoff.
                std::process::Command::new(daemon_bin)
                    .arg(socket_path)
                    .spawn()
                    .map_err(|e| anyhow!("spawn {daemon_bin:?}: {e}"))?;
                let mut found = None;
                for _ in 0..100 {
                    if let Some(s) = connect(socket_path) {
                        found = Some(s);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                found.ok_or_else(|| anyhow!("daemon did not come up at {socket_path:?}"))?
            }
        };

        let read_half = stream.try_clone()?;
        let (reply_tx, reply_rx) = channel::<ServerMsg>();
        let shared = Arc::new(Shared {
            write: Mutex::new(stream),
            callbacks: Mutex::new(HashMap::new()),
            reply_rx: Mutex::new(reply_rx),
            req: Mutex::new(()),
            next_seq: AtomicU64::new(1),
            alive: Arc::new(AtomicBool::new(true)),
        });

        spawn_reader(read_half, shared.clone(), reply_tx);
        Ok(TermClient { shared })
    }

    pub fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    pub fn daemon_version(&self) -> Result<u32> {
        match self.request(|seq| ClientMsg::Hello { version: PROTOCOL_VERSION, seq })? {
            ServerMsg::Hello { version, .. } => Ok(version),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn start_session(
        &self,
        id: &str,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
    ) -> Result<()> {
        self.start_session_with_fallback(id, cwd, command, args, env, cols, rows, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_session_with_fallback(
        &self,
        id: &str,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
        fallback: Option<FallbackSpec>,
    ) -> Result<()> {
        self.request(|seq| ClientMsg::StartSession {
            id: id.into(),
            cwd: cwd.to_string_lossy().into(),
            command: command.into(),
            args: args.to_vec(),
            env: env.to_vec(),
            cols,
            rows,
            fallback,
            seq,
        })
        .map(|_| ())
    }

    pub fn subscribe<F>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<Subscription>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        self.shared
            .callbacks
            .lock()
            .unwrap()
            .insert(id.to_string(), Arc::new(on_output));
        send_msg(&self.shared, &ClientMsg::Resize { id: id.into(), cols, rows })?;
        send_msg(&self.shared, &ClientMsg::Subscribe { id: id.into() })?;
        Ok(Subscription { id: id.into(), shared: self.shared.clone() })
    }

    pub fn input(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let payload = encode_input(id, bytes);
        let mut w = self.shared.write.lock().unwrap();
        write_frame(&mut *w, &payload).map_err(Into::into)
    }

    pub fn send_text(&self, id: &str, text: &str) -> Result<()> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(b'\r');
        self.input(id, &bytes)
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        send_msg(&self.shared, &ClientMsg::Resize { id: id.into(), cols, rows })
    }

    pub fn capture(&self, id: &str, lines: usize) -> Result<String> {
        match self.request(|seq| ClientMsg::Capture { id: id.into(), lines, seq })? {
            ServerMsg::Captured { text, .. } => Ok(text),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn status(&self, id: &str) -> Result<SessionStatus> {
        match self.request(|seq| ClientMsg::Status { id: id.into(), seq })? {
            ServerMsg::Status { status, .. } => Ok(status),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn kill(&self, id: &str) -> Result<()> {
        send_msg(&self.shared, &ClientMsg::Kill { id: id.into() })
    }

    pub fn list(&self) -> Result<Vec<(String, SessionStatus)>> {
        match self.request(|seq| ClientMsg::List { seq })? {
            ServerMsg::List { sessions, .. } => Ok(sessions),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn shutdown(&self) -> Result<()> {
        send_msg(&self.shared, &ClientMsg::Shutdown)
    }

    /// Send a request tagged with a fresh seq and block for the reply that
    /// echoes it. `req` serializes callers so the single reply channel never
    /// interleaves responses; the seq match discards a reply that arrives
    /// after its request already timed out — without it, one timeout would
    /// permanently shift every later request onto the previous reply.
    fn request(&self, make: impl FnOnce(u64) -> ClientMsg) -> Result<ServerMsg> {
        let _guard = self.shared.req.lock().unwrap();
        let seq = self.shared.next_seq.fetch_add(1, Ordering::SeqCst);
        let msg = make(seq);
        let is_hello = matches!(msg, ClientMsg::Hello { .. });
        send_msg(&self.shared, &msg)?;
        let rx = self.shared.reply_rx.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let reply = match rx.recv_timeout(remaining) {
                Ok(m) => m,
                Err(_) => return Err(anyhow!("daemon reply timeout")),
            };
            // A legacy daemon doesn't echo seq (reads as 0). Accept its
            // handshake reply — Hello, or an Error for our Hello (a daemon too
            // old to even parse it) — so version mismatch surfaces immediately
            // and connect_or_spawn can replace the daemon instead of timing out.
            let legacy_handshake = reply.seq() == 0
                && (matches!(reply, ServerMsg::Hello { .. })
                    || (is_hello && matches!(reply, ServerMsg::Error { .. })));
            if reply.seq() != seq && !legacy_handshake {
                log::warn!("termd: dropping stale reply (seq {}, expected {seq})", reply.seq());
                continue;
            }
            return match reply {
                ServerMsg::Error { message, .. } => Err(anyhow!(message)),
                m => Ok(m),
            };
        }
    }
}

fn connect(path: &Path) -> Option<UnixStream> {
    UnixStream::connect(path).ok()
}

fn spawn_reader(mut read_half: UnixStream, shared: Arc<Shared>, reply_tx: Sender<ServerMsg>) {
    std::thread::spawn(move || loop {
        let payload = match read_frame(&mut read_half) {
            Ok(p) => p,
            Err(_) => {
                shared.alive.store(false, Ordering::SeqCst);
                break; // daemon gone; higher layer surfaces via failed requests
            }
        };
        match decode_server(&payload) {
            Ok(ServerFrame::Output { id, bytes }) => fire(&shared, &id, bytes),
            Ok(ServerFrame::Snapshot { id, data, .. }) => fire(&shared, &id, data),
            Ok(ServerFrame::Msg(ServerMsg::Exited { id, code })) => {
                // Surface exit as an empty-output tick; status() reports the code.
                let _ = code;
                let _ = id;
            }
            Ok(ServerFrame::Msg(m)) => {
                let _ = reply_tx.send(m);
            }
            Err(_) => {
                shared.alive.store(false, Ordering::SeqCst);
                break;
            }
        }
    });
}

fn fire(shared: &Arc<Shared>, id: &str, bytes: Vec<u8>) {
    let cb = shared.callbacks.lock().unwrap().get(id).cloned();
    if let Some(cb) = cb {
        cb(bytes);
    }
}
