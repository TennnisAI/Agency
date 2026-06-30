use agency_core::profile::AgentProfile;
use agency_core::supervisor::{spawn_agent, AgentStatus};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn fixture_path() -> String {
    // tests/fixtures/fake_agent.sh relative to the crate manifest.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake_agent.sh");
    p.to_string_lossy().to_string()
}

/// Poll `buf` until it contains `needle` or the timeout elapses.
fn wait_for(buf: &Arc<Mutex<String>>, needle: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if buf.lock().unwrap().contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// A PTY child must see TERM=xterm-256color (what the frontend xterm.js speaks),
/// even when the parent process has a different/absent TERM. Otherwise a Finder-
/// launched bundle (no TERM from launchd) makes `tmux attach` fail with
/// "open terminal failed: terminal does not support clear".
#[test]
fn pty_child_gets_xterm_term() {
    // Force a wrong parent TERM so the assertion proves spawn_agent overrode it,
    // not that we merely inherited a good value from the test environment.
    std::env::set_var("TERM", "dumb-sentinel");
    let profile = AgentProfile {
        name: "term-probe".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "echo TERM=$TERM; sleep 0.1".into()],
        env: vec![],
        resume_args: None,
    };

    let buf = Arc::new(Mutex::new(String::new()));
    let buf_cb = buf.clone();
    let handle = spawn_agent(&profile, &std::env::temp_dir(), "", move |bytes| {
        buf_cb.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
    })
    .unwrap();

    let ok = wait_for(&buf, "TERM=xterm-256color", Duration::from_secs(5));
    drop(handle);
    assert!(ok, "PTY child TERM was: {:?}", buf.lock().unwrap());
}

#[test]
fn spawns_streams_input_and_exits() {
    let profile = AgentProfile {
        name: "fake".into(),
        command: fixture_path(),
        args: vec!["{{prompt}}".into()],
        env: vec![],
        resume_args: None,
    };

    let buf = Arc::new(Mutex::new(String::new()));
    let buf_cb = buf.clone();
    let cwd = std::env::temp_dir();

    let handle = spawn_agent(&profile, &cwd, "do-the-thing", move |bytes| {
        buf_cb.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
    })
    .unwrap();

    // Banner + rendered prompt arg appear.
    assert!(wait_for(&buf, "AGENT_READY", Duration::from_secs(5)));
    assert!(wait_for(&buf, "PROMPT:do-the-thing", Duration::from_secs(5)));

    // Inject input; the agent echoes it back.
    handle.write_input(b"ping\n").unwrap();
    assert!(wait_for(&buf, "GOT:ping", Duration::from_secs(5)));
    assert!(wait_for(&buf, "AGENT_DONE", Duration::from_secs(5)));

    // Wait for clean exit and assert status.
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if matches!(handle.status(), AgentStatus::Exited(_)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(handle.status(), AgentStatus::Exited(0));
}
