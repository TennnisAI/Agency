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

#[test]
fn commit_all_if_dirty_preserves_work_on_the_branch() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-wip", "HEAD").unwrap();

    // Clean worktree: no commit made.
    assert!(!mgr.commit_all_if_dirty("task-wip", "WIP").unwrap());

    // Dirty worktree (tracked edit + untracked file): committed.
    std::fs::write(wt.path.join("README.md"), "edited").unwrap();
    std::fs::write(wt.path.join("untracked.txt"), "new").unwrap();
    assert!(mgr.commit_all_if_dirty("task-wip", "WIP: archive").unwrap());

    // Worktree is clean afterwards and the commit is on the agent branch.
    assert!(!mgr.commit_all_if_dirty("task-wip", "WIP").unwrap());
    let out = Command::new("git")
        .args(["log", "-1", "--format=%s", "agent/task-wip"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "WIP: archive");

    // Archive+restore round-trip brings the work back.
    mgr.remove_keep_branch("task-wip").unwrap();
    let restored = mgr.restore("task-wip").unwrap();
    assert_eq!(std::fs::read_to_string(restored.path.join("untracked.txt")).unwrap(), "new");
}

#[test]
fn commit_all_if_dirty_is_noop_for_missing_worktree() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    assert!(!mgr.commit_all_if_dirty("no-such-task", "WIP").unwrap());
}

#[test]
fn copy_into_copies_untracked_files_and_dirs_skipping_bad_paths() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-copy", "HEAD").unwrap();

    std::fs::write(repo.path().join(".env"), "SECRET=1").unwrap();
    std::fs::create_dir_all(repo.path().join("config/certs")).unwrap();
    std::fs::write(repo.path().join("config/certs/dev.pem"), "pem").unwrap();

    let copied = mgr
        .copy_into(
            "task-copy",
            &[
                ".env".to_string(),
                "config".to_string(),
                "missing.txt".to_string(),
                "../escape".to_string(),
                "/abs/path".to_string(),
            ],
        )
        .unwrap();
    assert_eq!(copied, vec![".env".to_string(), "config".to_string()]);
    assert_eq!(std::fs::read_to_string(wt.path.join(".env")).unwrap(), "SECRET=1");
    assert_eq!(
        std::fs::read_to_string(wt.path.join("config/certs/dev.pem")).unwrap(),
        "pem"
    );
    assert!(!wt.path.join("missing.txt").exists());
}

#[test]
fn copy_essentials_auto_copies_untracked_root_env_files() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-env", "HEAD").unwrap();

    // Untracked local secrets — the frustrating case: not committed, so a
    // worktree wouldn't otherwise get them.
    std::fs::write(repo.path().join(".env"), "SECRET=1").unwrap();
    std::fs::write(repo.path().join(".env.local"), "LOCAL=2").unwrap();
    // A tracked env file must NOT be re-copied (it already materializes).
    std::fs::write(repo.path().join(".env.example"), "EXAMPLE=tracked").unwrap();
    git(repo.path(), &["add", ".env.example"]);
    git(repo.path(), &["commit", "-q", "-m", "add example"]);

    let detected = mgr.default_env_files();
    assert_eq!(detected, vec![".env".to_string(), ".env.local".to_string()]);

    // With no configured copy list, the env defaults still land in the worktree.
    let copied = mgr.copy_essentials("task-env", &[]).unwrap();
    assert_eq!(copied, vec![".env".to_string(), ".env.local".to_string()]);
    assert_eq!(std::fs::read_to_string(wt.path.join(".env")).unwrap(), "SECRET=1");
    assert_eq!(std::fs::read_to_string(wt.path.join(".env.local")).unwrap(), "LOCAL=2");
}

#[test]
fn copy_essentials_unions_configured_list_with_env_defaults_without_dupes() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    mgr.create("task-union", "HEAD").unwrap();

    std::fs::write(repo.path().join(".env"), "SECRET=1").unwrap();
    std::fs::create_dir_all(repo.path().join("config")).unwrap();
    std::fs::write(repo.path().join("config/dev.pem"), "pem").unwrap();

    // `.env` is listed explicitly AND auto-detected — it must appear once.
    let copied = mgr
        .copy_essentials("task-union", &[".env".to_string(), "config".to_string()])
        .unwrap();
    assert_eq!(copied, vec![".env".to_string(), "config".to_string()]);
}

#[test]
fn create_on_branch_checks_out_existing_branch() {
    let repo = init_repo();
    git(repo.path(), &["branch", "feat/existing"]);
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create_on_branch("task-pr", "feat/existing").unwrap();
    assert!(wt.path.exists());
    assert_eq!(wt.branch, "feat/existing");
    // No agent/ branch was created for it.
    assert!(!branch_exists(repo.path(), "agent/task-pr"));
}
