use agency_core::term::client::TermClient;
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
