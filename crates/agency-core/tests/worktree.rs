use agency_core::worktree::WorktreeManager;
use std::path::Path;
use std::process::Command;

/// Create a real git repo with one commit; return its path (kept alive by `dir`).
fn init_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {:?} failed", args);
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(dir.join("README.md"), "hi").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
}

#[test]
fn create_makes_worktree_and_branch() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let mgr = WorktreeManager::new(dir.path().to_path_buf());

    let wt = mgr.create("task-1", "HEAD").unwrap();

    assert_eq!(wt.task_id, "task-1");
    assert_eq!(wt.branch, "agent/task-1");
    assert!(wt.path.ends_with(".agency/worktrees/task-1"));
    assert!(wt.path.join("README.md").exists());

    // The exclude file should keep .agency/ untracked-invisible.
    let exclude = std::fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
    assert!(exclude.contains(".agency/"));
}

#[test]
fn list_returns_created_worktrees() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let mgr = WorktreeManager::new(dir.path().to_path_buf());

    mgr.create("task-1", "HEAD").unwrap();
    mgr.create("task-2", "HEAD").unwrap();

    let mut ids: Vec<String> = mgr.list().unwrap().into_iter().map(|w| w.task_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["task-1".to_string(), "task-2".to_string()]);
}

#[test]
fn remove_deletes_worktree() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let mgr = WorktreeManager::new(dir.path().to_path_buf());

    let wt = mgr.create("task-1", "HEAD").unwrap();
    mgr.remove("task-1").unwrap();

    assert!(!wt.path.exists());
    assert!(mgr.list().unwrap().is_empty());
}
