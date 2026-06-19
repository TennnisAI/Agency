use agency_app_lib::AppState;
use std::path::Path;

#[test]
fn new_seeds_default_shell_profile_and_version_holds() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    assert_eq!(AppState::version(), "0.1.0");
    assert!(state.profile_names().contains(&"shell".to_string()));
}

#[test]
fn project_crud_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();

    let p = state.add_project("demo", Path::new("/tmp/demo-repo")).unwrap();
    assert_eq!(p.name, "demo");

    let all = state.list_projects().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, p.id);

    state.remove_project(&p.id).unwrap();
    assert_eq!(state.list_projects().unwrap().len(), 0);
}

use agency_app_lib::TaskInfo;
use agency_core::profile::AgentProfile;
use agency_core::supervisor::AgentStatus;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn init_repo(dir: &Path) {
    let run = |args: &[&str]| {
        assert!(
            Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
            "git {:?}",
            args
        );
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "t@e.com"]);
    run(&["config", "user.name", "T"]);
    std::fs::write(dir.join("README.md"), "hi").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
}

fn fake_agent_command() -> String {
    // Reuse the Phase 1 fixture from agency-core.
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("agency-core")
        .join("tests")
        .join("fixtures")
        .join("fake_agent.sh");
    p.to_string_lossy().to_string()
}

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

#[test]
fn start_task_spawns_in_worktree_streams_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    state.register_profile(AgentProfile {
        name: "fake".into(),
        command: fake_agent_command(),
        args: vec!["{{prompt}}".into()],
        env: vec![],
    });

    let project = state.add_project("demo", &repo).unwrap();

    let buf = Arc::new(Mutex::new(String::new()));
    let buf_cb = buf.clone();
    let info: TaskInfo = state
        .start_task(&project.id, "do-the-thing", "fake", "HEAD", move |bytes| {
            buf_cb.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
        })
        .unwrap();

    assert_eq!(info.branch, format!("agent/{}", info.task_id));

    // Worktree exists on disk.
    let wt_path = repo.join(".agency").join("worktrees").join(&info.task_id);
    assert!(wt_path.exists());

    // Streaming + the rendered prompt arg.
    assert!(wait_for(&buf, "AGENT_READY", Duration::from_secs(5)));
    assert!(wait_for(&buf, "PROMPT:do-the-thing", Duration::from_secs(5)));

    // Input injection.
    state.send_input(&info.task_id, b"ping\n").unwrap();
    assert!(wait_for(&buf, "GOT:ping", Duration::from_secs(5)));

    // Status reaches Exited(0).
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if matches!(state.task_status(&info.task_id).unwrap(), AgentStatus::Exited(_)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(state.task_status(&info.task_id).unwrap(), AgentStatus::Exited(0));

    // Stop removes the worktree.
    state.stop_task(&info.task_id).unwrap();
    assert!(!wt_path.exists());
}

#[test]
fn worktree_path_resolves_for_active_task() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo); // helper already defined earlier in this test file

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    state.register_profile(AgentProfile {
        name: "fake".into(),
        command: fake_agent_command(),
        args: vec!["{{prompt}}".into()],
        env: vec![],
    });
    let project = state.add_project("demo", &repo).unwrap();
    let info = state
        .start_task(&project.id, "p", "fake", "HEAD", |_| {})
        .unwrap();

    let wt = state.worktree_path(&info.task_id).unwrap();
    assert!(wt.ends_with(format!(".agency/worktrees/{}", info.task_id)));
    assert!(wt.exists());

    assert!(state.worktree_path("does-not-exist").is_err());
}
