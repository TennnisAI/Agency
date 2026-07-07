use agency_core::term::client::TermClient;
use agency_core::term::protocol::{
    decode_client, encode_json, read_frame, write_frame, ClientFrame, ClientMsg, ServerMsg,
    SessionStatus, PROTOCOL_VERSION,
};
use agency_core::term::registry::Registry;
use agency_core::term::server;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// Spin the server in-process (no daemonize) on a temp socket and return a client.
fn server_and_client() -> (tempfile::TempDir, TermClient) {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("termd.sock");
    let registry = Registry::new();
    let clients = Arc::new(AtomicUsize::new(0));
    {
        let sock = sock.clone();
        std::thread::spawn(move || {
            let _ = server::run_with_registry(&sock, registry, clients);
        });
    }
    // Wait for the socket to appear.
    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let client = TermClient::connect_or_spawn(sock, std::path::PathBuf::from("/nonexistent")).unwrap();
    (dir, client)
}

#[test]
fn handshake_reports_protocol_version() {
    let (_dir, client) = server_and_client();
    assert_eq!(client.daemon_version().unwrap(), agency_core::term::protocol::PROTOCOL_VERSION);
}

/// A reply that arrives with the wrong seq (i.e. the answer to an earlier,
/// timed-out request) must be discarded, not returned for the current request.
#[test]
fn stale_reply_with_old_seq_is_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("fake.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut read = stream.try_clone().unwrap();
        let mut write = stream;
        loop {
            let payload = match read_frame(&mut read) {
                Ok(p) => p,
                Err(_) => break,
            };
            match decode_client(&payload) {
                Ok(ClientFrame::Msg(ClientMsg::Hello { seq, .. })) => {
                    let m = ServerMsg::Hello { version: PROTOCOL_VERSION, seq };
                    write_frame(&mut write, &encode_json(&m)).unwrap();
                }
                Ok(ClientFrame::Msg(ClientMsg::Status { id, seq })) => {
                    // First, a stale leftover reply (previous seq, wrong data)…
                    let stale = ServerMsg::Status {
                        id: id.clone(),
                        status: SessionStatus::Gone,
                        seq: seq.wrapping_sub(1),
                    };
                    write_frame(&mut write, &encode_json(&stale)).unwrap();
                    // …then the real reply for this request.
                    let real = ServerMsg::Status { id, status: SessionStatus::Running, seq };
                    write_frame(&mut write, &encode_json(&real)).unwrap();
                }
                _ => {}
            }
        }
    });

    let client = TermClient::connect_or_spawn(sock, std::path::PathBuf::from("/nonexistent")).unwrap();
    assert_eq!(client.status("x").unwrap(), SessionStatus::Running);
    // And the channel is not left shifted: the next request still matches.
    assert_eq!(client.status("y").unwrap(), SessionStatus::Running);
}

/// A surviving daemon that speaks an older protocol is shut down and replaced
/// by a freshly spawned one instead of failing the connect (which used to
/// crash-loop the app after an upgrade).
#[test]
fn protocol_mismatch_replaces_old_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("termd.sock");

    // Fake "old" daemon: v1 handshake (no seq on the wire), exits on Shutdown.
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let sock_srv = sock.clone();
    let old = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut read = stream.try_clone().unwrap();
        let mut write = stream;
        loop {
            let payload = match read_frame(&mut read) {
                Ok(p) => p,
                Err(_) => break,
            };
            match decode_client(&payload) {
                Ok(ClientFrame::Msg(ClientMsg::Hello { .. })) => {
                    let mut frame = vec![0u8]; // T_JSON, v1 payload without seq
                    frame.extend_from_slice(br#"{"Hello":{"version":1}}"#);
                    write_frame(&mut write, &frame).unwrap();
                }
                Ok(ClientFrame::Msg(ClientMsg::Shutdown)) => {
                    drop(listener);
                    let _ = std::fs::remove_file(&sock_srv);
                    return;
                }
                _ => {}
            }
        }
    });

    let termd = std::path::PathBuf::from(env!("CARGO_BIN_EXE_agency-termd"));
    let client = TermClient::connect_or_spawn(sock, termd).expect("recovered from mismatch");
    assert_eq!(client.daemon_version().unwrap(), PROTOCOL_VERSION);
    old.join().unwrap();
    // Don't leak the real daemon this test spawned.
    client.shutdown().unwrap();
}

#[test]
fn start_subscribe_input_capture_kill() {
    let (_dir, client) = server_and_client();
    client
        .start_session("r1", std::env::temp_dir().as_path(), "/bin/cat", &[], &[], 80, 24)
        .unwrap();

    let (tx, rx) = mpsc::channel();
    let sub = client
        .subscribe("r1", 80, 24, move |bytes| {
            let _ = tx.send(bytes);
        })
        .unwrap();

    // First frame delivered to the callback is the snapshot.
    let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(first.windows(2).any(|w| w == b"\x1b["), "snapshot should contain escapes");

    client.input("r1", b"echo-me\n").unwrap();
    let mut seen = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if let Ok(b) = rx.recv_timeout(Duration::from_millis(200)) {
            seen.push_str(&String::from_utf8_lossy(&b));
            if seen.contains("echo-me") {
                break;
            }
        }
    }
    assert!(seen.contains("echo-me"), "got: {seen:?}");
    assert!(client.capture("r1", 5).unwrap().contains("echo-me"));

    assert_eq!(client.list().unwrap().len(), 1);
    drop(sub);
    client.kill("r1").unwrap();
    assert_eq!(client.list().unwrap().len(), 0);
}

#[test]
fn start_session_with_fallback_runs_fresh_on_fast_primary_exit() {
    let (_dir, client) = server_and_client();
    client.start_session_with_fallback(
        "fb",
        std::env::temp_dir().as_path(),
        "/bin/sh",
        &["-c".into(), "printf NOPE; exit 1".into()],
        &[],
        80, 24,
        Some(agency_core::term::protocol::FallbackSpec {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "printf FRESH; sleep 3".into()],
            grace_ms: 2000,
        }),
    ).unwrap();

    let mut cap = String::new();
    for _ in 0..60 {
        cap = client.capture("fb", 10).unwrap();
        if cap.contains("FRESH") { break; }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(cap.contains("FRESH"), "fallback did not run through the daemon: {cap:?}");
    assert!(matches!(
        client.status("fb").unwrap(),
        agency_core::term::SessionStatus::Running
    ));
}
