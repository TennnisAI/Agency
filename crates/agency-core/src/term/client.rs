//! App-side client: one connection, demuxed output callbacks + request/reply.
use crate::term::protocol::*;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type OutputCb = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

struct Shared {
    write: Mutex<UnixStream>,
    callbacks: Mutex<HashMap<String, OutputCb>>,
    reply_rx: Mutex<Receiver<ServerMsg>>,
    req: Mutex<()>, // serializes request/reply pairs
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
        let stream = match connect(&socket_path) {
            Some(s) => s,
            None => {
                // Spawn the daemon (it self-daemonizes) then retry with backoff.
                std::process::Command::new(&daemon_bin)
                    .arg(&socket_path)
                    .spawn()
                    .map_err(|e| anyhow!("spawn {daemon_bin:?}: {e}"))?;
                let mut found = None;
                for _ in 0..100 {
                    if let Some(s) = connect(&socket_path) {
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
        });

        spawn_reader(read_half, shared.clone(), reply_tx);
        let client = TermClient { shared };
        let version = client.daemon_version()?;
        if version != PROTOCOL_VERSION {
            return Err(anyhow!(
                "termd protocol mismatch: app speaks {PROTOCOL_VERSION}, daemon speaks {version}. \
                 Quit running agents and restart, or relaunch the previous app version."
            ));
        }
        Ok(client)
    }

    pub fn daemon_version(&self) -> Result<u32> {
        match self.request(ClientMsg::Hello { version: PROTOCOL_VERSION })? {
            ServerMsg::Hello { version } => Ok(version),
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
        self.request(ClientMsg::StartSession {
            id: id.into(),
            cwd: cwd.to_string_lossy().into(),
            command: command.into(),
            args: args.to_vec(),
            env: env.to_vec(),
            cols,
            rows,
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
        match self.request(ClientMsg::Capture { id: id.into(), lines })? {
            ServerMsg::Captured { text, .. } => Ok(text),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn status(&self, id: &str) -> Result<SessionStatus> {
        match self.request(ClientMsg::Status { id: id.into() })? {
            ServerMsg::Status { status, .. } => Ok(status),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn kill(&self, id: &str) -> Result<()> {
        send_msg(&self.shared, &ClientMsg::Kill { id: id.into() })
    }

    pub fn list(&self) -> Result<Vec<(String, SessionStatus)>> {
        match self.request(ClientMsg::List)? {
            ServerMsg::List { sessions } => Ok(sessions),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn shutdown(&self) -> Result<()> {
        send_msg(&self.shared, &ClientMsg::Shutdown)
    }

    /// Send a request and block for the next reply. `req` serializes callers so
    /// the single reply channel never mixes responses.
    fn request(&self, msg: ClientMsg) -> Result<ServerMsg> {
        let _guard = self.shared.req.lock().unwrap();
        send_msg(&self.shared, &msg)?;
        let rx = self.shared.reply_rx.lock().unwrap();
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(ServerMsg::Error { message, .. }) => Err(anyhow!(message)),
            Ok(m) => Ok(m),
            Err(_) => Err(anyhow!("daemon reply timeout")),
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
            Err(_) => break, // daemon gone; higher layer surfaces via failed requests
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
            Err(_) => break,
        }
    });
}

fn fire(shared: &Arc<Shared>, id: &str, bytes: Vec<u8>) {
    let cb = shared.callbacks.lock().unwrap().get(id).cloned();
    if let Some(cb) = cb {
        cb(bytes);
    }
}
