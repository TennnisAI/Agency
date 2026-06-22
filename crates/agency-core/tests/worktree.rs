use agency_core::worktree::WorktreeManager;
use std::process::Command;
use tempfile::tempdir;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q"]);
    git(p, &["config", "user.email", "t@t.t"]);
    git(p, &["config", "user.name", "t"]);
    std::fs::write(p.join("README.md"), "hi").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "init"]);
    dir
}

fn branch_exists(dir: &std::path::Path, branch: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", branch])
        .current_dir(dir)
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn remove_keep_branch_keeps_the_branch_then_restore_recreates_worktree() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-1", "HEAD").unwrap();
    assert!(wt.path.exists());
    assert!(branch_exists(repo.path(), "agent/task-1"));

    // Archive: worktree gone, branch kept.
    mgr.remove_keep_branch("task-1").unwrap();
    assert!(!wt.path.exists(), "worktree dir removed");
    assert!(branch_exists(repo.path(), "agent/task-1"), "branch kept");

    // Restore: worktree recreated on the same branch.
    let restored = mgr.restore("task-1").unwrap();
    assert!(restored.path.exists(), "worktree recreated");
    assert_eq!(restored.branch, "agent/task-1");
}

#[test]
fn remove_after_keep_branch_deletes_the_branch_without_error() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    mgr.create("task-2", "HEAD").unwrap();
    mgr.remove_keep_branch("task-2").unwrap();
    // Discarding an archived run: worktree already gone, but the branch must go.
    mgr.remove("task-2").unwrap();
    assert!(!branch_exists(repo.path(), "agent/task-2"), "branch deleted on discard");
}

#[test]
fn create_makes_worktree_and_branch() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-1", "HEAD").unwrap();
    assert_eq!(wt.task_id, "task-1");
    assert_eq!(wt.branch, "agent/task-1");
    assert!(wt.path.ends_with(".agency/worktrees/task-1"));
    assert!(wt.path.join("README.md").exists());
    // The exclude file keeps agency artifacts untracked-invisible.
    let exclude = std::fs::read_to_string(repo.path().join(".git/info/exclude")).unwrap();
    assert!(exclude.contains(".agency/"));
}

#[test]
fn list_returns_created_worktrees() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    mgr.create("task-1", "HEAD").unwrap();
    mgr.create("task-2", "HEAD").unwrap();
    let mut ids: Vec<String> = mgr.list().unwrap().into_iter().map(|w| w.task_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["task-1".to_string(), "task-2".to_string()]);
}

#[test]
fn remove_deletes_worktree() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-1", "HEAD").unwrap();
    mgr.remove("task-1").unwrap();
    assert!(!wt.path.exists());
    assert!(mgr.list().unwrap().is_empty());
}
