//! Unix-socket front end: one accept loop, one reader + one writer thread per
//! client connection, dispatch into the shared Registry, fan output back.
use crate::term::protocol::*;
use crate::term::registry::Registry;
use crate::term::session::Fallback;
use anyhow::Result;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;

const IDLE_GRACE: Duration = Duration::from_secs(45);

pub fn run(socket_path: &Path) -> Result<()> {
    let registry = Registry::new();
    let clients = Arc::new(AtomicUsize::new(0));
    spawn_idle_watch(registry.clone(), clients.clone());
    run_with_registry(socket_path, registry, clients)
}

/// Exits the process once no client has been connected and no session has been
/// running for IDLE_GRACE — bounds the post-crash "ghost" window.
fn spawn_idle_watch(registry: Arc<Registry>, clients: Arc<AtomicUsize>) {
    std::thread::spawn(move || {
        let mut idle_since: Option<std::time::Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let idle = clients.load(Ordering::SeqCst) == 0 && registry.running_count() == 0;
            match (idle, idle_since) {
                (true, None) => idle_since = Some(std::time::Instant::now()),
                (true, Some(t)) if t.elapsed() >= IDLE_GRACE => std::process::exit(0),
                (true, Some(_)) => {}
                (false, _) => idle_since = None,
            }
        }
    });
}

pub fn run_with_registry(
    socket_path: &Path,
    registry: Arc<Registry>,
    clients: Arc<AtomicUsize>,
) -> Result<()> {
    let _ = std::fs::remove_file(socket_path); // clear stale socket
    if let Some(dir) = socket_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(socket_path)?;
    let next_id = Arc::new(AtomicU64::new(1));

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let client_id = next_id.fetch_add(1, Ordering::SeqCst);
        let registry = registry.clone();
        let clients = clients.clone();
        clients.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            handle_client(stream, client_id, registry.clone());
            // On disconnect: drop this client's subscriptions everywhere.
            for (id, _) in registry.list() {
                if let Some(s) = registry.get(&id) {
                    s.unsubscribe(client_id);
                }
            }
            clients.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}

fn handle_client(stream: UnixStream, client_id: u64, registry: Arc<Registry>) {
    let mut read_half = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut write_half = stream;

    // Writer thread: drain this client's outbound channel to the socket.
    let (out_tx, out_rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(frame) = out_rx.recv() {
            if write_frame(&mut write_half, &frame).is_err() {
                break;
            }
        }
    });

    loop {
        let payload = match read_frame(&mut read_half) {
            Ok(p) => p,
            Err(_) => break, // clean disconnect or codec error -> end connection
        };
        match decode_client(&payload) {
            Ok(frame) => dispatch(frame, client_id, &registry, &out_tx),
            Err(e) => {
                let _ = out_tx.send(encode_json(&ServerMsg::Error {
                    id: None,
                    message: format!("bad frame: {e}"),
                    seq: 0,
                }));
            }
        }
    }
}

fn dispatch(frame: ClientFrame, client_id: u64, registry: &Arc<Registry>, out: &Sender<Vec<u8>>) {
    let reply = |m: ServerMsg| {
        let _ = out.send(encode_json(&m));
    };
    match frame {
        ClientFrame::Msg(ClientMsg::Hello { seq, .. }) => {
            reply(ServerMsg::Hello { version: PROTOCOL_VERSION, seq });
        }
        ClientFrame::Msg(ClientMsg::StartSession {
            id,
            cwd,
            command,
            args,
            env,
            cols,
            rows,
            fallback,
            seq,
        }) => {
            let fb = fallback.map(|f| Fallback {
                command: f.command,
                args: f.args,
                grace: Duration::from_millis(f.grace_ms),
            });
            match registry.start(id.clone(), Path::new(&cwd), &command, &args, &env, cols, rows, fb)
            {
                Ok(()) => reply(ServerMsg::Started { id, seq }),
                Err(e) => reply(ServerMsg::Error { id: Some(id), message: e.to_string(), seq }),
            }
        }
        ClientFrame::Msg(ClientMsg::Subscribe { id }) => match registry.get(&id) {
            Some(s) => s.subscribe(client_id, out.clone()),
            // seq 0: Subscribe is fire-and-forget, this error is async.
            None => {
                reply(ServerMsg::Error { id: Some(id), message: "no such session".into(), seq: 0 })
            }
        },
        ClientFrame::Msg(ClientMsg::Unsubscribe { id }) => {
            if let Some(s) = registry.get(&id) {
                s.unsubscribe(client_id);
            }
        }
        ClientFrame::Msg(ClientMsg::Resize { id, cols, rows }) => {
            if let Some(s) = registry.get(&id) {
                s.resize(cols, rows);
            }
        }
        ClientFrame::Msg(ClientMsg::Capture { id, lines, style, seq }) => {
            match (registry.get(&id), style) {
                (Some(s), true) => {
                    let (text, style) = s.capture_styled(lines);
                    reply(ServerMsg::Captured { id: id.clone(), text, style: Some(style), seq })
                }
                (Some(s), false) => {
                    let text = s.capture(lines);
                    reply(ServerMsg::Captured { id: id.clone(), text, style: None, seq })
                }
                (None, _) => {
                    reply(ServerMsg::Captured { id, text: String::new(), style: None, seq })
                }
            }
        }
        ClientFrame::Msg(ClientMsg::Status { id, seq }) => {
            let status = registry.get(&id).map(|s| s.status()).unwrap_or(SessionStatus::Gone);
            reply(ServerMsg::Status { id, status, seq });
        }
        ClientFrame::Msg(ClientMsg::Kill { id }) => registry.kill(&id),
        ClientFrame::Msg(ClientMsg::List { seq }) => {
            reply(ServerMsg::List { sessions: registry.list(), seq })
        }
        ClientFrame::Msg(ClientMsg::Shutdown) => {
            registry.kill_all();
            std::process::exit(0);
        }
        ClientFrame::Input { id, bytes } => {
            if let Some(s) = registry.get(&id) {
                s.input(&bytes);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[test]
    fn connection_accounting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("test.sock");

        let registry = Registry::new();
        let clients = Arc::new(AtomicUsize::new(0));

        let sock_clone = sock.clone();
        let registry_clone = registry.clone();
        let clients_clone = clients.clone();
        std::thread::spawn(move || {
            let _ = run_with_registry(&sock_clone, registry_clone, clients_clone);
        });

        // Wait for a connection to be accepted, not for the socket file: the
        // path exists from the moment bind() creates the inode, and connecting
        // before listen() has set the backlog up gets ECONNREFUSED.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let _stream = loop {
            match UnixStream::connect(&sock) {
                Ok(s) => break s,
                Err(e) => assert!(
                    std::time::Instant::now() < deadline,
                    "server never accepted a connection: {e}"
                ),
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            if clients.load(Ordering::SeqCst) == 1 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "clients never reached 1");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(clients.load(Ordering::SeqCst), 1, "expected 1 live client");

        // Drop the connection — clients counter must return to 0.
        drop(_stream);

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            if clients.load(Ordering::SeqCst) == 0 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "clients never returned to 0");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(clients.load(Ordering::SeqCst), 0, "expected 0 live clients after disconnect");
    }
}
