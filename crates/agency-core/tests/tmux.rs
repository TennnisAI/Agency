use agency_core::tmux::{SessionStatus, Tmux};
use std::sync::{Arc, Mutex};

fn unique(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{prefix}-{n}")
}

#[test]
fn start_capture_status_kill() {
    let t = Tmux::resolved();
    let name = unique("agencytest");
    let cwd = std::env::temp_dir();

    // A command that prints then exits 0 quickly.
    t.start_session(&name, &cwd, "sh", &["-c".into(), "echo HELLO; exit 0".into()], &[])
        .unwrap();
    assert!(t.session_exists(&name).unwrap());

    // Wait for output to appear in the pane.
    let mut captured = String::new();
    for _ in 0..50 {
        captured = t.capture(&name, 50).unwrap();
        if captured.contains("HELLO") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(captured.contains("HELLO"), "capture was: {captured:?}");

    // remain-on-exit keeps the session; status becomes Exited(0).
    let mut status = SessionStatus::Running;
    for _ in 0..50 {
        status = t.session_status(&name).unwrap();
        if matches!(status, SessionStatus::Exited { .. }) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert_eq!(status, SessionStatus::Exited { code: 0 });

    t.kill_session(&name).unwrap();
    assert!(!t.session_exists(&name).unwrap());
    assert_eq!(t.session_status(&name).unwrap(), SessionStatus::Gone);
}

#[test]
fn start_session_runs_in_cwd_with_env() {
    let t = Tmux::resolved();
    let name = unique("agencyenv");
    let dir = tempfile::tempdir().unwrap();

    t.start_session(
        &name,
        dir.path(),
        "sh",
        &["-c".into(), "echo CWD=$(pwd); echo VAR=$MYVAR; sleep 2".into()],
        &[("MYVAR".into(), "xyz".into())],
    )
    .unwrap();

    let mut cap = String::new();
    for _ in 0..50 {
        cap = t.capture(&name, 50).unwrap();
        if cap.contains("VAR=xyz") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(cap.contains("VAR=xyz"), "cap: {cap:?}");
    // cwd path may be /private-symlinked on macOS; just assert the temp dir's final component.
    let leaf = dir.path().file_name().unwrap().to_string_lossy().to_string();
    assert!(cap.contains(&leaf), "cap: {cap:?}");

    t.kill_session(&name).unwrap();
}

#[test]
fn attach_streams_output_and_input() {
    let t = Tmux::resolved();
    let name = unique("agencyattach");
    let cwd = std::env::temp_dir();
    // A shell that prints READY, reads a line, echoes it.
    t.start_session(&name, &cwd, "sh", &["-c".into(), "echo READY; read x; echo GOT:$x; sleep 2".into()], &[])
        .unwrap();

    let buf = Arc::new(Mutex::new(String::new()));
    let b = buf.clone();
    let handle = t.attach(&name, move |bytes| {
        b.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
    }).unwrap();

    let wait = |needle: &str| {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if buf.lock().unwrap().contains(needle) { return true; }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        false
    };

    assert!(wait("READY"), "buf: {:?}", buf.lock().unwrap());
    handle.write_input(b"ping\n").unwrap();
    assert!(wait("GOT:ping"), "buf: {:?}", buf.lock().unwrap());

    drop(handle); // detach
    // session still alive right after detach
    assert!(t.session_exists(&name).unwrap());
    t.kill_session(&name).unwrap();
}
